use super::cpu::{ArmInstructionKind, DecodedArmInstruction};
use super::memory::{
    HEAP_SIZE, HOMEBREW_HEAP_BASE, LEGACY_SYSTEM_MMIO_BASE, LEGACY_SYSTEM_MMIO_SIZE,
};
use cranelift::codegen::ir::{
    condcodes::IntCC, types, AbiParam, Function, InstBuilder, MemFlagsData, Signature, Value,
};
use cranelift::codegen::settings::{self, Configurable};
use cranelift::frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{default_libcall_names, Linkage, Module};
use std::mem::transmute;

const HOT_BLOCK_THRESHOLD: u16 = 128;
const MIN_BLOCK_LEN: usize = 2;
const CACHE_SLOTS: usize = 4_096;
const MAX_COMPILES_PER_SLICE: u8 = 8;
const REGISTER_COUNT: usize = 16;
const N_FLAG: u32 = 1 << 31;
const Z_FLAG: u32 = 1 << 30;
const C_FLAG: u32 = 1 << 29;
const V_FLAG: u32 = 1 << 28;

type JitBlockFn = unsafe extern "C" fn(*mut u32, *mut u32, *mut u8) -> u64;

#[derive(Clone, Copy)]
struct CompiledBlock {
    function: JitBlockFn,
    instruction_count: usize,
}

#[derive(Clone, Copy, Default)]
struct CacheEntry {
    start: u32,
    generation: u64,
    hits: u16,
    failed: bool,
    block: Option<CompiledBlock>,
}

struct Compiler {
    module: JITModule,
    context: cranelift::codegen::Context,
    builder_context: FunctionBuilderContext,
    next_function_id: u64,
}

pub(crate) struct JitEngine {
    compiler: Option<Compiler>,
    entries: Box<[CacheEntry]>,
    compile_budget: u8,
    compiled_blocks: usize,
    enabled: bool,
}

pub(crate) struct JitCpuContext<'a> {
    pub(crate) registers: &'a mut [u32; REGISTER_COUNT],
    pub(crate) cpsr: &'a mut u32,
    pub(crate) heap: *mut u8,
    pub(crate) heap_base: u32,
}

impl JitEngine {
    pub(crate) fn new() -> Self {
        let compiler = match Compiler::new() {
            Ok(compiler) => Some(compiler),
            Err(error) => {
                log::warn!("A330 JIT backend unavailable: {error}");
                None
            }
        };
        Self {
            compiler,
            entries: vec![CacheEntry::default(); CACHE_SLOTS].into_boxed_slice(),
            compile_budget: MAX_COMPILES_PER_SLICE,
            compiled_blocks: 0,
            enabled: true,
        }
    }

    pub(crate) fn begin_slice(&mut self) {
        self.compile_budget = MAX_COMPILES_PER_SLICE;
    }

    pub(crate) fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub(crate) fn execute(
        &mut self,
        start: u32,
        generation: u64,
        instructions: &[DecodedArmInstruction],
        instruction_limit: usize,
        cpu: JitCpuContext<'_>,
    ) -> Option<usize> {
        if !self.enabled || instruction_limit == 0 || self.compiler.is_none() {
            return None;
        }
        let index = (start as usize >> 2) & (CACHE_SLOTS - 1);
        let entry = &mut self.entries[index];
        if entry.start != start || entry.generation != generation {
            *entry = CacheEntry {
                start,
                generation,
                ..CacheEntry::default()
            };
        }
        if let Some(block) = entry.block {
            if block.instruction_count > instruction_limit {
                return None;
            }
            // SAFETY: The compiled function uses this exact ABI and both pointers
            // remain valid for the duration of the call.
            let completed = unsafe {
                (block.function)(cpu.registers.as_mut_ptr(), cpu.cpsr as *mut u32, cpu.heap)
            } as usize;
            return (completed != 0).then_some(completed);
        }
        if entry.failed || self.compile_budget == 0 {
            return None;
        }
        entry.hits = entry.hits.saturating_add(1);
        if entry.hits < HOT_BLOCK_THRESHOLD {
            return None;
        }
        let candidate_len = candidate_len(instructions);
        if candidate_len < MIN_BLOCK_LEN {
            entry.failed = true;
            return None;
        }
        self.compile_budget -= 1;
        match self
            .compiler
            .as_mut()
            .expect("compiler presence checked above")
            .compile(start, &instructions[..candidate_len], cpu.heap_base)
        {
            Ok(Some(block)) => {
                entry.block = Some(block);
                self.compiled_blocks += 1;
                // SAFETY: The compiled function uses this exact ABI and both pointers
                // remain valid for the duration of the call.
                let completed = unsafe {
                    (block.function)(cpu.registers.as_mut_ptr(), cpu.cpsr as *mut u32, cpu.heap)
                } as usize;
                (completed != 0).then_some(completed)
            }
            Ok(None) => {
                entry.failed = true;
                None
            }
            Err(error) => {
                log::warn!("Failed to compile A330 block at {start:#010x}: {error}");
                entry.failed = true;
                None
            }
        }
    }
}

fn candidate_len(instructions: &[DecodedArmInstruction]) -> usize {
    let mut count = 0;
    for decoded in instructions {
        if !instruction_supported(decoded) {
            break;
        }
        count += 1;
        if decoded.kind == ArmInstructionKind::Branch {
            break;
        }
    }
    count
}

fn instruction_supported(decoded: &DecodedArmInstruction) -> bool {
    let instruction = decoded.instruction;
    if instruction >> 28 == 0xf {
        return false;
    }
    match decoded.kind {
        ArmInstructionKind::DataProcessing => {
            let opcode = (instruction >> 21) & 0xf;
            let rn = (instruction >> 16) & 0xf;
            let rd = (instruction >> 12) & 0xf;
            !matches!(opcode, 5..=7) && rn != 15 && rd != 15 && operand2_supported(instruction)
        }
        ArmInstructionKind::SingleTransfer => {
            let rn = (instruction >> 16) & 0xf;
            let rd = (instruction >> 12) & 0xf;
            rn != 15 && rd != 15 && transfer_offset_supported(instruction)
        }
        ArmInstructionKind::HalfTransfer => {
            let rn = (instruction >> 16) & 0xf;
            let rd = (instruction >> 12) & 0xf;
            let kind = (instruction >> 5) & 3;
            let load = instruction & (1 << 20) != 0;
            let valid_transfer = (load && matches!(kind, 1..=3)) || (!load && kind == 1);
            let valid_offset = instruction & (1 << 22) != 0 || instruction & 0xf != 15;
            rn != 15 && rd != 15 && valid_transfer && valid_offset
        }
        ArmInstructionKind::CountLeadingZeros => (instruction >> 12) & 0xf != 15,
        ArmInstructionKind::Multiply => [
            (instruction >> 16) & 0xf,
            (instruction >> 12) & 0xf,
            (instruction >> 8) & 0xf,
            instruction & 0xf,
        ]
        .iter()
        .all(|register| *register != 15),
        ArmInstructionKind::Branch => true,
        _ => false,
    }
}

fn operand2_supported(instruction: u32) -> bool {
    instruction & (1 << 25) != 0
        || (instruction & (1 << 4) == 0
            && instruction & 0xf != 15
            && !((instruction >> 5) & 3 == 3 && (instruction >> 7) & 0x1f == 0))
}

fn transfer_offset_supported(instruction: u32) -> bool {
    instruction & (1 << 25) == 0
        || (instruction & (1 << 4) == 0
            && instruction & 0xf != 15
            && !((instruction >> 5) & 3 == 3 && (instruction >> 7) & 0x1f == 0))
}

impl Compiler {
    fn new() -> anyhow::Result<Self> {
        let mut flag_builder = settings::builder();
        flag_builder.set("opt_level", "none")?;
        flag_builder.set("enable_alias_analysis", "false")?;
        let isa_builder = cranelift_native::builder()
            .map_err(|error| anyhow::anyhow!("unsupported JIT host: {error}"))?;
        let isa = isa_builder.finish(settings::Flags::new(flag_builder))?;
        if isa.pointer_type() != types::I64 {
            anyhow::bail!("A330 JIT requires a 64-bit host");
        }
        let module = JITModule::new(JITBuilder::with_isa(isa, default_libcall_names()));
        let context = module.make_context();
        Ok(Self {
            module,
            context,
            builder_context: FunctionBuilderContext::new(),
            next_function_id: 0,
        })
    }

    fn compile(
        &mut self,
        start: u32,
        instructions: &[DecodedArmInstruction],
        heap_base: u32,
    ) -> anyhow::Result<Option<CompiledBlock>> {
        self.context.clear();
        self.builder_context = FunctionBuilderContext::new();
        let target_config = self.module.target_config();
        let pointer_type = target_config.pointer_type();
        let mut signature = Signature::new(target_config.default_call_conv);
        signature.params.push(AbiParam::new(pointer_type));
        signature.params.push(AbiParam::new(pointer_type));
        signature.params.push(AbiParam::new(pointer_type));
        signature.returns.push(AbiParam::new(types::I64));
        self.context.func = Function::with_name_signature(
            cranelift::codegen::ir::UserFuncName::user(1, self.next_function_id as u32),
            signature.clone(),
        );

        {
            let mut builder =
                FunctionBuilder::new(&mut self.context.func, &mut self.builder_context);
            let entry = builder.create_block();
            builder.append_block_params_for_function_params(entry);
            builder.switch_to_block(entry);
            builder.seal_block(entry);
            let registers = builder.block_params(entry)[0];
            let cpsr = builder.block_params(entry)[1];
            let heap = builder.block_params(entry)[2];
            let mut state = LoweringState::new(registers, cpsr, heap, heap_base);
            lower_block(&mut builder, &mut state, start, instructions);
            builder.finalize(target_config);
        }

        let name = format!("jit_a330_block_{}", self.next_function_id);
        self.next_function_id = self.next_function_id.wrapping_add(1);
        let function_id = self
            .module
            .declare_function(&name, Linkage::Local, &signature)?;
        self.module
            .define_function(function_id, &mut self.context)?;
        self.module.clear_context(&mut self.context);
        self.module.finalize_definitions()?;
        let code = self.module.get_finalized_function(function_id);
        // SAFETY: The emitted function uses the JitBlockFn ABI.
        let function = unsafe { transmute::<*const u8, JitBlockFn>(code) };
        Ok(Some(CompiledBlock {
            function,
            instruction_count: instructions.len(),
        }))
    }
}

struct LoweringState {
    registers: Value,
    cpsr: Value,
    values: [Option<Value>; REGISTER_COUNT],
    dirty: [bool; REGISTER_COUNT],
    cpsr_value: Option<Value>,
    cpsr_dirty: bool,
    heap: Value,
    heap_base: u32,
}

impl LoweringState {
    fn new(registers: Value, cpsr: Value, heap: Value, heap_base: u32) -> Self {
        Self {
            registers,
            cpsr,
            values: [None; REGISTER_COUNT],
            dirty: [false; REGISTER_COUNT],
            cpsr_value: None,
            cpsr_dirty: false,
            heap,
            heap_base,
        }
    }

    fn read(&mut self, builder: &mut FunctionBuilder<'_>, index: usize) -> Value {
        if let Some(value) = self.values[index] {
            return value;
        }
        let value = builder.ins().load(
            types::I32,
            MemFlagsData::new(),
            self.registers,
            (index * 4) as i32,
        );
        self.values[index] = Some(value);
        value
    }

    fn write(&mut self, index: usize, value: Value) {
        self.values[index] = Some(value);
        self.dirty[index] = true;
    }

    fn write_conditionally(
        &mut self,
        builder: &mut FunctionBuilder<'_>,
        index: usize,
        value: Value,
        condition: Value,
    ) {
        let old = self.read(builder, index);
        self.write(index, builder.ins().select(condition, value, old));
    }

    fn read_cpsr(&mut self, builder: &mut FunctionBuilder<'_>) -> Value {
        if let Some(value) = self.cpsr_value {
            return value;
        }
        let value = builder
            .ins()
            .load(types::I32, MemFlagsData::new(), self.cpsr, 0);
        self.cpsr_value = Some(value);
        value
    }

    fn write_cpsr(&mut self, value: Value) {
        self.cpsr_value = Some(value);
        self.cpsr_dirty = true;
    }

    fn flush(&self, builder: &mut FunctionBuilder<'_>) {
        for index in 0..REGISTER_COUNT {
            if self.dirty[index] {
                builder.ins().store(
                    MemFlagsData::new(),
                    self.values[index].expect("dirty register has a value"),
                    self.registers,
                    (index * 4) as i32,
                );
            }
        }
        if self.cpsr_dirty {
            builder.ins().store(
                MemFlagsData::new(),
                self.cpsr_value.expect("dirty CPSR has a value"),
                self.cpsr,
                0,
            );
        }
    }
}

fn lower_block(
    builder: &mut FunctionBuilder<'_>,
    state: &mut LoweringState,
    start: u32,
    instructions: &[DecodedArmInstruction],
) {
    for (index, decoded) in instructions.iter().enumerate() {
        let instruction = decoded.instruction;
        let pc = start.wrapping_add(index as u32 * 4);
        let condition = lower_condition(builder, state, instruction >> 28);
        match decoded.kind {
            ArmInstructionKind::DataProcessing => {
                lower_data_processing(builder, state, instruction, condition)
            }
            ArmInstructionKind::SingleTransfer => {
                lower_single_transfer(builder, state, pc, index, instruction, condition)
            }
            ArmInstructionKind::HalfTransfer => {
                lower_half_transfer(builder, state, pc, index, instruction, condition)
            }
            ArmInstructionKind::CountLeadingZeros => {
                let rd = ((instruction >> 12) & 0xf) as usize;
                let rm = (instruction & 0xf) as usize;
                let value = state.read(builder, rm);
                let result = builder.ins().clz(value);
                state.write_conditionally(builder, rd, result, condition);
            }
            ArmInstructionKind::Multiply => lower_multiply(builder, state, instruction, condition),
            ArmInstructionKind::Branch => {
                lower_branch(builder, state, pc, index + 1, instruction, condition);
                return;
            }
            _ => unreachable!(),
        }
    }
    let next_pc = iconst_u32(builder, start.wrapping_add(instructions.len() as u32 * 4));
    emit_exit(builder, state, next_pc, instructions.len());
}

fn lower_data_processing(
    builder: &mut FunctionBuilder<'_>,
    state: &mut LoweringState,
    instruction: u32,
    condition: Value,
) {
    let opcode = (instruction >> 21) & 0xf;
    let rn = ((instruction >> 16) & 0xf) as usize;
    let rd = ((instruction >> 12) & 0xf) as usize;
    let left = state.read(builder, rn);
    let (right, shifter_carry) = lower_operand2(builder, state, instruction);
    let (result, carry, overflow) = match opcode {
        0 | 8 => (builder.ins().band(left, right), shifter_carry, None),
        1 | 9 => (builder.ins().bxor(left, right), shifter_carry, None),
        2 | 10 => lower_subtract(builder, left, right),
        3 => lower_subtract(builder, right, left),
        4 | 11 => lower_add(builder, left, right),
        12 => (builder.ins().bor(left, right), shifter_carry, None),
        13 => (right, shifter_carry, None),
        14 => (builder.ins().band_not(left, right), shifter_carry, None),
        15 => (builder.ins().bnot(right), shifter_carry, None),
        _ => unreachable!(),
    };
    let test_only = matches!(opcode, 8..=11);
    if !test_only {
        state.write_conditionally(builder, rd, result, condition);
    }
    if instruction & (1 << 20) != 0 || test_only {
        update_flags(builder, state, result, carry, overflow, condition);
    }
}

fn lower_operand2(
    builder: &mut FunctionBuilder<'_>,
    state: &mut LoweringState,
    instruction: u32,
) -> (Value, Option<Value>) {
    if instruction & (1 << 25) != 0 {
        let value = instruction & 0xff;
        let rotate = ((instruction >> 8) & 0xf) * 2;
        let result = iconst_u32(builder, value.rotate_right(rotate));
        let carry = if rotate == 0 {
            flag_value(builder, state, C_FLAG)
        } else {
            bit_is_set(builder, result, 31)
        };
        return (result, Some(carry));
    }
    let value = state.read(builder, (instruction & 0xf) as usize);
    lower_immediate_shift_with_carry(
        builder,
        state,
        value,
        (instruction >> 5) & 3,
        (instruction >> 7) & 0x1f,
    )
}

fn lower_immediate_shift(
    builder: &mut FunctionBuilder<'_>,
    value: Value,
    kind: u32,
    amount: u32,
) -> Value {
    match (kind, amount) {
        (_, 0) if kind == 0 => value,
        (0, _) => builder.ins().ishl_imm_u(value, i64::from(amount)),
        (1, 0) => builder.ins().iconst(types::I32, 0),
        (1, _) => builder.ins().ushr_imm_u(value, i64::from(amount)),
        (2, 0) => builder.ins().sshr_imm_u(value, 31),
        (2, _) => builder.ins().sshr_imm_u(value, i64::from(amount)),
        (3, 0) => unreachable!("RRX is not accepted by candidate validation"),
        (3, _) => builder.ins().rotr_imm_u(value, i64::from(amount)),
        _ => unreachable!(),
    }
}

fn lower_immediate_shift_with_carry(
    builder: &mut FunctionBuilder<'_>,
    state: &mut LoweringState,
    value: Value,
    kind: u32,
    amount: u32,
) -> (Value, Option<Value>) {
    let result = lower_immediate_shift(builder, value, kind, amount);
    let carry = match (kind, amount) {
        (0, 0) => flag_value(builder, state, C_FLAG),
        (0, _) => bit_is_set(builder, value, 32 - amount),
        (1 | 2, 0) => bit_is_set(builder, value, 31),
        (1 | 2, _) | (3, _) => bit_is_set(builder, value, amount - 1),
        _ => unreachable!(),
    };
    (result, Some(carry))
}

fn lower_add(
    builder: &mut FunctionBuilder<'_>,
    left: Value,
    right: Value,
) -> (Value, Option<Value>, Option<Value>) {
    let (result, carry) = builder.ins().uadd_overflow(left, right);
    let (_, overflow) = builder.ins().sadd_overflow(left, right);
    (result, Some(carry), Some(overflow))
}

fn lower_subtract(
    builder: &mut FunctionBuilder<'_>,
    left: Value,
    right: Value,
) -> (Value, Option<Value>, Option<Value>) {
    let (result, borrow) = builder.ins().usub_overflow(left, right);
    let (_, overflow) = builder.ins().ssub_overflow(left, right);
    let carry = invert_bool(builder, borrow);
    (result, Some(carry), Some(overflow))
}

fn lower_multiply(
    builder: &mut FunctionBuilder<'_>,
    state: &mut LoweringState,
    instruction: u32,
    condition: Value,
) {
    let rd = ((instruction >> 16) & 0xf) as usize;
    let rn = ((instruction >> 12) & 0xf) as usize;
    let rs = ((instruction >> 8) & 0xf) as usize;
    let rm = (instruction & 0xf) as usize;
    let left = state.read(builder, rm);
    let right = state.read(builder, rs);
    let mut result = builder.ins().imul(left, right);
    if instruction & (1 << 21) != 0 {
        let accumulator = state.read(builder, rn);
        result = builder.ins().iadd(result, accumulator);
    }
    state.write_conditionally(builder, rd, result, condition);
    if instruction & (1 << 20) != 0 {
        update_flags(builder, state, result, None, None, condition);
    }
}

fn lower_single_transfer(
    builder: &mut FunctionBuilder<'_>,
    state: &mut LoweringState,
    pc: u32,
    completed: usize,
    instruction: u32,
    condition: Value,
) {
    let rn = ((instruction >> 16) & 0xf) as usize;
    let rd = ((instruction >> 12) & 0xf) as usize;
    let base = state.read(builder, rn);
    let offset = if instruction & (1 << 25) == 0 {
        builder
            .ins()
            .iconst(types::I32, i64::from(instruction & 0xfff))
    } else {
        let value = state.read(builder, (instruction & 0xf) as usize);
        lower_immediate_shift(
            builder,
            value,
            (instruction >> 5) & 3,
            (instruction >> 7) & 0x1f,
        )
    };
    let adjusted = if instruction & (1 << 23) != 0 {
        builder.ins().iadd(base, offset)
    } else {
        builder.ins().isub(base, offset)
    };
    let pre = instruction & (1 << 24) != 0;
    let address = if pre { adjusted } else { base };
    let byte = instruction & (1 << 22) != 0;
    let load = instruction & (1 << 20) != 0;
    let store_value = (!load).then(|| state.read(builder, rd));
    let old_load_value = load.then(|| state.read(builder, rd));
    let execute_block = builder.create_block();
    let mapped_block = builder.create_block();
    let bailout_block = builder.create_block();
    let continuation_block = builder.create_block();
    if load {
        builder.append_block_param(continuation_block, types::I32);
    }
    let skipped_arguments = old_load_value.map_or_else(Vec::new, |value| vec![value.into()]);
    builder.ins().brif(
        condition,
        execute_block,
        &[],
        continuation_block,
        &skipped_arguments,
    );

    builder.switch_to_block(execute_block);
    builder.seal_block(execute_block);
    let access_address = if load && !byte {
        builder.ins().band_imm_u(address, i64::from(!3_u32))
    } else {
        address
    };
    let width = if byte { 1 } else { 4 };
    let (host_address, mapped) = heap_address(builder, state, access_address, width, !load);
    builder
        .ins()
        .brif(mapped, mapped_block, &[], bailout_block, &[]);

    builder.switch_to_block(mapped_block);
    builder.seal_block(mapped_block);
    if load {
        let load_type = if byte { types::I8 } else { types::I32 };
        let loaded = builder
            .ins()
            .load(load_type, MemFlagsData::new(), host_address, 0);
        let mut value = if byte {
            builder.ins().uextend(types::I32, loaded)
        } else {
            loaded
        };
        if !byte {
            let rotate = builder.ins().ishl_imm_u(address, 3);
            value = builder.ins().rotr(value, rotate);
        }
        builder.ins().jump(continuation_block, &[value.into()]);
    } else {
        let value = store_value.expect("store value was captured");
        let value = if byte {
            builder.ins().ireduce(types::I8, value)
        } else {
            value
        };
        builder
            .ins()
            .store(MemFlagsData::new(), value, host_address, 0);
        builder.ins().jump(continuation_block, &[]);
    }

    builder.switch_to_block(bailout_block);
    builder.seal_block(bailout_block);
    state.flush(builder);
    let current_pc = iconst_u32(builder, pc);
    builder
        .ins()
        .store(MemFlagsData::new(), current_pc, state.registers, 15 * 4);
    let count = builder.ins().iconst(types::I64, completed as i64);
    builder.ins().return_(&[count]);

    builder.switch_to_block(continuation_block);
    builder.seal_block(continuation_block);
    if load {
        state.write(rd, builder.block_params(continuation_block)[0]);
    }
    if !pre || instruction & (1 << 21) != 0 {
        state.write_conditionally(builder, rn, adjusted, condition);
    }
}

fn lower_half_transfer(
    builder: &mut FunctionBuilder<'_>,
    state: &mut LoweringState,
    pc: u32,
    completed: usize,
    instruction: u32,
    condition: Value,
) {
    let rn = ((instruction >> 16) & 0xf) as usize;
    let rd = ((instruction >> 12) & 0xf) as usize;
    let base = state.read(builder, rn);
    let offset = if instruction & (1 << 22) != 0 {
        let immediate = ((instruction >> 4) & 0xf0) | (instruction & 0xf);
        iconst_u32(builder, immediate)
    } else {
        state.read(builder, (instruction & 0xf) as usize)
    };
    let adjusted = if instruction & (1 << 23) != 0 {
        builder.ins().iadd(base, offset)
    } else {
        builder.ins().isub(base, offset)
    };
    let pre = instruction & (1 << 24) != 0;
    let address = if pre { adjusted } else { base };
    let kind = (instruction >> 5) & 3;
    let load = instruction & (1 << 20) != 0;
    let width = if kind == 2 { 1 } else { 2 };
    let store_value = (!load).then(|| state.read(builder, rd));
    let old_load_value = load.then(|| state.read(builder, rd));
    let execute_block = builder.create_block();
    let mapped_block = builder.create_block();
    let bailout_block = builder.create_block();
    let continuation_block = builder.create_block();
    if load {
        builder.append_block_param(continuation_block, types::I32);
    }
    let skipped_arguments = old_load_value.map_or_else(Vec::new, |value| vec![value.into()]);
    builder.ins().brif(
        condition,
        execute_block,
        &[],
        continuation_block,
        &skipped_arguments,
    );

    builder.switch_to_block(execute_block);
    builder.seal_block(execute_block);
    let (host_address, mapped) = heap_address(builder, state, address, width, !load);
    builder
        .ins()
        .brif(mapped, mapped_block, &[], bailout_block, &[]);

    builder.switch_to_block(mapped_block);
    builder.seal_block(mapped_block);
    if load {
        let load_type = if width == 1 { types::I8 } else { types::I16 };
        let loaded = builder
            .ins()
            .load(load_type, MemFlagsData::new(), host_address, 0);
        let value = if kind == 1 {
            builder.ins().uextend(types::I32, loaded)
        } else {
            builder.ins().sextend(types::I32, loaded)
        };
        builder.ins().jump(continuation_block, &[value.into()]);
    } else {
        let value = builder
            .ins()
            .ireduce(types::I16, store_value.expect("store value was captured"));
        builder
            .ins()
            .store(MemFlagsData::new(), value, host_address, 0);
        builder.ins().jump(continuation_block, &[]);
    }

    builder.switch_to_block(bailout_block);
    builder.seal_block(bailout_block);
    state.flush(builder);
    let current_pc = iconst_u32(builder, pc);
    builder
        .ins()
        .store(MemFlagsData::new(), current_pc, state.registers, 15 * 4);
    let count = builder.ins().iconst(types::I64, completed as i64);
    builder.ins().return_(&[count]);

    builder.switch_to_block(continuation_block);
    builder.seal_block(continuation_block);
    if load {
        state.write(rd, builder.block_params(continuation_block)[0]);
    }
    if !pre || instruction & (1 << 21) != 0 {
        state.write_conditionally(builder, rn, adjusted, condition);
    }
}

fn heap_address(
    builder: &mut FunctionBuilder<'_>,
    state: &LoweringState,
    address: Value,
    width: u32,
    write: bool,
) -> (Value, Value) {
    let offset = builder
        .ins()
        .iadd_imm_s(address, -i64::from(state.heap_base));
    let mut mapped = builder.ins().icmp_imm_u(
        IntCC::UnsignedLessThanOrEqual,
        offset,
        (HEAP_SIZE as u32 - width) as i64,
    );
    if write && state.heap_base == HOMEBREW_HEAP_BASE {
        let access_end = builder.ins().iadd_imm_s(address, i64::from(width));
        let before_mmio = builder.ins().icmp_imm_u(
            IntCC::UnsignedLessThanOrEqual,
            access_end,
            i64::from(LEGACY_SYSTEM_MMIO_BASE),
        );
        let after_mmio = builder.ins().icmp_imm_u(
            IntCC::UnsignedGreaterThanOrEqual,
            address,
            i64::from(LEGACY_SYSTEM_MMIO_BASE + LEGACY_SYSTEM_MMIO_SIZE as u32),
        );
        let outside_mmio = builder.ins().bor(before_mmio, after_mmio);
        mapped = builder.ins().band(mapped, outside_mmio);
    }
    let offset = builder.ins().uextend(types::I64, offset);
    (builder.ins().iadd(state.heap, offset), mapped)
}

fn lower_branch(
    builder: &mut FunctionBuilder<'_>,
    state: &mut LoweringState,
    pc: u32,
    completed: usize,
    instruction: u32,
    condition: Value,
) {
    let offset = (((instruction & 0x00ff_ffff) << 8) as i32 >> 6) as u32;
    let target = iconst_u32(builder, pc.wrapping_add(8).wrapping_add(offset));
    let fallthrough = iconst_u32(builder, pc.wrapping_add(4));
    let next_pc = builder.ins().select(condition, target, fallthrough);
    if instruction & (1 << 24) != 0 {
        let link = iconst_u32(builder, pc.wrapping_add(4));
        state.write_conditionally(builder, 14, link, condition);
    }
    emit_exit(builder, state, next_pc, completed);
}

fn emit_exit(
    builder: &mut FunctionBuilder<'_>,
    state: &mut LoweringState,
    next_pc: Value,
    completed: usize,
) {
    state.write(15, next_pc);
    state.flush(builder);
    let count = builder.ins().iconst(types::I64, completed as i64);
    builder.ins().return_(&[count]);
}

fn lower_condition(
    builder: &mut FunctionBuilder<'_>,
    state: &mut LoweringState,
    condition: u32,
) -> Value {
    if condition == 14 {
        return builder.ins().iconst(types::I8, 1);
    }
    // Each condition reads only the flags it needs, so hot conditional code
    // lowers to the smallest possible instruction sequence.
    match condition {
        0 => flag_value(builder, state, Z_FLAG),
        1 => {
            let z = flag_value(builder, state, Z_FLAG);
            invert_bool(builder, z)
        }
        2 => flag_value(builder, state, C_FLAG),
        3 => {
            let c = flag_value(builder, state, C_FLAG);
            invert_bool(builder, c)
        }
        4 => flag_value(builder, state, N_FLAG),
        5 => {
            let n = flag_value(builder, state, N_FLAG);
            invert_bool(builder, n)
        }
        6 => flag_value(builder, state, V_FLAG),
        7 => {
            let v = flag_value(builder, state, V_FLAG);
            invert_bool(builder, v)
        }
        8 => {
            let z = flag_value(builder, state, Z_FLAG);
            let not_z = invert_bool(builder, z);
            let c = flag_value(builder, state, C_FLAG);
            builder.ins().band(c, not_z)
        }
        9 => {
            let c = flag_value(builder, state, C_FLAG);
            let not_c = invert_bool(builder, c);
            let z = flag_value(builder, state, Z_FLAG);
            builder.ins().bor(not_c, z)
        }
        10 | 11 => {
            let n = flag_value(builder, state, N_FLAG);
            let v = flag_value(builder, state, V_FLAG);
            let equal = builder.ins().icmp(IntCC::Equal, n, v);
            if condition == 10 {
                equal
            } else {
                invert_bool(builder, equal)
            }
        }
        12 => {
            let z = flag_value(builder, state, Z_FLAG);
            let not_z = invert_bool(builder, z);
            let n = flag_value(builder, state, N_FLAG);
            let v = flag_value(builder, state, V_FLAG);
            let equal = builder.ins().icmp(IntCC::Equal, n, v);
            builder.ins().band(not_z, equal)
        }
        13 => {
            let n = flag_value(builder, state, N_FLAG);
            let v = flag_value(builder, state, V_FLAG);
            let unequal = builder.ins().icmp(IntCC::NotEqual, n, v);
            let z = flag_value(builder, state, Z_FLAG);
            builder.ins().bor(z, unequal)
        }
        _ => unreachable!("condition support checked before lowering"),
    }
}

fn flag_value(builder: &mut FunctionBuilder<'_>, state: &mut LoweringState, flag: u32) -> Value {
    let cpsr = state.read_cpsr(builder);
    let masked = builder.ins().band_imm_u(cpsr, i64::from(flag));
    builder.ins().icmp_imm_u(IntCC::NotEqual, masked, 0)
}

fn bit_is_set(builder: &mut FunctionBuilder<'_>, value: Value, bit: u32) -> Value {
    let shifted = builder.ins().ushr_imm_u(value, i64::from(bit));
    let masked = builder.ins().band_imm_u(shifted, 1);
    builder.ins().icmp_imm_u(IntCC::NotEqual, masked, 0)
}

fn invert_bool(builder: &mut FunctionBuilder<'_>, value: Value) -> Value {
    builder.ins().icmp_imm_u(IntCC::Equal, value, 0)
}

fn update_flags(
    builder: &mut FunctionBuilder<'_>,
    state: &mut LoweringState,
    result: Value,
    carry: Option<Value>,
    overflow: Option<Value>,
    condition: Value,
) {
    let old = state.read_cpsr(builder);
    // The N flag sits at bit 31 exactly like in the result, and every other
    // flag is a shifted boolean, so the new flag bits are built as one mask
    // instead of one select per flag.
    let mut mask = builder.ins().band_imm_u(result, i64::from(N_FLAG));
    let zero = builder.ins().icmp_imm_u(IntCC::Equal, result, 0);
    let zero_bit = flag_bit(builder, zero, Z_FLAG);
    mask = builder.ins().bor(mask, zero_bit);
    if let Some(carry) = carry {
        let carry_bit = flag_bit(builder, carry, C_FLAG);
        mask = builder.ins().bor(mask, carry_bit);
    }
    if let Some(overflow) = overflow {
        let overflow_bit = flag_bit(builder, overflow, V_FLAG);
        mask = builder.ins().bor(mask, overflow_bit);
    }
    let mut clear_mask = N_FLAG | Z_FLAG;
    if carry.is_some() {
        clear_mask |= C_FLAG;
    }
    if overflow.is_some() {
        clear_mask |= V_FLAG;
    }
    let cleared = builder.ins().band_imm_u(old, i64::from(!clear_mask));
    let updated = builder.ins().bor(cleared, mask);
    state.write_cpsr(builder.ins().select(condition, updated, old));
}

/// Moves a boolean value into its CPSR flag bit position.
fn flag_bit(builder: &mut FunctionBuilder<'_>, enabled: Value, flag: u32) -> Value {
    let widened = builder.ins().uextend(types::I32, enabled);
    builder
        .ins()
        .ishl_imm_u(widened, i64::from(flag.trailing_zeros()))
}

fn iconst_u32(builder: &mut FunctionBuilder<'_>, value: u32) -> Value {
    builder.ins().iconst(types::I32, i64::from(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiled_integer_block_updates_registers_and_pc() {
        let start = 0x1000_1000;
        let instructions = [
            DecodedArmInstruction::decode(0xe280_0001),
            DecodedArmInstruction::decode(0xe1a0_1080),
            DecodedArmInstruction::decode(0xe242_2002),
        ];
        let mut compiler = Compiler::new().unwrap();
        let block = compiler
            .compile(start, &instructions, 0x2100_0000)
            .unwrap()
            .unwrap();
        let mut registers = [0_u32; REGISTER_COUNT];
        let mut cpsr = 0_u32;
        registers[0] = 10;
        registers[2] = 5;

        // SAFETY: The compiler created this function and the block performs no memory access.
        let completed =
            unsafe { (block.function)(registers.as_mut_ptr(), &mut cpsr, std::ptr::null_mut()) };

        assert_eq!(completed, 3);
        assert_eq!(registers[0], 11);
        assert_eq!(registers[1], 22);
        assert_eq!(registers[2], 3);
        assert_eq!(registers[15], start + 12);
    }

    #[test]
    fn compiled_flags_conditions_and_branch_match_arm_semantics() {
        let start = 0x1000_1000;
        let instructions = [
            DecodedArmInstruction::decode(0xe350_000a),
            DecodedArmInstruction::decode(0x03a0_1007),
            DecodedArmInstruction::decode(0x1281_1001),
            DecodedArmInstruction::decode(0xea00_0001),
        ];
        let mut compiler = Compiler::new().unwrap();
        let block = compiler
            .compile(start, &instructions, 0x2100_0000)
            .unwrap()
            .unwrap();
        let mut equal_registers = [0_u32; REGISTER_COUNT];
        let mut equal_cpsr = 0_u32;
        equal_registers[0] = 10;

        // SAFETY: The compiler created this function and the block performs no memory access.
        let equal_completed = unsafe {
            (block.function)(
                equal_registers.as_mut_ptr(),
                &mut equal_cpsr,
                std::ptr::null_mut(),
            )
        };

        assert_eq!(equal_completed, 4);
        assert_eq!(equal_registers[1], 7);
        assert_eq!(equal_registers[15], start + 24);
        assert_eq!(
            equal_cpsr & (N_FLAG | Z_FLAG | C_FLAG | V_FLAG),
            Z_FLAG | C_FLAG
        );

        let mut different_registers = [0_u32; REGISTER_COUNT];
        let mut different_cpsr = 0_u32;
        different_registers[0] = 9;

        // SAFETY: The compiler created this function and the block performs no memory access.
        let different_completed = unsafe {
            (block.function)(
                different_registers.as_mut_ptr(),
                &mut different_cpsr,
                std::ptr::null_mut(),
            )
        };

        assert_eq!(different_completed, 4);
        assert_eq!(different_registers[1], 1);
        assert_eq!(different_registers[15], start + 24);
        assert_eq!(different_cpsr & (N_FLAG | Z_FLAG | C_FLAG | V_FLAG), N_FLAG);
    }

    #[test]
    fn compiled_block_accesses_heap_directly() {
        let start = 0x1000_1000;
        let heap_base = 0x2100_0000;
        let instructions = [
            DecodedArmInstruction::decode(0xe590_1000),
            DecodedArmInstruction::decode(0xe281_1001),
            DecodedArmInstruction::decode(0xe580_1004),
        ];
        let mut heap = vec![0_u8; 0x1000];
        heap[0x100..0x104].copy_from_slice(&41_u32.to_le_bytes());
        let mut compiler = Compiler::new().unwrap();
        let block = compiler
            .compile(start, &instructions, heap_base)
            .unwrap()
            .unwrap();
        let mut registers = [0_u32; REGISTER_COUNT];
        let mut cpsr = 0_u32;
        registers[0] = heap_base + 0x100;

        // SAFETY: The supplied heap contains both guest memory accesses.
        let completed =
            unsafe { (block.function)(registers.as_mut_ptr(), &mut cpsr, heap.as_mut_ptr()) };

        assert_eq!(completed, 3);
        assert_eq!(registers[1], 42);
        assert_eq!(
            u32::from_le_bytes(heap[0x104..0x108].try_into().unwrap()),
            42
        );
        assert_eq!(registers[15], start + 12);
    }

    #[test]
    fn compiled_block_accesses_halfwords_directly() {
        let start = 0x1000_1000;
        let heap_base = 0x2100_0000;
        let instructions = [
            DecodedArmInstruction::decode(0xe1c0_10b2),
            DecodedArmInstruction::decode(0xe1d0_20b2),
            DecodedArmInstruction::decode(0xe1d0_30d1),
            DecodedArmInstruction::decode(0xe1d0_40f2),
        ];
        let mut heap = vec![0_u8; 0x1000];
        heap[0x101] = 0x80;
        let mut compiler = Compiler::new().unwrap();
        let block = compiler
            .compile(start, &instructions, heap_base)
            .unwrap()
            .unwrap();
        let mut registers = [0_u32; REGISTER_COUNT];
        let mut cpsr = 0_u32;
        registers[0] = heap_base + 0x100;
        registers[1] = 0xff80;

        // SAFETY: The supplied heap contains every guest memory access.
        let completed =
            unsafe { (block.function)(registers.as_mut_ptr(), &mut cpsr, heap.as_mut_ptr()) };

        assert_eq!(completed, 4);
        assert_eq!(registers[2], 0xff80);
        assert_eq!(registers[3], 0xffff_ff80);
        assert_eq!(registers[4], 0xffff_ff80);
        assert_eq!(registers[15], start + 16);
    }

    #[test]
    fn compiled_block_bails_out_before_unmapped_access() {
        let start = 0x1000_1000;
        let instructions = [
            DecodedArmInstruction::decode(0xe590_1000),
            DecodedArmInstruction::decode(0xe281_1001),
            DecodedArmInstruction::decode(0xe580_1004),
        ];
        let mut compiler = Compiler::new().unwrap();
        let block = compiler
            .compile(start, &instructions, 0x2100_0000)
            .unwrap()
            .unwrap();
        let mut registers = [0_u32; REGISTER_COUNT];
        let mut cpsr = 0_u32;
        registers[0] = 0x1000_0200;
        registers[1] = 9;

        // SAFETY: The unmapped access exits before dereferencing the null heap pointer.
        let completed =
            unsafe { (block.function)(registers.as_mut_ptr(), &mut cpsr, std::ptr::null_mut()) };

        assert_eq!(completed, 0);
        assert_eq!(registers[1], 9);
        assert_eq!(registers[15], start);
    }

    #[test]
    fn candidate_accepts_stateful_and_conditional_instructions() {
        let instructions = [
            DecodedArmInstruction::decode(0xe280_0001),
            DecodedArmInstruction::decode(0xe590_1000),
            DecodedArmInstruction::decode(0xe580_1000),
            DecodedArmInstruction::decode(0x1280_0001),
        ];

        assert_eq!(candidate_len(&instructions), 4);
    }
}
