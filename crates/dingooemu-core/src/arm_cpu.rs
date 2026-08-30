use crate::error::{Result, SimulatorError};

const N: u32 = 1 << 31;
const Z: u32 = 1 << 30;
const C: u32 = 1 << 29;
const V: u32 = 1 << 28;
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

    pub fn step<B: ArmBus>(&mut self, bus: &mut B) -> Result<()> {
        if !self.running {
            return Ok(());
        }
        let pc = self.r[15];
        match self.state {
            ArmExecutionState::Arm => {
                let instruction = bus.fetch32(pc)?;
                self.r[15] = pc.wrapping_add(4);
                self.execute_arm(instruction, pc, bus)?;
            }
            ArmExecutionState::Thumb => {
                let instruction = bus.fetch16(pc)?;
                self.r[15] = pc.wrapping_add(2);
                self.execute_thumb(instruction, pc, bus)?;
            }
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
            self.write_reg(rd, value);
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
        if instruction & (1 << 20) != 0 {
            let value = match kind {
                1 => u32::from(bus.read16(address)?),
                2 => bus.read8(address)? as i8 as i32 as u32,
                3 => bus.read16(address)? as i16 as i32 as u32,
                _ => {
                    return Err(SimulatorError::InvalidInstruction {
                        pc,
                        instr: instruction,
                    })
                }
            };
            self.write_reg(rd, value);
        } else if kind == 1 {
            bus.write16(address, self.read_reg(rd, pc, true) as u16)?;
        } else {
            return Err(SimulatorError::InvalidInstruction {
                pc,
                instr: instruction,
            });
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
                self.write_reg(reg, value);
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

    fn execute_thumb<B: ArmBus>(&mut self, instruction: u16, pc: u32, _bus: &mut B) -> Result<()> {
        Err(SimulatorError::InvalidInstruction {
            pc,
            instr: u32::from(instruction),
        })
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
        (0, 1..=31) => (value << amount, value >> (32 - amount) != 0),
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
        assert!(matches!(
            cpu.step(&mut bus),
            Err(SimulatorError::InvalidInstruction { .. })
        ));

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
}
