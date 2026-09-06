use super::cpu::{ArmInstructionKind, DecodedArmInstruction};
use super::runtime::{jit_read32, jit_read8};
use cranelift::codegen::ir::{
    types, AbiParam, Function, InstBuilder, MemFlagsData, Signature, Value,
};
use cranelift::codegen::settings::{self, Configurable};
use cranelift::frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{default_libcall_names, Linkage, Module};
use std::mem::transmute;

const HOT_BLOCK_THRESHOLD: u16 = 128;
const MIN_BLOCK_LEN: usize = 3;
const CACHE_SLOTS: usize = 4_096;
const MAX_COMPILES_PER_SLICE: u8 = 8;
const REGISTER_COUNT: usize = 16;

type JitBlockFn = unsafe extern "C" fn(*mut u32, *mut u8) -> u64;

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
        registers: &mut [u32; REGISTER_COUNT],
        bus: *mut u8,
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
            let completed = unsafe { (block.function)(registers.as_mut_ptr(), bus) } as usize;
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
            .compile(start, &instructions[..candidate_len])
        {
            Ok(Some(block)) => {
                entry.block = Some(block);
                self.compiled_blocks += 1;
                // SAFETY: The compiled function uses this exact ABI and both pointers
                // remain valid for the duration of the call.
                let completed = unsafe { (block.function)(registers.as_mut_ptr(), bus) } as usize;
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
    instructions
        .iter()
        .take_while(|decoded| instruction_supported(decoded))
        .count()
}

fn instruction_supported(decoded: &DecodedArmInstruction) -> bool {
    let instruction = decoded.instruction;
    if instruction >> 28 != 0xe {
        return false;
    }
    match decoded.kind {
        ArmInstructionKind::DataProcessing => {
            let opcode = (instruction >> 21) & 0xf;
            let set_flags = instruction & (1 << 20) != 0;
            let rn = (instruction >> 16) & 0xf;
            let rd = (instruction >> 12) & 0xf;
            !set_flags
                && !matches!(opcode, 5..=11)
                && rn != 15
                && rd != 15
                && operand2_supported(instruction)
        }
        ArmInstructionKind::SingleTransfer => {
            let load = instruction & (1 << 20) != 0;
            let rn = (instruction >> 16) & 0xf;
            let rd = (instruction >> 12) & 0xf;
            load && rn != 15 && rd != 15 && transfer_offset_supported(instruction)
        }
        ArmInstructionKind::CountLeadingZeros => (instruction >> 12) & 0xf != 15,
        ArmInstructionKind::Multiply => {
            instruction & (1 << 20) == 0
                && [
                    (instruction >> 16) & 0xf,
                    (instruction >> 12) & 0xf,
                    (instruction >> 8) & 0xf,
                    instruction & 0xf,
                ]
                .iter()
                .all(|register| *register != 15)
        }
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
        flag_builder.set("opt_level", "speed")?;
        flag_builder.set("enable_alias_analysis", "true")?;
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
    ) -> anyhow::Result<Option<CompiledBlock>> {
        self.context.clear();
        self.builder_context = FunctionBuilderContext::new();
        let target_config = self.module.target_config();
        let pointer_type = target_config.pointer_type();
        let mut signature = Signature::new(target_config.default_call_conv);
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
            let bus = builder.block_params(entry)[1];
            let mut state = LoweringState::new(registers);
            for (index, decoded) in instructions.iter().enumerate() {
                lower_instruction(
                    &mut builder,
                    &mut state,
                    bus,
                    start.wrapping_add(index as u32 * 4),
                    index,
                    decoded.instruction,
                    decoded.kind,
                );
            }
            state.flush(&mut builder);
            let next_pc = builder.ins().iconst(
                types::I32,
                i64::from(start.wrapping_add(instructions.len() as u32 * 4)),
            );
            builder
                .ins()
                .store(MemFlagsData::new(), next_pc, registers, 15 * 4);
            let completed = builder.ins().iconst(types::I64, instructions.len() as i64);
            builder.ins().return_(&[completed]);
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
    values: [Option<Value>; REGISTER_COUNT],
    dirty: [bool; REGISTER_COUNT],
}

impl LoweringState {
    fn new(registers: Value) -> Self {
        Self {
            registers,
            values: [None; REGISTER_COUNT],
            dirty: [false; REGISTER_COUNT],
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
    }
}

fn lower_instruction(
    builder: &mut FunctionBuilder<'_>,
    state: &mut LoweringState,
    bus: Value,
    pc: u32,
    completed: usize,
    instruction: u32,
    kind: ArmInstructionKind,
) {
    match kind {
        ArmInstructionKind::DataProcessing => lower_data_processing(builder, state, instruction),
        ArmInstructionKind::SingleTransfer => {
            lower_single_transfer(builder, state, bus, pc, completed, instruction)
        }
        ArmInstructionKind::CountLeadingZeros => {
            let rd = ((instruction >> 12) & 0xf) as usize;
            let rm = (instruction & 0xf) as usize;
            let value = state.read(builder, rm);
            let result = builder.ins().clz(value);
            state.write(rd, result);
        }
        ArmInstructionKind::Multiply => {
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
            state.write(rd, result);
        }
        _ => unreachable!(),
    }
}

fn lower_data_processing(
    builder: &mut FunctionBuilder<'_>,
    state: &mut LoweringState,
    instruction: u32,
) {
    let opcode = (instruction >> 21) & 0xf;
    let rn = ((instruction >> 16) & 0xf) as usize;
    let rd = ((instruction >> 12) & 0xf) as usize;
    let left = state.read(builder, rn);
    let right = lower_operand2(builder, state, instruction);
    let result = match opcode {
        0 => builder.ins().band(left, right),
        1 => builder.ins().bxor(left, right),
        2 => builder.ins().isub(left, right),
        3 => builder.ins().isub(right, left),
        4 => builder.ins().iadd(left, right),
        12 => builder.ins().bor(left, right),
        13 => right,
        14 => builder.ins().band_not(left, right),
        15 => builder.ins().bnot(right),
        _ => unreachable!(),
    };
    state.write(rd, result);
}

fn lower_operand2(
    builder: &mut FunctionBuilder<'_>,
    state: &mut LoweringState,
    instruction: u32,
) -> Value {
    if instruction & (1 << 25) != 0 {
        let value = instruction & 0xff;
        let rotate = ((instruction >> 8) & 0xf) * 2;
        return builder
            .ins()
            .iconst(types::I32, i64::from(value.rotate_right(rotate)));
    }
    let value = state.read(builder, (instruction & 0xf) as usize);
    lower_immediate_shift(
        builder,
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

fn lower_single_transfer(
    builder: &mut FunctionBuilder<'_>,
    state: &mut LoweringState,
    bus: Value,
    pc: u32,
    completed: usize,
    instruction: u32,
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
    let callback = if byte { jit_read8 } else { jit_read32 };
    let packed = emit_read_call(builder, bus, address, callback as usize);
    let success_bits = builder.ins().ushr_imm_u(packed, 32);
    let failed = builder
        .ins()
        .icmp_imm_u(cranelift::prelude::IntCC::Equal, success_bits, 0);
    let failure_block = builder.create_block();
    let success_block = builder.create_block();
    builder
        .ins()
        .brif(failed, failure_block, &[], success_block, &[]);

    builder.switch_to_block(failure_block);
    builder.seal_block(failure_block);
    state.flush(builder);
    let current_pc = builder.ins().iconst(types::I32, i64::from(pc));
    builder
        .ins()
        .store(MemFlagsData::new(), current_pc, state.registers, 15 * 4);
    let count = builder.ins().iconst(types::I64, completed as i64);
    builder.ins().return_(&[count]);

    builder.switch_to_block(success_block);
    builder.seal_block(success_block);
    let mut value = builder.ins().ireduce(types::I32, packed);
    if !byte {
        let rotate = builder.ins().ishl_imm_u(address, 3);
        value = builder.ins().rotr(value, rotate);
    }
    state.write(rd, value);
    if !pre || instruction & (1 << 21) != 0 {
        state.write(rn, adjusted);
    }
}

fn emit_read_call(
    builder: &mut FunctionBuilder<'_>,
    bus: Value,
    address: Value,
    callback: usize,
) -> Value {
    let mut signature = Signature::new(builder.func.signature.call_conv);
    signature.params.push(AbiParam::new(types::I64));
    signature.params.push(AbiParam::new(types::I32));
    signature.returns.push(AbiParam::new(types::I64));
    let signature = builder.import_signature(signature);
    let function = builder.ins().iconst(types::I64, callback as i64);
    let call = builder
        .ins()
        .call_indirect(signature, function, &[bus, address]);
    builder.inst_results(call)[0]
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
        let block = compiler.compile(start, &instructions).unwrap().unwrap();
        let mut registers = [0_u32; REGISTER_COUNT];
        registers[0] = 10;
        registers[2] = 5;

        // SAFETY: The compiler created this function and the block performs no memory access.
        let completed = unsafe { (block.function)(registers.as_mut_ptr(), std::ptr::null_mut()) };

        assert_eq!(completed, 3);
        assert_eq!(registers[0], 11);
        assert_eq!(registers[1], 22);
        assert_eq!(registers[2], 3);
        assert_eq!(registers[15], start + 12);
    }

    #[test]
    fn candidate_stops_before_stateful_or_conditional_instructions() {
        let instructions = [
            DecodedArmInstruction::decode(0xe280_0001),
            DecodedArmInstruction::decode(0xe590_1000),
            DecodedArmInstruction::decode(0xe580_1000),
            DecodedArmInstruction::decode(0x1280_0001),
        ];

        assert_eq!(candidate_len(&instructions), 2);
    }
}
