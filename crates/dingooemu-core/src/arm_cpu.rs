use crate::common::execution::UnknownInstructionPolicy;
use crate::error::{Result, SimulatorError};

const N: u32 = 1 << 31;
const Z: u32 = 1 << 30;
const C: u32 = 1 << 29;
const V: u32 = 1 << 28;
const Q: u32 = 1 << 27;
const T: u32 = 1 << 5;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ArmExecutionState {
    #[default]
    Arm,
    Thumb,
}

pub trait ArmBus {
    fn read8(&mut self, address: u32) -> Result<u8>;
    fn read16(&mut self, address: u32) -> Result<u16>;
    fn read32(&mut self, address: u32) -> Result<u32>;
    fn write8(&mut self, address: u32, value: u8) -> Result<()>;
    fn write16(&mut self, address: u32, value: u16) -> Result<()>;
    fn write32(&mut self, address: u32, value: u32) -> Result<()>;
    fn svc(&mut self, cpu: &mut ArmCpu, immediate: u32) -> Result<()>;

    fn fetch16(&mut self, address: u32) -> Result<u16> {
        self.read16(address)
    }

    fn fetch32(&mut self, address: u32) -> Result<u32> {
        self.read32(address)
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ArmCpu {
    pub r: [u32; 16],
    pub cpsr: u32,
    pub instruction_count: u64,
    state: ArmExecutionState,
    running: bool,
    unknown_instruction_policy: UnknownInstructionPolicy,
}

impl ArmCpu {
    pub fn new(entry: u32, stack: u32, link_register: u32) -> Self {
        let state = if entry & 1 != 0 {
            ArmExecutionState::Thumb
        } else {
            ArmExecutionState::Arm
        };
        let mut r = [0; 16];
        r[13] = stack;
        r[14] = link_register;
        r[15] = match state {
            ArmExecutionState::Arm => entry & !3,
            ArmExecutionState::Thumb => entry & !1,
        };
        Self {
            r,
            cpsr: if state == ArmExecutionState::Thumb {
                T
            } else {
                0
            },
            instruction_count: 0,
            state,
            running: false,
            unknown_instruction_policy: UnknownInstructionPolicy::default(),
        }
    }

    pub fn start(&mut self) {
        self.running = true;
    }

    pub fn stop(&mut self) {
        self.running = false;
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn execution_state(&self) -> ArmExecutionState {
        self.state
    }

    pub fn set_unknown_instruction_policy(&mut self, policy: UnknownInstructionPolicy) {
        self.unknown_instruction_policy = policy;
    }

    pub fn unknown_instruction_policy(&self) -> UnknownInstructionPolicy {
        self.unknown_instruction_policy
    }

    pub fn step<B: ArmBus>(&mut self, bus: &mut B) -> Result<()> {
        if !self.running {
            return Ok(());
        }
        let pc = self.r[15];
        let result = match self.state {
            ArmExecutionState::Arm => {
                let instruction = bus.fetch32(pc)?;
                self.r[15] = pc.wrapping_add(4);
                self.execute_arm(instruction, pc, bus)
            }
            ArmExecutionState::Thumb => {
                let instruction = bus.fetch16(pc)?;
                self.r[15] = pc.wrapping_add(2);
                self.execute_thumb(instruction, pc, bus)
            }
        };
        match result {
            Ok(()) => {}
            Err(SimulatorError::InvalidInstruction { pc, instr }) => {
                match self.unknown_instruction_policy {
                    UnknownInstructionPolicy::Stop => {
                        return Err(SimulatorError::InvalidInstruction { pc, instr });
                    }
                    UnknownInstructionPolicy::Skip => log::warn!(
                        "Skipping unimplemented ARM instruction {instr:#010x} at PC={pc:#010x}"
                    ),
                }
            }
            Err(error) => return Err(error),
        }
        self.instruction_count = self.instruction_count.wrapping_add(1);
        Ok(())
    }

    pub fn run<B: ArmBus>(&mut self, bus: &mut B, limit: u64) -> Result<u64> {
        let initial = self.instruction_count;
        while self.running && self.instruction_count.wrapping_sub(initial) < limit {
            self.step(bus)?;
        }
        Ok(self.instruction_count.wrapping_sub(initial))
    }

    fn execute_arm<B: ArmBus>(&mut self, instruction: u32, pc: u32, bus: &mut B) -> Result<()> {
        let condition = instruction >> 28;
        if condition != 0xf && !self.condition_passed(condition) {
            return Ok(());
        }
        if condition == 0xf && instruction & 0x0e00_0000 == 0x0a00_0000 {
            let offset = (((instruction & 0x00ff_ffff) << 8) as i32 >> 6) as u32;
            let target = pc
                .wrapping_add(8)
                .wrapping_add(offset)
                .wrapping_add((instruction >> 23) & 2);
            self.r[14] = pc.wrapping_add(4);
            self.state = ArmExecutionState::Thumb;
            self.cpsr |= T;
            self.r[15] = target & !1;
            return Ok(());
        }
        if instruction & 0x0fff_fff0 == 0x012f_ff10 {
            return self.branch_exchange(self.read_reg((instruction & 0xf) as usize, pc, false));
        }
        if instruction & 0x0fff_fff0 == 0x012f_ff30 {
            self.r[14] = pc.wrapping_add(4);
            return self.branch_exchange(self.read_reg((instruction & 0xf) as usize, pc, false));
        }
        if instruction & 0x0fff_0ff0 == 0x016f_0f10 {
            let rd = ((instruction >> 12) & 0xf) as usize;
            let rm = (instruction & 0xf) as usize;
            self.write_reg(rd, self.read_reg(rm, pc, false).leading_zeros());
            return Ok(());
        }
        match (instruction >> 25) & 7 {
            0 | 1 => self.execute_arm_data_or_misc(instruction, pc, bus),
            2 | 3 => self.execute_single_transfer(instruction, pc, bus),
            4 => self.execute_block_transfer(instruction, pc, bus),
            5 => self.execute_branch(instruction, pc),
            7 if instruction & (1 << 24) != 0 => bus.svc(self, instruction & 0x00ff_ffff),
            _ => Err(SimulatorError::InvalidInstruction {
                pc,
                instr: instruction,
            }),
        }
    }

    fn execute_arm_data_or_misc<B: ArmBus>(
        &mut self,
        instruction: u32,
        pc: u32,
        bus: &mut B,
    ) -> Result<()> {
        let signed_halfword_multiply = instruction & 0x0ff0_0090;
        if matches!(
            signed_halfword_multiply,
            0x0100_0080 | 0x0140_0080 | 0x0160_0080
        ) {
            return self.execute_signed_halfword_multiply(instruction, pc);
        }
        if instruction & 0x0fc0_00f0 == 0x0000_0090 {
            return self.execute_multiply(instruction, pc);
        }
        if instruction & 0x0f80_00f0 == 0x0080_0090 {
            return self.execute_long_multiply(instruction, pc);
        }
        if instruction & 0x0e00_0090 == 0x0000_0090 {
            return self.execute_half_transfer(instruction, pc, bus);
        }
        self.execute_data_processing(instruction, pc)
    }

    fn execute_data_processing(&mut self, instruction: u32, pc: u32) -> Result<()> {
        let opcode = (instruction >> 21) & 0xf;
        let set_flags = instruction & (1 << 20) != 0;
        let rn = ((instruction >> 16) & 0xf) as usize;
        let rd = ((instruction >> 12) & 0xf) as usize;
        let left = self.read_reg(rn, pc, false);
        let (right, shifter_carry) = self.decode_operand2(instruction, pc);
        let carry = u32::from(self.cpsr & C != 0);
        let (result, carry_out, overflow) = match opcode {
            0 | 8 => (left & right, shifter_carry, false),
            1 | 9 => (left ^ right, shifter_carry, false),
            2 | 10 => sub_with_carry(left, right, 1),
            3 => sub_with_carry(right, left, 1),
            4 | 11 => add_with_carry(left, right, 0),
            5 => add_with_carry(left, right, carry),
            6 => sub_with_carry(left, right, carry),
            7 => sub_with_carry(right, left, carry),
            12 => (left | right, shifter_carry, false),
            13 => (right, shifter_carry, false),
            14 => (left & !right, shifter_carry, false),
            15 => (!right, shifter_carry, false),
            _ => unreachable!(),
        };
        let test_only = matches!(opcode, 8..=11);
        if !test_only {
            self.write_reg(rd, result);
        }
        if set_flags || test_only {
            self.set_nz(result);
            self.set_flag(C, carry_out);
            if matches!(opcode, 2..=7 | 10 | 11) {
                self.set_flag(V, overflow);
            }
        }
        Ok(())
    }

    fn execute_signed_halfword_multiply(&mut self, instruction: u32, pc: u32) -> Result<()> {
        let operation = instruction & 0x0ff0_0090;
        let destination_high = ((instruction >> 16) & 0xf) as usize;
        let accumulator_or_low = ((instruction >> 12) & 0xf) as usize;
        let right_register = ((instruction >> 8) & 0xf) as usize;
        let left_register = (instruction & 0xf) as usize;
        if [
            destination_high,
            accumulator_or_low,
            right_register,
            left_register,
        ]
        .contains(&15)
            || (operation == 0x0140_0080 && destination_high == accumulator_or_low)
        {
            return Err(SimulatorError::InvalidInstruction {
                pc,
                instr: instruction,
            });
        }

        let selected_half = |value: u32, top: bool| -> i64 {
            i64::from(if top {
                (value >> 16) as i16
            } else {
                value as i16
            })
        };
        let left = selected_half(self.r[left_register], instruction & (1 << 5) != 0);
        let right = selected_half(self.r[right_register], instruction & (1 << 6) != 0);
        let product = left * right;

        match operation {
            0x0100_0080 => {
                let accumulator = i64::from(self.r[accumulator_or_low] as i32);
                let result = product + accumulator;
                self.r[destination_high] = result as u32;
                if result < i64::from(i32::MIN) || result > i64::from(i32::MAX) {
                    self.cpsr |= Q;
                }
            }
            0x0140_0080 => {
                let accumulator = ((u64::from(self.r[destination_high]) << 32)
                    | u64::from(self.r[accumulator_or_low]))
                    as i64;
                let result = accumulator.wrapping_add(product);
                self.r[accumulator_or_low] = result as u32;
                self.r[destination_high] = (result as u64 >> 32) as u32;
            }
            0x0160_0080 => self.r[destination_high] = product as u32,
            _ => unreachable!(),
        }
        Ok(())
    }

    fn decode_operand2(&self, instruction: u32, pc: u32) -> (u32, bool) {
        if instruction & (1 << 25) != 0 {
            let value = instruction & 0xff;
            let rotate = ((instruction >> 8) & 0xf) * 2;
            let result = value.rotate_right(rotate);
            return (
                result,
                if rotate == 0 {
                    self.cpsr & C != 0
                } else {
                    result >> 31 != 0
                },
            );
        }
        let value = self.read_reg((instruction & 0xf) as usize, pc, false);
        let kind = (instruction >> 5) & 3;
        let amount = if instruction & (1 << 4) != 0 {
            self.read_reg(((instruction >> 8) & 0xf) as usize, pc, false) & 0xff
        } else {
            (instruction >> 7) & 0x1f
        };
        shift(
            value,
            kind,
            amount,
            self.cpsr & C != 0,
            instruction & (1 << 4) == 0,
        )
    }

    fn execute_multiply(&mut self, instruction: u32, pc: u32) -> Result<()> {
        let rd = ((instruction >> 16) & 0xf) as usize;
        let rn = ((instruction >> 12) & 0xf) as usize;
        let rs = ((instruction >> 8) & 0xf) as usize;
        let rm = (instruction & 0xf) as usize;
        if rd == 15 || rm == 15 || rs == 15 || rn == 15 {
            return Err(SimulatorError::InvalidInstruction {
                pc,
                instr: instruction,
            });
        }
        let mut result = self.r[rm].wrapping_mul(self.r[rs]);
        if instruction & (1 << 21) != 0 {
            result = result.wrapping_add(self.r[rn]);
        }
        self.r[rd] = result;
        if instruction & (1 << 20) != 0 {
            self.set_nz(result);
        }
        Ok(())
    }

    fn execute_long_multiply(&mut self, instruction: u32, pc: u32) -> Result<()> {
        let hi = ((instruction >> 16) & 0xf) as usize;
        let lo = ((instruction >> 12) & 0xf) as usize;
        let rs = ((instruction >> 8) & 0xf) as usize;
        let rm = (instruction & 0xf) as usize;
        if [hi, lo, rs, rm].contains(&15) || hi == lo {
            return Err(SimulatorError::InvalidInstruction {
                pc,
                instr: instruction,
            });
        }
        let signed = instruction & (1 << 22) != 0;
        let mut result = if signed {
            (self.r[rm] as i32 as i64).wrapping_mul(self.r[rs] as i32 as i64) as u64
        } else {
            u64::from(self.r[rm]) * u64::from(self.r[rs])
        };
        if instruction & (1 << 21) != 0 {
            result = result.wrapping_add((u64::from(self.r[hi]) << 32) | u64::from(self.r[lo]));
        }
        self.r[lo] = result as u32;
        self.r[hi] = (result >> 32) as u32;
        if instruction & (1 << 20) != 0 {
            self.set_flag(N, result >> 63 != 0);
            self.set_flag(Z, result == 0);
        }
        Ok(())
    }

    fn execute_single_transfer<B: ArmBus>(
        &mut self,
        instruction: u32,
        pc: u32,
        bus: &mut B,
    ) -> Result<()> {
        let rn = ((instruction >> 16) & 0xf) as usize;
        let rd = ((instruction >> 12) & 0xf) as usize;
        let base = self.read_reg(rn, pc, false);
        let offset = if instruction & (1 << 25) == 0 {
            instruction & 0xfff
        } else {
            let rm = self.read_reg((instruction & 0xf) as usize, pc, false);
            let kind = (instruction >> 5) & 3;
            let amount = (instruction >> 7) & 0x1f;
            shift(rm, kind, amount, self.cpsr & C != 0, true).0
        };
        let adjusted = if instruction & (1 << 23) != 0 {
            base.wrapping_add(offset)
        } else {
            base.wrapping_sub(offset)
        };
        let pre = instruction & (1 << 24) != 0;
        let address = if pre { adjusted } else { base };
        let byte = instruction & (1 << 22) != 0;
        if instruction & (1 << 20) != 0 {
            let value = if byte {
                u32::from(bus.read8(address)?)
            } else {
                bus.read32(address & !3)?.rotate_right((address & 3) * 8)
            };
            if rd == 15 {
                self.branch_exchange(value)?;
            } else {
                self.r[rd] = value;
            }
        } else {
            let value = self.read_reg(rd, pc, true);
            if byte {
                bus.write8(address, value as u8)?;
            } else {
                bus.write32(address, value)?;
            }
        }
        if (!pre || instruction & (1 << 21) != 0) && rn != 15 {
            self.r[rn] = adjusted;
        }
        Ok(())
    }

    fn execute_half_transfer<B: ArmBus>(
        &mut self,
        instruction: u32,
        pc: u32,
        bus: &mut B,
    ) -> Result<()> {
        let rn = ((instruction >> 16) & 0xf) as usize;
        let rd = ((instruction >> 12) & 0xf) as usize;
        let base = self.read_reg(rn, pc, false);
        let offset = if instruction & (1 << 22) != 0 {
            ((instruction >> 4) & 0xf0) | (instruction & 0xf)
        } else {
            self.read_reg((instruction & 0xf) as usize, pc, false)
        };
        let adjusted = if instruction & (1 << 23) != 0 {
            base.wrapping_add(offset)
        } else {
            base.wrapping_sub(offset)
        };
        let pre = instruction & (1 << 24) != 0;
        let address = if pre { adjusted } else { base };
        let kind = (instruction >> 5) & 3;
        let load = instruction & (1 << 20) != 0;
        match (load, kind) {
            (true, 1) => self.write_reg(rd, u32::from(bus.read16(address)?)),
            (true, 2) => self.write_reg(rd, bus.read8(address)? as i8 as i32 as u32),
            (true, 3) => self.write_reg(rd, bus.read16(address)? as i16 as i32 as u32),
            (false, 1) => bus.write16(address, self.read_reg(rd, pc, true) as u16)?,
            (false, 2 | 3) if rd & 1 == 0 && rd < 14 => {
                if kind == 2 {
                    self.r[rd] = bus.read32(address)?;
                    self.r[rd + 1] = bus.read32(address.wrapping_add(4))?;
                } else {
                    bus.write32(address, self.r[rd])?;
                    bus.write32(address.wrapping_add(4), self.r[rd + 1])?;
                }
            }
            _ => {
                return Err(SimulatorError::InvalidInstruction {
                    pc,
                    instr: instruction,
                })
            }
        }
        if (!pre || instruction & (1 << 21) != 0) && rn != 15 {
            self.r[rn] = adjusted;
        }
        Ok(())
    }

    fn execute_block_transfer<B: ArmBus>(
        &mut self,
        instruction: u32,
        pc: u32,
        bus: &mut B,
    ) -> Result<()> {
        let rn = ((instruction >> 16) & 0xf) as usize;
        let list = instruction & 0xffff;
        if list == 0 {
            return Err(SimulatorError::InvalidInstruction {
                pc,
                instr: instruction,
            });
        }
        let count = list.count_ones();
        let base = self.r[rn];
        let up = instruction & (1 << 23) != 0;
        let pre = instruction & (1 << 24) != 0;
        let mut address = if up {
            base
        } else {
            base.wrapping_sub(count * 4)
        };
        if pre == up {
            address = address.wrapping_add(4);
        }
        let load = instruction & (1 << 20) != 0;
        for reg in 0..16 {
            if list & (1 << reg) == 0 {
                continue;
            }
            if load {
                let value = bus.read32(address)?;
                if reg == 15 {
                    self.branch_exchange(value)?;
                } else {
                    self.r[reg] = value;
                }
            } else {
                bus.write32(address, self.read_reg(reg, pc, true))?;
            }
            address = address.wrapping_add(4);
        }
        if instruction & (1 << 21) != 0 && !(load && list & (1 << rn) != 0) {
            self.r[rn] = if up {
                base.wrapping_add(count * 4)
            } else {
                base.wrapping_sub(count * 4)
            };
        }
        Ok(())
    }

    fn execute_branch(&mut self, instruction: u32, pc: u32) -> Result<()> {
        let offset = (((instruction & 0x00ff_ffff) << 8) as i32 >> 6) as u32;
        if instruction & (1 << 24) != 0 {
            self.r[14] = pc.wrapping_add(4);
        }
        self.r[15] = pc.wrapping_add(8).wrapping_add(offset);
        Ok(())
    }

    fn execute_thumb<B: ArmBus>(&mut self, instruction: u16, pc: u32, bus: &mut B) -> Result<()> {
        let op = u32::from(instruction);
        let rd = (op & 7) as usize;
        let rs = ((op >> 3) & 7) as usize;

        if op & 0xe000 == 0 {
            if op & 0x1800 != 0x1800 {
                let (result, carry) = shift(
                    self.r[rs],
                    (op >> 11) & 3,
                    (op >> 6) & 0x1f,
                    self.cpsr & C != 0,
                    true,
                );
                self.r[rd] = result;
                self.set_nz(result);
                self.set_flag(C, carry);
            } else {
                let right = if op & (1 << 10) != 0 {
                    (op >> 6) & 7
                } else {
                    self.r[((op >> 6) & 7) as usize]
                };
                let (result, carry, overflow) = if op & (1 << 9) != 0 {
                    sub_with_carry(self.r[rs], right, 1)
                } else {
                    add_with_carry(self.r[rs], right, 0)
                };
                self.r[rd] = result;
                self.set_nz(result);
                self.set_flag(C, carry);
                self.set_flag(V, overflow);
            }
            return Ok(());
        }

        if op & 0xe000 == 0x2000 {
            let opcode = (op >> 11) & 3;
            let register = ((op >> 8) & 7) as usize;
            let immediate = op & 0xff;
            let (result, carry, overflow) = match opcode {
                0 => (immediate, self.cpsr & C != 0, false),
                1 | 3 => sub_with_carry(self.r[register], immediate, 1),
                2 => add_with_carry(self.r[register], immediate, 0),
                _ => unreachable!(),
            };
            if opcode != 1 {
                self.r[register] = result;
            }
            self.set_nz(result);
            if opcode != 0 {
                self.set_flag(C, carry);
                self.set_flag(V, overflow);
            }
            return Ok(());
        }

        if op & 0xfc00 == 0x4000 {
            let opcode = (op >> 6) & 0xf;
            let left = self.r[rd];
            let right = self.r[rs];
            let carry_in = u32::from(self.cpsr & C != 0);
            let (result, carry, overflow) = match opcode {
                0 | 8 => (left & right, self.cpsr & C != 0, false),
                1 => (left ^ right, self.cpsr & C != 0, false),
                2..=4 | 7 => {
                    let kind = if opcode == 7 { 3 } else { opcode - 2 };
                    let (value, carry) = shift(left, kind, right & 0xff, self.cpsr & C != 0, false);
                    (value, carry, false)
                }
                5 => add_with_carry(left, right, carry_in),
                6 => sub_with_carry(left, right, carry_in),
                9 => sub_with_carry(0, right, 1),
                10 => sub_with_carry(left, right, 1),
                11 => add_with_carry(left, right, 0),
                12 => (left | right, self.cpsr & C != 0, false),
                13 => (left.wrapping_mul(right), self.cpsr & C != 0, false),
                14 => (left & !right, self.cpsr & C != 0, false),
                15 => (!right, self.cpsr & C != 0, false),
                _ => return Err(SimulatorError::InvalidInstruction { pc, instr: op }),
            };
            if !matches!(opcode, 8 | 10 | 11) {
                self.r[rd] = result;
            }
            self.set_nz(result);
            if matches!(opcode, 2..=7 | 9..=11) {
                self.set_flag(C, carry);
            }
            if matches!(opcode, 5 | 6 | 9..=11) {
                self.set_flag(V, overflow);
            }
            return Ok(());
        }

        if op & 0xfc00 == 0x4400 {
            let opcode = (op >> 8) & 3;
            let destination = ((op & 7) | ((op >> 4) & 8)) as usize;
            let source = ((op >> 3) & 0xf) as usize;
            let right = self.read_reg(source, pc, false);
            match opcode {
                0 => self.write_reg(
                    destination,
                    self.read_reg(destination, pc, false).wrapping_add(right),
                ),
                1 => {
                    let (result, carry, overflow) =
                        sub_with_carry(self.read_reg(destination, pc, false), right, 1);
                    self.set_nz(result);
                    self.set_flag(C, carry);
                    self.set_flag(V, overflow);
                }
                2 => self.write_reg(destination, right),
                3 => {
                    if op & (1 << 7) != 0 {
                        self.r[14] = pc.wrapping_add(2) | 1;
                    }
                    self.branch_exchange(right)?;
                }
                _ => unreachable!(),
            }
            return Ok(());
        }

        if op & 0xf800 == 0x4800 {
            let register = ((op >> 8) & 7) as usize;
            let address = (pc.wrapping_add(4) & !3).wrapping_add((op & 0xff) << 2);
            self.r[register] = bus.read32(address)?;
            return Ok(());
        }

        if op & 0xf000 == 0x5000 {
            let opcode = (op >> 9) & 7;
            let address = self.r[rs].wrapping_add(self.r[((op >> 6) & 7) as usize]);
            match opcode {
                0 => bus.write32(address, self.r[rd])?,
                1 => bus.write16(address, self.r[rd] as u16)?,
                2 => bus.write8(address, self.r[rd] as u8)?,
                3 => self.r[rd] = bus.read8(address)? as i8 as i32 as u32,
                4 => self.r[rd] = bus.read32(address & !3)?.rotate_right((address & 3) * 8),
                5 => self.r[rd] = u32::from(bus.read16(address)?),
                6 => self.r[rd] = u32::from(bus.read8(address)?),
                7 => self.r[rd] = bus.read16(address)? as i16 as i32 as u32,
                _ => unreachable!(),
            }
            return Ok(());
        }

        if op & 0xe000 == 0x6000 {
            let byte = op & (1 << 12) != 0;
            let load = op & (1 << 11) != 0;
            let address = self.r[rs].wrapping_add(if byte {
                (op >> 6) & 0x1f
            } else {
                ((op >> 6) & 0x1f) << 2
            });
            match (load, byte) {
                (false, false) => bus.write32(address, self.r[rd])?,
                (false, true) => bus.write8(address, self.r[rd] as u8)?,
                (true, false) => self.r[rd] = bus.read32(address)?,
                (true, true) => self.r[rd] = u32::from(bus.read8(address)?),
            }
            return Ok(());
        }

        if op & 0xf000 == 0x8000 {
            let address = self.r[rs].wrapping_add(((op >> 6) & 0x1f) << 1);
            if op & (1 << 11) != 0 {
                self.r[rd] = u32::from(bus.read16(address)?);
            } else {
                bus.write16(address, self.r[rd] as u16)?;
            }
            return Ok(());
        }

        if op & 0xf000 == 0x9000 {
            let register = ((op >> 8) & 7) as usize;
            let address = self.r[13].wrapping_add((op & 0xff) << 2);
            if op & (1 << 11) != 0 {
                self.r[register] = bus.read32(address)?;
            } else {
                bus.write32(address, self.r[register])?;
            }
            return Ok(());
        }

        if op & 0xf000 == 0xa000 {
            let register = ((op >> 8) & 7) as usize;
            let base = if op & (1 << 11) != 0 {
                self.r[13]
            } else {
                pc.wrapping_add(4) & !3
            };
            self.r[register] = base.wrapping_add((op & 0xff) << 2);
            return Ok(());
        }

        if op & 0xff00 == 0xb000 {
            let amount = (op & 0x7f) << 2;
            self.r[13] = if op & 0x80 != 0 {
                self.r[13].wrapping_sub(amount)
            } else {
                self.r[13].wrapping_add(amount)
            };
            return Ok(());
        }

        if op & 0xf600 == 0xb400 {
            let pop = op & (1 << 11) != 0;
            let list = op & 0xff;
            if pop {
                for reg in 0..8 {
                    if list & (1 << reg) != 0 {
                        self.r[reg] = bus.read32(self.r[13])?;
                        self.r[13] = self.r[13].wrapping_add(4);
                    }
                }
                if op & (1 << 8) != 0 {
                    let target = bus.read32(self.r[13])?;
                    self.r[13] = self.r[13].wrapping_add(4);
                    self.branch_exchange(target)?;
                }
            } else {
                if op & (1 << 8) != 0 {
                    self.r[13] = self.r[13].wrapping_sub(4);
                    bus.write32(self.r[13], self.r[14])?;
                }
                for reg in (0..8).rev() {
                    if list & (1 << reg) != 0 {
                        self.r[13] = self.r[13].wrapping_sub(4);
                        bus.write32(self.r[13], self.r[reg])?;
                    }
                }
            }
            return Ok(());
        }

        if op & 0xf000 == 0xc000 {
            let load = op & (1 << 11) != 0;
            let base = ((op >> 8) & 7) as usize;
            let list = op & 0xff;
            if list == 0 {
                return Err(SimulatorError::InvalidInstruction { pc, instr: op });
            }
            let mut address = self.r[base];
            for reg in 0..8 {
                if list & (1 << reg) != 0 {
                    if load {
                        self.r[reg] = bus.read32(address)?;
                    } else {
                        bus.write32(address, self.r[reg])?;
                    }
                    address = address.wrapping_add(4);
                }
            }
            if !(load && list & (1 << base) != 0) {
                self.r[base] = address;
            }
            return Ok(());
        }

        if op & 0xf000 == 0xd000 {
            let condition = (op >> 8) & 0xf;
            if condition == 0xf {
                return bus.svc(self, op & 0xff);
            }
            if condition == 0xe {
                return Err(SimulatorError::InvalidInstruction { pc, instr: op });
            }
            if self.condition_passed(condition) {
                self.r[15] = pc
                    .wrapping_add(4)
                    .wrapping_add(((op as u8 as i8 as i32) << 1) as u32);
            }
            return Ok(());
        }

        match op >> 11 {
            0b11100 => {
                self.r[15] = pc
                    .wrapping_add(4)
                    .wrapping_add((((op & 0x7ff) << 21) as i32 >> 20) as u32)
            }
            0b11110 => {
                self.r[14] = pc
                    .wrapping_add(4)
                    .wrapping_add((((op & 0x7ff) << 21) as i32 >> 9) as u32)
            }
            0b11101 => {
                let target = self.r[14].wrapping_add((op & 0x7ff) << 1) & !3;
                self.r[14] = pc.wrapping_add(2) | 1;
                self.state = ArmExecutionState::Arm;
                self.cpsr &= !T;
                self.r[15] = target;
            }
            0b11111 => {
                let target = self.r[14].wrapping_add((op & 0x7ff) << 1);
                self.r[14] = pc.wrapping_add(2) | 1;
                self.r[15] = target & !1;
            }
            _ => return Err(SimulatorError::InvalidInstruction { pc, instr: op }),
        }
        Ok(())
    }

    fn branch_exchange(&mut self, target: u32) -> Result<()> {
        if target & 1 != 0 {
            self.state = ArmExecutionState::Thumb;
            self.cpsr |= T;
            self.r[15] = target & !1;
        } else {
            self.state = ArmExecutionState::Arm;
            self.cpsr &= !T;
            self.r[15] = target & !3;
        }
        Ok(())
    }

    fn read_reg(&self, reg: usize, pc: u32, store: bool) -> u32 {
        if reg != 15 {
            return self.r[reg];
        }
        match self.state {
            ArmExecutionState::Arm => pc.wrapping_add(if store { 12 } else { 8 }),
            ArmExecutionState::Thumb => pc.wrapping_add(4),
        }
    }

    fn write_reg(&mut self, reg: usize, value: u32) {
        if reg == 15 {
            self.r[15] = match self.state {
                ArmExecutionState::Arm => value & !3,
                ArmExecutionState::Thumb => value & !1,
            };
        } else {
            self.r[reg] = value;
        }
    }

    fn condition_passed(&self, condition: u32) -> bool {
        let n = self.cpsr & N != 0;
        let z = self.cpsr & Z != 0;
        let c = self.cpsr & C != 0;
        let v = self.cpsr & V != 0;
        match condition {
            0 => z,
            1 => !z,
            2 => c,
            3 => !c,
            4 => n,
            5 => !n,
            6 => v,
            7 => !v,
            8 => c && !z,
            9 => !c || z,
            10 => n == v,
            11 => n != v,
            12 => !z && n == v,
            13 => z || n != v,
            14 => true,
            _ => false,
        }
    }

    fn set_nz(&mut self, value: u32) {
        self.set_flag(N, value >> 31 != 0);
        self.set_flag(Z, value == 0);
    }

    fn set_flag(&mut self, flag: u32, set: bool) {
        if set {
            self.cpsr |= flag;
        } else {
            self.cpsr &= !flag;
        }
    }
}

fn add_with_carry(left: u32, right: u32, carry: u32) -> (u32, bool, bool) {
    let wide = u64::from(left) + u64::from(right) + u64::from(carry);
    let result = wide as u32;
    let signed = i64::from(left as i32) + i64::from(right as i32) + i64::from(carry);
    (result, wide >> 32 != 0, signed != i64::from(result as i32))
}

fn sub_with_carry(left: u32, right: u32, carry: u32) -> (u32, bool, bool) {
    add_with_carry(left, !right, carry)
}

fn shift(value: u32, kind: u32, amount: u32, old_carry: bool, immediate: bool) -> (u32, bool) {
    match (kind, amount) {
        (0, 0) => (value, old_carry),
        (0, 1..=31) => (value << amount, value >> (32 - amount) & 1 != 0),
        (0, 32) => (0, value & 1 != 0),
        (0, _) => (0, false),
        (1, 0) if immediate => (0, value >> 31 != 0),
        (1, 0) => (value, old_carry),
        (1, 1..=31) => (value >> amount, value >> (amount - 1) & 1 != 0),
        (1, 32) => (0, value >> 31 != 0),
        (1, _) => (0, false),
        (2, 0) if immediate => ((value as i32 >> 31) as u32, value >> 31 != 0),
        (2, 0) => (value, old_carry),
        (2, 1..=31) => (
            (value as i32 >> amount) as u32,
            value >> (amount - 1) & 1 != 0,
        ),
        (2, _) => ((value as i32 >> 31) as u32, value >> 31 != 0),
        (3, 0) if immediate => ((u32::from(old_carry) << 31) | (value >> 1), value & 1 != 0),
        (3, 0) => (value, old_carry),
        (3, _) => {
            let rotate = amount & 31;
            if rotate == 0 {
                (value, value >> 31 != 0)
            } else {
                let result = value.rotate_right(rotate);
                (result, result >> 31 != 0)
            }
        }
        _ => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestBus {
        data: Vec<u8>,
        svc: Option<u32>,
    }

    impl TestBus {
        fn new(instructions: &[u32]) -> Self {
            let mut data = vec![0; 0x1000];
            for (index, instruction) in instructions.iter().enumerate() {
                data[index * 4..index * 4 + 4].copy_from_slice(&instruction.to_le_bytes());
            }
            Self { data, svc: None }
        }

        fn new_thumb(instructions: &[u16]) -> Self {
            let mut data = vec![0; 0x1000];
            for (index, instruction) in instructions.iter().enumerate() {
                data[index * 2..index * 2 + 2].copy_from_slice(&instruction.to_le_bytes());
            }
            Self { data, svc: None }
        }

        fn range(&self, address: u32, size: usize) -> Result<std::ops::Range<usize>> {
            let start = address as usize;
            let end = start
                .checked_add(size)
                .ok_or_else(|| SimulatorError::MemoryError {
                    addr: address,
                    message: "address overflow".into(),
                })?;
            if end > self.data.len() {
                return Err(SimulatorError::MemoryError {
                    addr: address,
                    message: "test bus access outside memory".into(),
                });
            }
            Ok(start..end)
        }
    }

    impl ArmBus for TestBus {
        fn read8(&mut self, address: u32) -> Result<u8> {
            Ok(self.data[self.range(address, 1)?.start])
        }

        fn read16(&mut self, address: u32) -> Result<u16> {
            let bytes: [u8; 2] = self.data[self.range(address, 2)?].try_into().unwrap();
            Ok(u16::from_le_bytes(bytes))
        }

        fn read32(&mut self, address: u32) -> Result<u32> {
            let bytes: [u8; 4] = self.data[self.range(address, 4)?].try_into().unwrap();
            Ok(u32::from_le_bytes(bytes))
        }

        fn write8(&mut self, address: u32, value: u8) -> Result<()> {
            let index = self.range(address, 1)?.start;
            self.data[index] = value;
            Ok(())
        }

        fn write16(&mut self, address: u32, value: u16) -> Result<()> {
            let range = self.range(address, 2)?;
            self.data[range].copy_from_slice(&value.to_le_bytes());
            Ok(())
        }

        fn write32(&mut self, address: u32, value: u32) -> Result<()> {
            let range = self.range(address, 4)?;
            self.data[range].copy_from_slice(&value.to_le_bytes());
            Ok(())
        }

        fn svc(&mut self, cpu: &mut ArmCpu, immediate: u32) -> Result<()> {
            self.svc = Some(immediate);
            cpu.r[0] = 0x55aa;
            Ok(())
        }
    }

    fn running_cpu() -> ArmCpu {
        let mut cpu = ArmCpu::new(0, 0xf00, 0xffff_ffff);
        cpu.start();
        cpu
    }

    #[test]
    fn data_processing_sets_flags_and_honors_conditions() {
        let mut bus = TestBus::new(&[
            0xe3e0_0000, // MVN r0, #0
            0xe290_1001, // ADDS r1, r0, #1
            0x03a0_2007, // MOVEQ r2, #7
            0x13a0_3009, // MOVNE r3, #9
        ]);
        let mut cpu = running_cpu();
        assert_eq!(cpu.run(&mut bus, 4).unwrap(), 4);
        assert_eq!(cpu.r[1], 0);
        assert_eq!(cpu.r[2], 7);
        assert_eq!(cpu.r[3], 0);
        assert_ne!(cpu.cpsr & Z, 0);
        assert_ne!(cpu.cpsr & C, 0);
    }

    #[test]
    fn logical_shift_left_uses_the_last_shifted_bit_for_carry() {
        let mut bus = TestBus::new(&[
            0xe1b0_ce02, // MOVS r12, r2, LSL #28
            0x23a0_3001, // MOVHS r3, #1
        ]);
        let mut cpu = running_cpu();
        cpu.r[2] = 0xffff_ffe4;
        cpu.run(&mut bus, 2).unwrap();
        assert_eq!(cpu.r[12], 0x4000_0000);
        assert_eq!(cpu.cpsr & C, 0);
        assert_eq!(cpu.r[3], 0);
    }

    #[test]
    fn branch_link_and_exchange_use_arm_pc_semantics() {
        let mut bus = TestBus::new(&[0xeb00_0001]);
        let mut cpu = running_cpu();
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.r[14], 4);
        assert_eq!(cpu.r[15], 12);

        bus.write32(12, 0xe12f_ff10).unwrap();
        cpu.r[0] = 0x101;
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.execution_state(), ArmExecutionState::Thumb);
        assert_eq!(cpu.r[15], 0x100);
    }

    #[test]
    fn single_and_block_transfers_apply_writeback() {
        let mut bus = TestBus::new(&[
            0xe5a0_1004, // STR r1, [r0, #4]!
            0xe4d0_2001, // LDRB r2, [r0], #1
            0xe8a0_0006, // STMIA r0!, {r1, r2}
        ]);
        let mut cpu = running_cpu();
        cpu.r[0] = 0x100;
        cpu.r[1] = 0x1122_3344;
        cpu.step(&mut bus).unwrap();
        assert_eq!(bus.read32(0x104).unwrap(), 0x1122_3344);
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.r[2], 0x44);
        cpu.step(&mut bus).unwrap();
        assert_eq!(bus.read32(0x105).unwrap(), 0x1122_3344);
        assert_eq!(bus.read32(0x109).unwrap(), 0x44);
        assert_eq!(cpu.r[0], 0x10d);
    }

    #[test]
    fn doubleword_transfers_move_adjacent_registers() {
        let mut bus = TestBus::new(&[
            0xe1cd_20f0, // STRD r2, r3, [sp]
            0xe1cd_40d0, // LDRD r4, r5, [sp]
        ]);
        let mut cpu = running_cpu();
        cpu.r[13] = 0x100;
        cpu.r[2] = 0x1122_3344;
        cpu.r[3] = 0x5566_7788;
        cpu.step(&mut bus).unwrap();
        assert_eq!(bus.read32(0x100).unwrap(), 0x1122_3344);
        assert_eq!(bus.read32(0x104).unwrap(), 0x5566_7788);
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.r[4], 0x1122_3344);
        assert_eq!(cpu.r[5], 0x5566_7788);
    }

    #[test]
    fn arm_loads_into_pc_support_thumb_interworking() {
        let mut bus = TestBus::new(&[
            0xe590_f000, // LDR pc, [r0]
            0xe8b0_8000, // LDMIA r0!, {pc}
        ]);
        bus.write32(0x100, 0x201).unwrap();
        let mut cpu = running_cpu();
        cpu.r[0] = 0x100;
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.execution_state(), ArmExecutionState::Thumb);
        assert_eq!(cpu.r[15], 0x200);

        cpu = running_cpu();
        cpu.r[15] = 4;
        cpu.r[0] = 0x100;
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.execution_state(), ArmExecutionState::Thumb);
        assert_eq!(cpu.r[15], 0x200);
        assert_eq!(cpu.r[0], 0x104);
    }

    #[test]
    fn multiply_long_multiply_and_clz_are_supported() {
        let mut bus = TestBus::new(&[0xe003_0192, 0xe083_2190, 0xe16f_4f13]);
        let mut cpu = running_cpu();
        cpu.r[0] = 0xffff_ffff;
        cpu.r[1] = 2;
        cpu.r[2] = 3;
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.r[3], 6);
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.r[2], 0xffff_fffe);
        assert_eq!(cpu.r[3], 1);
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.r[4], 31);
    }

    #[test]
    fn signed_halfword_multiply_variants_use_selected_halves() {
        let mut bus = TestBus::new(&[
            0xe163_0281, // SMULBB r3, r1, r2
            0xe163_02c1, // SMULBT r3, r1, r2
            0xe163_02a1, // SMULTB r3, r1, r2
            0xe163_02e1, // SMULTT r3, r1, r2
            0xe105_4281, // SMLABB r5, r1, r2, r4
            0xe142_1480, // SMLALBB r1, r2, r0, r4
        ]);
        let mut cpu = running_cpu();
        cpu.r[0] = 2;
        cpu.r[1] = 0xfffe_0003;
        cpu.r[2] = 0x0004_fffd;
        cpu.r[4] = 10;
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.r[3], (-9_i32) as u32);
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.r[3], 12);
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.r[3], 6);
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.r[3], (-8_i32) as u32);
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.r[5], 1);

        cpu.r[1] = u32::MAX;
        cpu.r[2] = u32::MAX;
        cpu.r[4] = 3;
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.r[1], 5);
        assert_eq!(cpu.r[2], 0);
    }

    #[test]
    fn signed_halfword_multiply_accumulate_sets_sticky_q_on_overflow() {
        let mut bus = TestBus::new(&[0xe105_4281, 0xe105_4281]);
        let mut cpu = running_cpu();
        cpu.r[1] = 0x0000_8000;
        cpu.r[2] = 0x0000_8000;
        cpu.r[4] = 0x4000_0000;
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.r[5], 0x8000_0000);
        assert_ne!(cpu.cpsr & Q, 0);

        cpu.r[1] = 1;
        cpu.r[2] = 1;
        cpu.r[4] = 1;
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.r[5], 2);
        assert_ne!(cpu.cpsr & Q, 0);
    }

    #[test]
    fn svc_can_update_cpu_state() {
        let mut bus = TestBus::new(&[0xef00_0123]);
        let mut cpu = running_cpu();
        cpu.step(&mut bus).unwrap();
        assert_eq!(bus.svc, Some(0x123));
        assert_eq!(cpu.r[0], 0x55aa);
    }

    #[test]
    fn invalid_instruction_and_memory_errors_are_reported() {
        let mut bus = TestBus::new(&[0xee00_0010]);
        let mut cpu = running_cpu();
        cpu.set_unknown_instruction_policy(UnknownInstructionPolicy::Stop);
        assert!(matches!(
            cpu.step(&mut bus),
            Err(SimulatorError::InvalidInstruction { .. })
        ));

        let mut skip_cpu = running_cpu();
        skip_cpu.step(&mut bus).unwrap();
        assert_eq!(skip_cpu.r[15], 4);
        assert_eq!(skip_cpu.instruction_count, 1);

        cpu.r[15] = 0x1000;
        assert!(matches!(
            cpu.step(&mut bus),
            Err(SimulatorError::MemoryError { .. })
        ));
    }

    #[test]
    fn run_respects_the_instruction_limit() {
        let mut bus = TestBus::new(&[0xeaff_fffe]);
        let mut cpu = running_cpu();
        assert_eq!(cpu.run(&mut bus, 25).unwrap(), 25);
        assert_eq!(cpu.r[15], 0);
    }

    #[test]
    fn thumb_arithmetic_and_conditional_branch_set_flags() {
        let mut bus = TestBus::new_thumb(&[0x2005, 0x3003, 0x2808, 0xd000, 0x2101, 0x2102]);
        let mut cpu = ArmCpu::new(1, 0xf00, 0xffff_ffff);
        cpu.start();
        cpu.run(&mut bus, 3).unwrap();
        assert_eq!(cpu.r[0], 8);
        assert_ne!(cpu.cpsr & Z, 0);
        cpu.run(&mut bus, 2).unwrap();
        assert_eq!(cpu.r[1], 2);
    }

    #[test]
    fn thumb_load_store_and_stack_round_trip() {
        let mut bus = TestBus::new_thumb(&[0x6008, 0x680a, 0xb503, 0xbd0c]);
        let mut cpu = ArmCpu::new(1, 0xf00, 0x21);
        cpu.r[0] = 0x1234_5678;
        cpu.r[1] = 0x100;
        cpu.start();
        cpu.step(&mut bus).unwrap();
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.r[2], 0x1234_5678);
        cpu.step(&mut bus).unwrap();
        cpu.step(&mut bus).unwrap();
        assert_eq!(cpu.r[2], 0x1234_5678);
        assert_eq!(cpu.r[3], 0x100);
        assert_eq!(cpu.r[15], 0x20);
        assert_eq!(cpu.r[13], 0xf00);
    }

    #[test]
    fn thumb_long_branch_and_svc_preserve_return_state() {
        let mut bus = TestBus::new_thumb(&[0xf000, 0xf802, 0x2000, 0x2000, 0xdf42]);
        let mut cpu = ArmCpu::new(1, 0xf00, 0);
        cpu.start();
        cpu.run(&mut bus, 3).unwrap();
        assert_eq!(cpu.r[14], 5);
        assert_eq!(cpu.r[15], 10);
        assert_eq!(bus.svc, Some(0x42));
        assert_eq!(cpu.r[0], 0x55aa);
    }

    #[test]
    fn immediate_blx_switches_between_arm_and_thumb() {
        let mut arm_bus = TestBus::new(&[0xfa00_0000]);
        let mut arm_cpu = running_cpu();
        arm_cpu.step(&mut arm_bus).unwrap();
        assert_eq!(arm_cpu.execution_state(), ArmExecutionState::Thumb);
        assert_eq!(arm_cpu.r[15], 8);
        assert_eq!(arm_cpu.r[14], 4);

        let mut thumb_bus = TestBus::new_thumb(&[0xf000, 0xe802]);
        let mut thumb_cpu = ArmCpu::new(1, 0xf00, 0);
        thumb_cpu.start();
        thumb_cpu.run(&mut thumb_bus, 2).unwrap();
        assert_eq!(thumb_cpu.execution_state(), ArmExecutionState::Arm);
        assert_eq!(thumb_cpu.r[15], 8);
        assert_eq!(thumb_cpu.r[14], 5);
    }
}
