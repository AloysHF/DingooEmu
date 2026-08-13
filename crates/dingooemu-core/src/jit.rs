use crate::cpu::Registers;
use cranelift::codegen::ir::{types, AbiParam, Function, InstBuilder, MemFlags, Signature, Value};
use cranelift::codegen::settings::{self, Configurable};
use cranelift::frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift::prelude::IntCC;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{default_libcall_names, Linkage, Module};
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::mem::{offset_of, transmute};
use std::time::Instant;

const HOT_BLOCK_THRESHOLD: u16 = 256;
const MIN_COMPILED_BLOCK_LEN: usize = 4;
const MAX_JIT_CACHE_ENTRIES: usize = 32_768;
const FAST_JIT_CACHE_SLOTS: usize = 4_096;
const MAX_ZERO_INSTRUCTION_EXITS: u8 = 4;
const COMPILE_COOLDOWN_FRAMES: u8 = 3;
const REGISTER_COUNT: usize = 34;
const HI_INDEX: usize = 32;
const LO_INDEX: usize = 33;
const KSEG_MASK: u32 = 0x1fff_ffff;

type JitBlockFn = unsafe extern "C" fn(*mut Registers, *mut u8, *mut u8) -> u64;

#[derive(Clone, Copy)]
struct CompiledBlock {
    function: JitBlockFn,
    instruction_count: usize,
}

#[derive(Clone, Copy, Default)]
struct FastJitCacheEntry {
    start: u32,
    block: Option<CompiledBlock>,
}

#[derive(Default)]
struct JitCacheEntry {
    hits: u16,
    failed: bool,
    zero_instruction_exits: u8,
    block: Option<CompiledBlock>,
}

#[derive(Default)]
struct GuestAddressHasher(u64);

impl Hasher for GuestAddressHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        self.0 = bytes.iter().fold(0, |hash, &byte| {
            hash.wrapping_mul(16777619) ^ u64::from(byte)
        });
    }

    fn write_u32(&mut self, value: u32) {
        self.0 = u64::from(value >> 2);
    }
}

type JitEntryMap = HashMap<u32, JitCacheEntry, BuildHasherDefault<GuestAddressHasher>>;

struct Compiler {
    module: JITModule,
    context: cranelift::codegen::Context,
    builder_context: FunctionBuilderContext,
    next_function_id: u64,
}

pub(crate) struct JitEngine {
    compiler: Option<Compiler>,
    entries: JitEntryMap,
    fast_entries: Box<[FastJitCacheEntry]>,
    enabled: bool,
    compile_budget: u8,
    compile_cooldown: u8,
    compiled_block_count: u64,
}

impl JitEngine {
    pub(crate) fn new() -> Self {
        let compiler = match Compiler::new() {
            Ok(compiler) => {
                log::info!("MIPS JIT backend initialized");
                Some(compiler)
            }
            Err(error) => {
                log::warn!("MIPS JIT backend unavailable: {error}");
                None
            }
        };
        Self {
            compiler,
            entries: HashMap::with_capacity_and_hasher(4_096, BuildHasherDefault::default()),
            fast_entries: vec![FastJitCacheEntry::default(); FAST_JIT_CACHE_SLOTS]
                .into_boxed_slice(),
            enabled: true,
            compile_budget: 1,
            compile_cooldown: 0,
            compiled_block_count: 0,
        }
    }

    pub(crate) fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.fast_entries.fill(FastJitCacheEntry::default());
        if self.compiled_block_count != 0 {
            // Discard every function pointer before dropping the module that owns it.
            if let Some(compiler) = self.compiler.take() {
                drop(compiler);
                self.compiler = match Compiler::new() {
                    Ok(compiler) => Some(compiler),
                    Err(error) => {
                        log::warn!(
                            "MIPS JIT backend unavailable after cache invalidation: {error}"
                        );
                        None
                    }
                };
            }
            self.compiled_block_count = 0;
        }
        self.compile_budget = 1;
        self.compile_cooldown = 0;
    }

    pub(crate) fn begin_frame(&mut self) {
        if self.compile_cooldown == 0 {
            self.compile_budget = 1;
        } else {
            self.compile_cooldown -= 1;
            self.compile_budget = 0;
        }
    }

    pub(crate) fn execute(
        &mut self,
        start: u32,
        instructions: &[u32],
        instruction_limit: usize,
        registers: &mut Registers,
        ram: *mut u8,
        framebuffer: *mut u8,
    ) -> Option<u64> {
        if !self.enabled || instruction_limit == 0 || self.compiler.is_none() {
            return None;
        }

        let fast_index = (start as usize >> 2) & (FAST_JIT_CACHE_SLOTS - 1);
        let fast_entry = self.fast_entries[fast_index];
        if fast_entry.start == start {
            if let Some(block) = fast_entry.block {
                let completed =
                    execute_compiled_block(block, instruction_limit, registers, ram, framebuffer);
                if let Some(completed) = completed {
                    if completed != 0 {
                        return Some(completed);
                    }
                    if let Some(entry) = self.entries.get_mut(&start) {
                        entry.zero_instruction_exits =
                            entry.zero_instruction_exits.saturating_add(1);
                        if entry.zero_instruction_exits >= MAX_ZERO_INSTRUCTION_EXITS {
                            entry.block = None;
                            entry.failed = true;
                            self.fast_entries[fast_index].block = None;
                        }
                    }
                }
                return None;
            }
        }

        let at_capacity = self.entries.len() >= MAX_JIT_CACHE_ENTRIES;
        let entry = match self.entries.entry(start) {
            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::hash_map::Entry::Vacant(entry) => {
                if at_capacity {
                    return None;
                }
                entry.insert(JitCacheEntry::default())
            }
        };

        if let Some(block) = entry.block {
            self.fast_entries[fast_index] = FastJitCacheEntry {
                start,
                block: Some(block),
            };
            let completed =
                execute_compiled_block(block, instruction_limit, registers, ram, framebuffer);
            if let Some(completed) = completed {
                if completed == 0 {
                    entry.zero_instruction_exits = entry.zero_instruction_exits.saturating_add(1);
                    if entry.zero_instruction_exits >= MAX_ZERO_INSTRUCTION_EXITS {
                        entry.block = None;
                        entry.failed = true;
                        self.fast_entries[fast_index].block = None;
                    }
                    return None;
                }
                entry.zero_instruction_exits = 0;
                return Some(completed);
            }
            return None;
        }
        if entry.failed {
            return None;
        }

        entry.hits = entry.hits.saturating_add(1);
        if entry.hits < HOT_BLOCK_THRESHOLD || self.compile_budget == 0 {
            return None;
        }
        if candidate_instruction_count(instructions) < MIN_COMPILED_BLOCK_LEN {
            entry.failed = true;
            return None;
        }

        self.compile_budget = 0;
        self.compile_cooldown = COMPILE_COOLDOWN_FRAMES;
        let compile_start = Instant::now();
        let compile_result = self
            .compiler
            .as_mut()
            .expect("compiler presence checked above")
            .compile(start, instructions);
        match compile_result {
            Ok(Some(block)) => {
                self.compiled_block_count = self.compiled_block_count.wrapping_add(1);
                log::debug!(
                    "JIT compiled block {start:#010x}: instructions={} elapsed_us={} total={}",
                    block.instruction_count,
                    compile_start.elapsed().as_micros(),
                    self.compiled_block_count
                );
                entry.block = Some(block);
                self.fast_entries[fast_index] = FastJitCacheEntry {
                    start,
                    block: Some(block),
                };
                execute_compiled_block(block, instruction_limit, registers, ram, framebuffer)
                    .filter(|&count| count != 0)
            }
            Ok(None) => {
                entry.failed = true;
                None
            }
            Err(error) => {
                log::warn!("Failed to compile MIPS block at {start:#010x}: {error}");
                entry.failed = true;
                None
            }
        }
    }
}

fn execute_compiled_block(
    block: CompiledBlock,
    instruction_limit: usize,
    registers: &mut Registers,
    ram: *mut u8,
    framebuffer: *mut u8,
) -> Option<u64> {
    if block.instruction_count > instruction_limit {
        return None;
    }
    // SAFETY: Cranelift emitted this function with the exact signature below.
    // Registers has a stable C layout and both pointers remain valid for the call.
    let completed = unsafe { (block.function)(registers, ram, framebuffer) };
    Some(completed)
}

fn candidate_instruction_count(instructions: &[u32]) -> usize {
    let mut index = 0;
    while index < instructions.len() {
        let instruction = instructions[index];
        if is_control_flow(instruction) {
            return if index + 1 < instructions.len()
                && is_supported_delay_instruction(instructions[index + 1])
                && lowerable_branch(instruction)
            {
                index + 2
            } else {
                index
            };
        }
        if !is_memory_instruction(instruction) && !is_supported_register_instruction(instruction) {
            return index;
        }
        index += 1;
    }
    index
}

fn lowerable_branch(instruction: u32) -> bool {
    match instruction >> 26 {
        0 => matches!(instruction & 0x3f, 0x08 | 0x09),
        0x01 => matches!((instruction >> 16) & 0x1f, 0x00 | 0x01 | 0x10 | 0x11),
        0x02..=0x07 => true,
        _ => false,
    }
}

impl Compiler {
    fn new() -> anyhow::Result<Self> {
        let mut flag_builder = settings::builder();
        flag_builder.set("opt_level", "speed")?;
        flag_builder.set("enable_alias_analysis", "true")?;
        let isa_builder = cranelift_native::builder()
            .map_err(|error| anyhow::anyhow!("unsupported JIT host: {error}"))?;
        let isa = isa_builder.finish(settings::Flags::new(flag_builder))?;
        let builder = JITBuilder::with_isa(isa, default_libcall_names());
        let module = JITModule::new(builder);
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
        instructions: &[u32],
    ) -> anyhow::Result<Option<CompiledBlock>> {
        self.context.clear();
        self.builder_context = FunctionBuilderContext::new();
        let target_config = self.module.target_config();
        let pointer_type = target_config.pointer_type();
        debug_assert_eq!(pointer_type, types::I64);

        let mut signature = Signature::new(target_config.default_call_conv);
        signature.params.push(AbiParam::new(pointer_type));
        signature.params.push(AbiParam::new(pointer_type));
        signature.params.push(AbiParam::new(pointer_type));
        signature.returns.push(AbiParam::new(types::I64));
        self.context.func = Function::with_name_signature(
            cranelift::codegen::ir::UserFuncName::user(0, self.next_function_id as u32),
            signature.clone(),
        );

        let instruction_count = {
            let mut builder =
                FunctionBuilder::new(&mut self.context.func, &mut self.builder_context);
            let entry = builder.create_block();
            builder.append_block_params_for_function_params(entry);
            builder.switch_to_block(entry);
            builder.seal_block(entry);
            let registers = builder.block_params(entry)[0];
            let ram = builder.block_params(entry)[1];
            let framebuffer = builder.block_params(entry)[2];
            let mut state = LoweringState::new(registers, ram, framebuffer);
            let count = lower_block(&mut builder, &mut state, start, instructions);
            if count != 0 {
                builder.finalize();
            }
            count
        };

        if instruction_count == 0 {
            return Ok(None);
        }

        let name = format!("jit_mips_block_{}", self.next_function_id);
        self.next_function_id = self.next_function_id.wrapping_add(1);
        let function_id = self
            .module
            .declare_function(&name, Linkage::Local, &signature)?;
        self.module
            .define_function(function_id, &mut self.context)?;
        self.module.clear_context(&mut self.context);
        self.module.finalize_definitions()?;
        let code = self.module.get_finalized_function(function_id);
        // SAFETY: The emitted function uses the JitBlockFn ABI and parameter types.
        let function = unsafe { transmute::<*const u8, JitBlockFn>(code) };
        Ok(Some(CompiledBlock {
            function,
            instruction_count,
        }))
    }
}

struct LoweringState {
    registers: Value,
    ram: Value,
    framebuffer: Value,
    values: [Option<Value>; REGISTER_COUNT],
    dirty: [bool; REGISTER_COUNT],
}

impl LoweringState {
    fn new(registers: Value, ram: Value, framebuffer: Value) -> Self {
        Self {
            registers,
            ram,
            framebuffer,
            values: [None; REGISTER_COUNT],
            dirty: [false; REGISTER_COUNT],
        }
    }
}

fn lower_block(
    builder: &mut FunctionBuilder<'_>,
    state: &mut LoweringState,
    start: u32,
    instructions: &[u32],
) -> usize {
    let mut index = 0usize;
    while index < instructions.len() {
        let instruction = instructions[index];
        if is_control_flow(instruction) {
            if index + 1 >= instructions.len()
                || !is_supported_delay_instruction(instructions[index + 1])
            {
                break;
            }
            let branch_pc = start.wrapping_add((index as u32).wrapping_mul(4));
            let Some(target) = lower_branch(builder, state, instruction, branch_pc) else {
                break;
            };
            if !lower_register_instruction(builder, state, instructions[index + 1]) {
                break;
            }
            index += 2;
            emit_exit(builder, state, target, index as u64);
            return index;
        }

        if is_memory_instruction(instruction) {
            let pc = start.wrapping_add((index as u32).wrapping_mul(4));
            if !lower_memory_instruction(builder, state, instruction, pc, index as u64) {
                break;
            }
        } else if !lower_register_instruction(builder, state, instruction) {
            break;
        }
        index += 1;
    }

    if index != 0 {
        let pc = iconst_u32(builder, start.wrapping_add((index as u32).wrapping_mul(4)));
        emit_exit(builder, state, pc, index as u64);
    }
    index
}

fn is_control_flow(instruction: u32) -> bool {
    let opcode = instruction >> 26;
    matches!(opcode, 0x01..=0x07) || (opcode == 0 && matches!(instruction & 0x3f, 0x08 | 0x09))
}

fn is_supported_delay_instruction(instruction: u32) -> bool {
    !is_control_flow(instruction)
        && !is_memory_instruction(instruction)
        && is_supported_register_instruction(instruction)
}

fn is_supported_register_instruction(instruction: u32) -> bool {
    if instruction == 0 {
        return true;
    }
    let opcode = instruction >> 26;
    match opcode {
        0 => matches!(
            instruction & 0x3f,
            0x00 | 0x02
                | 0x03
                | 0x04
                | 0x06
                | 0x07
                | 0x0a
                | 0x0b
                | 0x0f
                | 0x10
                | 0x11
                | 0x12
                | 0x13
                | 0x18
                | 0x19
                | 0x1a
                | 0x1b
                | 0x20..=0x27 | 0x2a | 0x2b
        ),
        0x08..=0x0f | 0x33 => true,
        0x1c => matches!(
            instruction & 0x3f,
            0x00 | 0x01 | 0x02 | 0x04 | 0x05 | 0x20 | 0x21
        ),
        _ => false,
    }
}

fn is_memory_instruction(instruction: u32) -> bool {
    matches!(
        instruction >> 26,
        0x20 | 0x21
            | 0x22
            | 0x23
            | 0x24
            | 0x25
            | 0x26
            | 0x28
            | 0x29
            | 0x2a
            | 0x2b
            | 0x2e
            | 0x30
            | 0x38
    )
}

fn lower_register_instruction(
    builder: &mut FunctionBuilder<'_>,
    state: &mut LoweringState,
    instruction: u32,
) -> bool {
    if instruction == 0 {
        return true;
    }
    let opcode = instruction >> 26;
    let rs = ((instruction >> 21) & 0x1f) as usize;
    let rt = ((instruction >> 16) & 0x1f) as usize;
    let rd = ((instruction >> 11) & 0x1f) as usize;
    let shamt = (instruction >> 6) & 0x1f;

    let result = match opcode {
        0 => match instruction & 0x3f {
            0x00 => {
                let value = read_register(builder, state, rt);
                Some((rd, builder.ins().ishl_imm(value, i64::from(shamt))))
            }
            0x02 => {
                let value = read_register(builder, state, rt);
                Some((rd, builder.ins().ushr_imm(value, i64::from(shamt))))
            }
            0x03 => {
                let value = read_register(builder, state, rt);
                Some((rd, builder.ins().sshr_imm(value, i64::from(shamt))))
            }
            0x04 | 0x06 | 0x07 => {
                let value = read_register(builder, state, rt);
                let shift = read_register(builder, state, rs);
                let shift = builder.ins().band_imm(shift, 0x1f);
                let shifted = match instruction & 0x3f {
                    0x04 => builder.ins().ishl(value, shift),
                    0x06 => builder.ins().ushr(value, shift),
                    _ => builder.ins().sshr(value, shift),
                };
                Some((rd, shifted))
            }
            0x0a | 0x0b => {
                let source = read_register(builder, state, rs);
                let condition_value = read_register(builder, state, rt);
                let current = read_register(builder, state, rd);
                let condition = builder.ins().icmp_imm(
                    if instruction & 0x3f == 0x0a {
                        IntCC::Equal
                    } else {
                        IntCC::NotEqual
                    },
                    condition_value,
                    0,
                );
                Some((rd, builder.ins().select(condition, source, current)))
            }
            0x0f => None,
            0x10 => Some((rd, read_special_register(builder, state, HI_INDEX))),
            0x11 => {
                let value = read_register(builder, state, rs);
                write_special_register(state, HI_INDEX, value);
                None
            }
            0x12 => Some((rd, read_special_register(builder, state, LO_INDEX))),
            0x13 => {
                let value = read_register(builder, state, rs);
                write_special_register(state, LO_INDEX, value);
                None
            }
            0x18 | 0x19 => {
                let left = read_register(builder, state, rs);
                let right = read_register(builder, state, rt);
                let (left, right) = if instruction & 0x3f == 0x18 {
                    (
                        builder.ins().sextend(types::I64, left),
                        builder.ins().sextend(types::I64, right),
                    )
                } else {
                    (
                        builder.ins().uextend(types::I64, left),
                        builder.ins().uextend(types::I64, right),
                    )
                };
                let product = builder.ins().imul(left, right);
                write_i64_pair(builder, state, product);
                None
            }
            0x1a | 0x1b => {
                let numerator = read_register(builder, state, rs);
                let denominator = read_register(builder, state, rt);
                let zero = builder.ins().icmp_imm(IntCC::Equal, denominator, 0);
                let invalid = if instruction & 0x3f == 0x1a {
                    let minimum = iconst_u32(builder, i32::MIN as u32);
                    let negative_one = iconst_u32(builder, u32::MAX);
                    let is_minimum = builder.ins().icmp(IntCC::Equal, numerator, minimum);
                    let is_negative_one =
                        builder.ins().icmp(IntCC::Equal, denominator, negative_one);
                    let overflow = builder.ins().band(is_minimum, is_negative_one);
                    builder.ins().bor(zero, overflow)
                } else {
                    zero
                };
                let one = iconst_u32(builder, 1);
                let safe_denominator = builder.ins().select(invalid, one, denominator);
                let (quotient, remainder) = if instruction & 0x3f == 0x1a {
                    (
                        builder.ins().sdiv(numerator, safe_denominator),
                        builder.ins().srem(numerator, safe_denominator),
                    )
                } else {
                    (
                        builder.ins().udiv(numerator, safe_denominator),
                        builder.ins().urem(numerator, safe_denominator),
                    )
                };
                let old_lo = read_special_register(builder, state, LO_INDEX);
                let old_hi = read_special_register(builder, state, HI_INDEX);
                let lo = builder.ins().select(invalid, old_lo, quotient);
                let hi = builder.ins().select(invalid, old_hi, remainder);
                write_special_register(state, LO_INDEX, lo);
                write_special_register(state, HI_INDEX, hi);
                None
            }
            0x20 | 0x21 => {
                let left = read_register(builder, state, rs);
                let right = read_register(builder, state, rt);
                Some((rd, builder.ins().iadd(left, right)))
            }
            0x22 | 0x23 => {
                let left = read_register(builder, state, rs);
                let right = read_register(builder, state, rt);
                Some((rd, builder.ins().isub(left, right)))
            }
            0x24 => {
                let left = read_register(builder, state, rs);
                let right = read_register(builder, state, rt);
                Some((rd, builder.ins().band(left, right)))
            }
            0x25 => {
                let left = read_register(builder, state, rs);
                let right = read_register(builder, state, rt);
                Some((rd, builder.ins().bor(left, right)))
            }
            0x26 => {
                let left = read_register(builder, state, rs);
                let right = read_register(builder, state, rt);
                Some((rd, builder.ins().bxor(left, right)))
            }
            0x27 => {
                let left = read_register(builder, state, rs);
                let right = read_register(builder, state, rt);
                let value = builder.ins().bor(left, right);
                Some((rd, builder.ins().bnot(value)))
            }
            0x2a | 0x2b => {
                let left = read_register(builder, state, rs);
                let right = read_register(builder, state, rt);
                let condition = builder.ins().icmp(
                    if instruction & 0x3f == 0x2a {
                        IntCC::SignedLessThan
                    } else {
                        IntCC::UnsignedLessThan
                    },
                    left,
                    right,
                );
                Some((rd, builder.ins().uextend(types::I32, condition)))
            }
            _ => return false,
        },
        0x08 | 0x09 => {
            let source = read_register(builder, state, rs);
            let immediate = iconst_u32(builder, instruction as i16 as i32 as u32);
            Some((rt, builder.ins().iadd(source, immediate)))
        }
        0x0a | 0x0b => {
            let source = read_register(builder, state, rs);
            let immediate = iconst_u32(builder, instruction as i16 as i32 as u32);
            let condition = builder.ins().icmp(
                if opcode == 0x0a {
                    IntCC::SignedLessThan
                } else {
                    IntCC::UnsignedLessThan
                },
                source,
                immediate,
            );
            Some((rt, builder.ins().uextend(types::I32, condition)))
        }
        0x0c => {
            let source = read_register(builder, state, rs);
            Some((
                rt,
                builder
                    .ins()
                    .band_imm(source, i64::from(instruction as u16)),
            ))
        }
        0x0d => {
            let source = read_register(builder, state, rs);
            Some((
                rt,
                builder.ins().bor_imm(source, i64::from(instruction as u16)),
            ))
        }
        0x0e => {
            let source = read_register(builder, state, rs);
            Some((
                rt,
                builder
                    .ins()
                    .bxor_imm(source, i64::from(instruction as u16)),
            ))
        }
        0x0f => Some((rt, iconst_u32(builder, u32::from(instruction as u16) << 16))),
        0x1c if matches!(
            instruction & 0x3f,
            0x00 | 0x01 | 0x02 | 0x04 | 0x05 | 0x20 | 0x21
        ) =>
        {
            lower_special2(builder, state, instruction, rs, rt, rd);
            None
        }
        0x33 => None,
        _ => return false,
    };

    if let Some((register, value)) = result {
        write_register(state, register, value);
    }
    true
}

fn lower_special2(
    builder: &mut FunctionBuilder<'_>,
    state: &mut LoweringState,
    instruction: u32,
    rs: usize,
    rt: usize,
    rd: usize,
) {
    match instruction & 0x3f {
        0x00 | 0x01 | 0x04 | 0x05 => {
            let left = read_register(builder, state, rs);
            let right = read_register(builder, state, rt);
            let left = builder.ins().uextend(types::I64, left);
            let right = builder.ins().uextend(types::I64, right);
            let product = builder.ins().imul(left, right);
            let hi = read_special_register(builder, state, HI_INDEX);
            let lo = read_special_register(builder, state, LO_INDEX);
            let hi = builder.ins().uextend(types::I64, hi);
            let hi = builder.ins().ishl_imm(hi, 32);
            let lo = builder.ins().uextend(types::I64, lo);
            let accumulator = builder.ins().bor(hi, lo);
            let result = if matches!(instruction & 0x3f, 0x00 | 0x01) {
                builder.ins().iadd(accumulator, product)
            } else {
                builder.ins().isub(accumulator, product)
            };
            write_i64_pair(builder, state, result);
        }
        0x02 => {
            let left = read_register(builder, state, rs);
            let right = read_register(builder, state, rt);
            let result = builder.ins().imul(left, right);
            write_register(state, rd, result);
        }
        0x20 | 0x21 => {
            let value = read_register(builder, state, rs);
            let value = if instruction & 0x3f == 0x21 {
                builder.ins().bnot(value)
            } else {
                value
            };
            let result = builder.ins().clz(value);
            write_register(state, rd, result);
        }
        _ => unreachable!("instruction support checked before lowering"),
    }
}

fn lower_branch(
    builder: &mut FunctionBuilder<'_>,
    state: &mut LoweringState,
    instruction: u32,
    branch_pc: u32,
) -> Option<Value> {
    let opcode = instruction >> 26;
    let rs = ((instruction >> 21) & 0x1f) as usize;
    let rt = ((instruction >> 16) & 0x1f) as usize;
    let fallthrough = branch_pc.wrapping_add(8);
    let branch_target = branch_pc
        .wrapping_add(4)
        .wrapping_add(((instruction as i16 as i32) << 2) as u32);

    match opcode {
        0 => match instruction & 0x3f {
            0x08 => Some(read_register(builder, state, rs)),
            0x09 => {
                let target = read_register(builder, state, rs);
                write_register(
                    state,
                    ((instruction >> 11) & 0x1f) as usize,
                    iconst_u32(builder, fallthrough),
                );
                Some(target)
            }
            _ => None,
        },
        0x01 => {
            let kind = rt;
            if matches!(kind, 0x10 | 0x11) {
                let link = iconst_u32(builder, fallthrough);
                write_register(state, 31, link);
            }
            let source = read_register(builder, state, rs);
            let condition = match kind {
                0x00 | 0x10 => builder.ins().icmp_imm(IntCC::SignedLessThan, source, 0),
                0x01 | 0x11 => builder
                    .ins()
                    .icmp_imm(IntCC::SignedGreaterThanOrEqual, source, 0),
                _ => return None,
            };
            Some(select_branch_target(
                builder,
                condition,
                branch_target,
                fallthrough,
            ))
        }
        0x02 | 0x03 => {
            if opcode == 0x03 {
                let link = iconst_u32(builder, fallthrough);
                write_register(state, 31, link);
            }
            Some(iconst_u32(
                builder,
                (branch_pc & 0xf000_0000) | ((instruction & 0x03ff_ffff) << 2),
            ))
        }
        0x04 | 0x05 => {
            let left = read_register(builder, state, rs);
            let right = read_register(builder, state, rt);
            let condition = builder.ins().icmp(
                if opcode == 0x04 {
                    IntCC::Equal
                } else {
                    IntCC::NotEqual
                },
                left,
                right,
            );
            Some(select_branch_target(
                builder,
                condition,
                branch_target,
                fallthrough,
            ))
        }
        0x06 | 0x07 => {
            let source = read_register(builder, state, rs);
            let condition = builder.ins().icmp_imm(
                if opcode == 0x06 {
                    IntCC::SignedLessThanOrEqual
                } else {
                    IntCC::SignedGreaterThan
                },
                source,
                0,
            );
            Some(select_branch_target(
                builder,
                condition,
                branch_target,
                fallthrough,
            ))
        }
        _ => None,
    }
}

fn select_branch_target(
    builder: &mut FunctionBuilder<'_>,
    condition: Value,
    taken: u32,
    fallthrough: u32,
) -> Value {
    let taken = iconst_u32(builder, taken);
    let fallthrough = iconst_u32(builder, fallthrough);
    builder.ins().select(condition, taken, fallthrough)
}

fn lower_memory_instruction(
    builder: &mut FunctionBuilder<'_>,
    state: &mut LoweringState,
    instruction: u32,
    pc: u32,
    completed: u64,
) -> bool {
    let opcode = instruction >> 26;
    let rs = ((instruction >> 21) & 0x1f) as usize;
    let rt = ((instruction >> 16) & 0x1f) as usize;
    let width = match opcode {
        0x20 | 0x24 | 0x28 => 1u32,
        0x21 | 0x25 | 0x29 => 2,
        0x22 | 0x23 | 0x26 | 0x2a | 0x2b | 0x2e | 0x30 | 0x38 => 4,
        _ => return false,
    };
    let base = read_register(builder, state, rs);
    let offset = iconst_u32(builder, instruction as i16 as i32 as u32);
    let address = builder.ins().iadd(base, offset);
    let unaligned = matches!(opcode, 0x22 | 0x26 | 0x2a | 0x2e);
    let access_address = if unaligned {
        builder.ins().band_imm(address, i64::from(!3u32))
    } else {
        address
    };
    let physical = translate_address(builder, access_address);
    let ram_in_bounds = builder.ins().icmp_imm(
        IntCC::UnsignedLessThanOrEqual,
        physical,
        i64::from(crate::memory::RAM_SIZE - width),
    );
    let physical_pointer = builder.ins().uextend(types::I64, physical);
    let ram_address = builder.ins().iadd(state.ram, physical_pointer);
    let fast = builder.create_block();
    builder.append_block_param(fast, types::I64);
    let check_framebuffer = builder.create_block();
    let slow = builder.create_block();
    let ram_args = [ram_address.into()];
    builder
        .ins()
        .brif(ram_in_bounds, fast, &ram_args, check_framebuffer, &[]);

    builder.switch_to_block(check_framebuffer);
    builder.seal_block(check_framebuffer);
    let mut framebuffer_address = state.framebuffer;
    let mut mapped = None;
    for base in crate::memory::LCD_FRAMEBUFFER_ALIASES {
        let alias_offset = builder.ins().iadd_imm(access_address, -i64::from(base));
        let alias_in_bounds = builder.ins().icmp_imm(
            IntCC::UnsignedLessThanOrEqual,
            alias_offset,
            (crate::video::FRAMEBUFFER_MAP_SIZE as u32 - width) as i64,
        );
        let alias_pointer = builder.ins().uextend(types::I64, alias_offset);
        let alias_address = builder.ins().iadd(state.framebuffer, alias_pointer);
        framebuffer_address =
            builder
                .ins()
                .select(alias_in_bounds, alias_address, framebuffer_address);
        mapped = Some(match mapped {
            Some(previous) => builder.ins().bor(previous, alias_in_bounds),
            None => alias_in_bounds,
        });
    }
    let framebuffer_args = [framebuffer_address.into()];
    builder.ins().brif(
        mapped.expect("framebuffer has at least one alias"),
        fast,
        &framebuffer_args,
        slow,
        &[],
    );

    builder.switch_to_block(slow);
    builder.seal_block(slow);
    let bailout_pc = iconst_u32(builder, pc);
    emit_exit(builder, state, bailout_pc, completed);

    builder.switch_to_block(fast);
    builder.seal_block(fast);
    let host_address = builder.block_params(fast)[0];
    let flags = MemFlags::new();

    match opcode {
        0x20 | 0x21 | 0x23 | 0x24 | 0x25 | 0x30 => {
            let load_type = match width {
                1 => types::I8,
                2 => types::I16,
                _ => types::I32,
            };
            let value = builder.ins().load(load_type, flags, host_address, 0);
            let value = match opcode {
                0x20 | 0x21 => builder.ins().sextend(types::I32, value),
                0x24 | 0x25 => builder.ins().uextend(types::I32, value),
                _ => value,
            };
            write_register(state, rt, value);
        }
        0x22 | 0x26 => {
            let memory = builder.ins().load(types::I32, flags, host_address, 0);
            let current = read_register(builder, state, rt);
            let byte = builder.ins().band_imm(address, 3);
            let cases = if opcode == 0x22 {
                [
                    merge_shift_left(builder, memory, 24, current, 0x00ff_ffff),
                    merge_shift_left(builder, memory, 16, current, 0x0000_ffff),
                    merge_shift_left(builder, memory, 8, current, 0x0000_00ff),
                    memory,
                ]
            } else {
                [
                    memory,
                    merge_shift_right(builder, memory, 8, current, 0xff00_0000),
                    merge_shift_right(builder, memory, 16, current, 0xffff_0000),
                    merge_shift_right(builder, memory, 24, current, 0xffff_ff00),
                ]
            };
            let value = select_byte_case(builder, byte, cases);
            write_register(state, rt, value);
        }
        0x28 | 0x29 | 0x2b | 0x38 => {
            let value = read_register(builder, state, rt);
            let value = match width {
                1 => builder.ins().ireduce(types::I8, value),
                2 => builder.ins().ireduce(types::I16, value),
                _ => value,
            };
            builder.ins().store(flags, value, host_address, 0);
            if opcode == 0x38 {
                let success = iconst_u32(builder, 1);
                write_register(state, rt, success);
            }
        }
        0x2a | 0x2e => {
            let memory = builder.ins().load(types::I32, flags, host_address, 0);
            let value = read_register(builder, state, rt);
            let byte = builder.ins().band_imm(address, 3);
            let cases = if opcode == 0x2a {
                [
                    merge_store_right(builder, memory, 0xffff_ff00, value, 24),
                    merge_store_right(builder, memory, 0xffff_0000, value, 16),
                    merge_store_right(builder, memory, 0xff00_0000, value, 8),
                    value,
                ]
            } else {
                [
                    value,
                    merge_store_left(builder, memory, 0x0000_00ff, value, 8),
                    merge_store_left(builder, memory, 0x0000_ffff, value, 16),
                    merge_store_left(builder, memory, 0x00ff_ffff, value, 24),
                ]
            };
            let value = select_byte_case(builder, byte, cases);
            builder.ins().store(flags, value, host_address, 0);
        }
        _ => return false,
    }
    true
}

fn merge_shift_left(
    builder: &mut FunctionBuilder<'_>,
    memory: Value,
    shift: i64,
    current: Value,
    mask: u32,
) -> Value {
    let memory = builder.ins().ishl_imm(memory, shift);
    let current = builder.ins().band_imm(current, i64::from(mask));
    builder.ins().bor(memory, current)
}

fn merge_shift_right(
    builder: &mut FunctionBuilder<'_>,
    memory: Value,
    shift: i64,
    current: Value,
    mask: u32,
) -> Value {
    let memory = builder.ins().ushr_imm(memory, shift);
    let current = builder.ins().band_imm(current, i64::from(mask));
    builder.ins().bor(memory, current)
}

fn merge_store_right(
    builder: &mut FunctionBuilder<'_>,
    memory: Value,
    mask: u32,
    value: Value,
    shift: i64,
) -> Value {
    let memory = builder.ins().band_imm(memory, i64::from(mask));
    let value = builder.ins().ushr_imm(value, shift);
    builder.ins().bor(memory, value)
}

fn merge_store_left(
    builder: &mut FunctionBuilder<'_>,
    memory: Value,
    mask: u32,
    value: Value,
    shift: i64,
) -> Value {
    let memory = builder.ins().band_imm(memory, i64::from(mask));
    let value = builder.ins().ishl_imm(value, shift);
    builder.ins().bor(memory, value)
}

fn select_byte_case(builder: &mut FunctionBuilder<'_>, byte: Value, cases: [Value; 4]) -> Value {
    let mut result = cases[3];
    for index in (0..3).rev() {
        let matches = builder.ins().icmp_imm(IntCC::Equal, byte, index as i64);
        result = builder.ins().select(matches, cases[index], result);
    }
    result
}

fn translate_address(builder: &mut FunctionBuilder<'_>, address: Value) -> Value {
    let segment = builder.ins().band_imm(address, i64::from(0xe000_0000_u32));
    let kseg0 = iconst_u32(builder, 0x8000_0000);
    let kseg1 = iconst_u32(builder, 0xa000_0000);
    let is_kseg0 = builder.ins().icmp(IntCC::Equal, segment, kseg0);
    let is_kseg1 = builder.ins().icmp(IntCC::Equal, segment, kseg1);
    let masked = builder.ins().band_imm(address, i64::from(KSEG_MASK));
    let physical = builder.ins().select(is_kseg0, masked, address);
    builder.ins().select(is_kseg1, masked, physical)
}

fn read_register(
    builder: &mut FunctionBuilder<'_>,
    state: &mut LoweringState,
    register: usize,
) -> Value {
    if register == 0 {
        return iconst_u32(builder, 0);
    }
    read_special_register(builder, state, register)
}

fn write_register(state: &mut LoweringState, register: usize, value: Value) {
    if register != 0 {
        write_special_register(state, register, value);
    }
}

fn read_special_register(
    builder: &mut FunctionBuilder<'_>,
    state: &mut LoweringState,
    register: usize,
) -> Value {
    if let Some(value) = state.values[register] {
        return value;
    }
    let value = builder.ins().load(
        types::I32,
        MemFlags::new(),
        state.registers,
        register_offset(register),
    );
    state.values[register] = Some(value);
    value
}

fn write_special_register(state: &mut LoweringState, register: usize, value: Value) {
    state.values[register] = Some(value);
    state.dirty[register] = true;
}

fn write_i64_pair(builder: &mut FunctionBuilder<'_>, state: &mut LoweringState, value: Value) {
    let lo = builder.ins().ireduce(types::I32, value);
    let hi = builder.ins().ushr_imm(value, 32);
    let hi = builder.ins().ireduce(types::I32, hi);
    write_special_register(state, LO_INDEX, lo);
    write_special_register(state, HI_INDEX, hi);
}

fn register_offset(register: usize) -> i32 {
    let offset = match register {
        0..=31 => offset_of!(Registers, gpr) + register * std::mem::size_of::<u32>(),
        HI_INDEX => offset_of!(Registers, hi),
        LO_INDEX => offset_of!(Registers, lo),
        _ => unreachable!("invalid register index"),
    };
    offset as i32
}

fn emit_exit(builder: &mut FunctionBuilder<'_>, state: &LoweringState, pc: Value, completed: u64) {
    for register in 1..REGISTER_COUNT {
        if state.dirty[register] {
            builder.ins().store(
                MemFlags::new(),
                state.values[register].expect("dirty registers always have a value"),
                state.registers,
                register_offset(register),
            );
        }
    }
    builder.ins().store(
        MemFlags::new(),
        pc,
        state.registers,
        offset_of!(Registers, pc) as i32,
    );
    let completed = builder.ins().iconst(types::I64, completed as i64);
    builder.ins().return_(&[completed]);
}

fn iconst_u32(builder: &mut FunctionBuilder<'_>, value: u32) -> Value {
    builder.ins().iconst(types::I32, i64::from(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::Cpu;
    use crate::memory::Memory;

    #[test]
    fn register_layout_matches_jit_offsets() {
        assert_eq!(register_offset(0), offset_of!(Registers, gpr) as i32);
        assert_eq!(register_offset(HI_INDEX), offset_of!(Registers, hi) as i32);
        assert_eq!(register_offset(LO_INDEX), offset_of!(Registers, lo) as i32);
    }

    #[test]
    fn compilation_budget_preserves_colliding_hot_blocks() {
        let instructions = [
            (0x09 << 26) | (8 << 16) | 1,
            (0x09 << 26) | (8 << 21) | (8 << 16) | 1,
            (8 << 21) | (8 << 16) | (9 << 11) | 0x21,
            (9 << 21) | (8 << 16) | (10 << 11) | 0x26,
        ];
        let first_start = 0x1000;
        let second_start = first_start + 0x4000;
        let mut engine = JitEngine::new();
        let mut registers = Registers::new(first_start);
        let mut memory = Memory::new();
        let ram = memory.jit_ram_ptr();
        let framebuffer = memory.jit_framebuffer_ptr();

        for _ in 0..HOT_BLOCK_THRESHOLD {
            let _ = engine.execute(
                first_start,
                &instructions,
                instructions.len(),
                &mut registers,
                ram,
                framebuffer,
            );
        }
        assert!(engine.entries[&first_start].block.is_some());

        registers.pc = second_start;
        for _ in 0..HOT_BLOCK_THRESHOLD {
            assert!(engine
                .execute(
                    second_start,
                    &instructions,
                    instructions.len(),
                    &mut registers,
                    ram,
                    framebuffer,
                )
                .is_none());
        }
        assert!(engine.entries[&second_start].block.is_none());

        for _ in 0..=COMPILE_COOLDOWN_FRAMES {
            engine.begin_frame();
        }
        assert!(engine
            .execute(
                second_start,
                &instructions,
                instructions.len(),
                &mut registers,
                ram,
                framebuffer,
            )
            .is_some());
        assert!(engine.entries[&first_start].block.is_some());
        assert!(engine.entries[&second_start].block.is_some());
    }

    #[test]
    fn clearing_cache_recreates_compiler_after_native_code_generation() {
        let instructions = [
            (0x09 << 26) | (8 << 16) | 1,
            (0x09 << 26) | (8 << 21) | (8 << 16) | 1,
            (8 << 21) | (8 << 16) | (9 << 11) | 0x21,
            (9 << 21) | (8 << 16) | (10 << 11) | 0x26,
        ];
        let start = 0x1000;
        let mut engine = JitEngine::new();
        let mut registers = Registers::new(start);
        let mut memory = Memory::new();
        let ram = memory.jit_ram_ptr();
        let framebuffer = memory.jit_framebuffer_ptr();

        for _ in 0..HOT_BLOCK_THRESHOLD {
            let _ = engine.execute(
                start,
                &instructions,
                instructions.len(),
                &mut registers,
                ram,
                framebuffer,
            );
        }
        assert_eq!(engine.compiled_block_count, 1);
        assert_eq!(engine.compiler.as_ref().unwrap().next_function_id, 1);

        engine.clear();

        assert!(engine.entries.is_empty());
        assert!(engine
            .fast_entries
            .iter()
            .all(|entry| entry.block.is_none()));
        assert_eq!(engine.compiled_block_count, 0);
        assert_eq!(engine.compiler.as_ref().unwrap().next_function_id, 0);
    }

    #[test]
    fn compiled_block_matches_interpreter_state() {
        let start = 0x1000;
        let instructions = [
            (0x09 << 26) | (8 << 16) | 5,                  // addiu t0, zero, 5
            (0x09 << 26) | (8 << 21) | (9 << 16) | 0xfffe, // addiu t1, t0, -2
            (9 << 16) | (10 << 11) | (4 << 6),             // sll t2, t1, 4
            (0x2b << 26) | (10 << 16) | 0x0200,            // sw t2, 0x200(zero)
            (0x23 << 26) | (11 << 16) | 0x0200,            // lw t3, 0x200(zero)
            (0x04 << 26) | (11 << 21) | (10 << 16) | 2,    // beq t3, t2, +2
            (0x09 << 26) | (12 << 16) | 7,                 // addiu t4, zero, 7
        ];

        let mut compiler = Compiler::new().unwrap();
        let block = compiler.compile(start, &instructions).unwrap().unwrap();
        assert_eq!(block.instruction_count, instructions.len());

        let mut jit_registers = Registers::new(start);
        let mut jit_memory = Memory::new();
        let ram = jit_memory.jit_ram_ptr();
        let framebuffer = jit_memory.jit_framebuffer_ptr();
        let completed = unsafe { (block.function)(&mut jit_registers, ram, framebuffer) };
        assert_eq!(completed, instructions.len() as u64);

        let mut interpreter = Cpu::new(start);
        let mut interpreter_memory = Memory::new();
        interpreter.start();
        for &instruction in &instructions {
            assert!(interpreter
                .step_fetched_unaccounted(instruction, &mut interpreter_memory)
                .unwrap());
        }

        assert_eq!(jit_registers.gpr, interpreter.regs.gpr);
        assert_eq!(jit_registers.pc, interpreter.regs.pc);
        assert_eq!(jit_registers.hi, interpreter.regs.hi);
        assert_eq!(jit_registers.lo, interpreter.regs.lo);
        assert_eq!(jit_memory.read_u32(0x200).unwrap(), 48);
        assert_eq!(
            jit_memory.read_u32(0x200).unwrap(),
            interpreter_memory.read_u32(0x200).unwrap()
        );
    }

    #[test]
    fn extended_integer_and_unaligned_memory_match_interpreter() {
        let start = 0x1800;
        let instructions = [
            (8 << 21) | (9 << 16) | 0x1a,             // div t0, t1
            (10 << 11) | 0x12,                        // mflo t2
            (11 << 11) | 0x10,                        // mfhi t3
            (0x22 << 26) | (4 << 21) | (5 << 16) | 3, // lwl a1, 3(a0)
            (0x26 << 26) | (4 << 21) | (5 << 16),     // lwr a1, 0(a0)
            (0x2a << 26) | (6 << 21) | (7 << 16) | 3, // swl a3, 3(a2)
            (0x2e << 26) | (6 << 21) | (7 << 16),     // swr a3, 0(a2)
            (8 << 21) | (12 << 16) | 0x1a,            // div t0, t4
            (13 << 11) | 0x12,                        // mflo t5
            (14 << 11) | 0x10,                        // mfhi t6
        ];
        let mut compiler = Compiler::new().unwrap();
        let block = compiler.compile(start, &instructions).unwrap().unwrap();

        let mut jit_registers = Registers::new(start);
        jit_registers.write(4, 0x101);
        jit_registers.write(5, 0xdead_beef);
        jit_registers.write(6, 0x201);
        jit_registers.write(7, 0x4433_2211);
        jit_registers.write(8, (-7i32) as u32);
        jit_registers.write(9, 3);
        jit_registers.write(12, 0);
        let mut interpreter = Cpu::new(start);
        interpreter.regs = jit_registers.clone();
        interpreter.start();

        let mut jit_memory = Memory::new();
        let mut interpreter_memory = Memory::new();
        for memory in [&mut jit_memory, &mut interpreter_memory] {
            memory
                .load_data(0x100, &[0x11, 0x22, 0x33, 0x44, 0x55])
                .unwrap();
            memory.load_data(0x200, &[0xaa; 8]).unwrap();
        }
        let ram = jit_memory.jit_ram_ptr();
        let framebuffer = jit_memory.jit_framebuffer_ptr();
        let completed = unsafe { (block.function)(&mut jit_registers, ram, framebuffer) };
        for &instruction in &instructions {
            interpreter
                .step_fetched_unaccounted(instruction, &mut interpreter_memory)
                .unwrap();
        }

        assert_eq!(completed, instructions.len() as u64);
        assert_eq!(jit_registers.gpr, interpreter.regs.gpr);
        assert_eq!(jit_registers.pc, interpreter.regs.pc);
        assert_eq!(jit_registers.hi, interpreter.regs.hi);
        assert_eq!(jit_registers.lo, interpreter.regs.lo);
        assert_eq!(
            &jit_memory.system_ram()[0x200..0x208],
            &interpreter_memory.system_ram()[0x200..0x208]
        );
    }

    #[test]
    fn special_memory_mapping_bails_out_before_side_effects() {
        let start = 0x2000;
        let instructions = [
            (0x2b << 26) | (9 << 21) | (8 << 16),
            (0x09 << 26) | (10 << 16) | 1,
        ];
        let mut compiler = Compiler::new().unwrap();
        let block = compiler.compile(start, &instructions).unwrap().unwrap();
        let mut registers = Registers::new(start);
        registers.write(8, 0x1234_5678);
        registers.write(9, 0x1308_0000);
        let mut memory = Memory::new();
        let ram = memory.jit_ram_ptr();
        let framebuffer = memory.jit_framebuffer_ptr();

        let completed = unsafe { (block.function)(&mut registers, ram, framebuffer) };

        assert_eq!(completed, 0);
        assert_eq!(registers.pc, start);
        assert_eq!(registers.read(10), 0);
        assert_eq!(memory.read_u32(0x1308_0000).unwrap(), 0);
    }

    #[test]
    fn framebuffer_alias_uses_native_memory_path() {
        let start = 0x3000;
        let instructions = [(0x2b << 26) | (9 << 21) | (8 << 16)];
        let mut compiler = Compiler::new().unwrap();
        let block = compiler.compile(start, &instructions).unwrap().unwrap();
        let mut registers = Registers::new(start);
        registers.write(8, 0x1234_5678);
        registers.write(9, crate::video::VM_LCD_FB_ADDRESS);
        let mut memory = Memory::new();
        let ram = memory.jit_ram_ptr();
        let framebuffer = memory.jit_framebuffer_ptr();

        let completed = unsafe { (block.function)(&mut registers, ram, framebuffer) };

        assert_eq!(completed, 1);
        assert_eq!(registers.pc, start + 4);
        assert_eq!(
            memory.read_u32(crate::video::VM_LCD_FB_ADDRESS).unwrap(),
            0x1234_5678
        );
    }
}
