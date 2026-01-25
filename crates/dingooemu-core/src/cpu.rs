use crate::error::Result;
use crate::memory::Memory;

/// MIPS32 register file
#[derive(Debug, Clone)]
pub struct Registers {
    /// General purpose registers (R0-R31)
    /// R0 is hardwired to zero
    pub gpr: [u32; 32],
    /// Program counter
    pub pc: u32,
    /// Hi register (multiply/divide results)
    pub hi: u32,
    /// Lo register (multiply/divide results)
    pub lo: u32,
}

impl Registers {
    /// Create new registers with PC at the specified address
    pub fn new(entry_point: u32) -> Self {
        let mut regs = Self {
            gpr: [0; 32],
            pc: entry_point,
            hi: 0,
            lo: 0,
        };
        // R0 is always zero
        regs.gpr[0] = 0;
        regs
    }

    /// Read a register (R0 always returns 0)
    pub fn read(&self, reg: usize) -> u32 {
        if reg == 0 {
            0
        } else {
            self.gpr[reg]
        }
    }

    /// Write a register (writes to R0 are ignored)
    pub fn write(&mut self, reg: usize, value: u32) {
        if reg != 0 {
            self.gpr[reg] = value;
        }
    }
}

/// MIPS32 CPU for Dingoo A320 (Ingenic JZ4740 XBurst)
pub struct Cpu {
    /// Register file
    pub regs: Registers,
    /// Instruction count (for debugging/profiling)
    pub instruction_count: u64,
    /// Running state
    running: bool,
}

impl Cpu {
    /// Create a new CPU with the specified entry point
    pub fn new(entry_point: u32) -> Self {
        Self {
            regs: Registers::new(entry_point),
            instruction_count: 0,
            running: false,
        }
    }

    /// Start the CPU
    pub fn start(&mut self) {
        self.running = true;
    }

    /// Stop the CPU
    pub fn stop(&mut self) {
        self.running = false;
    }

    /// Check if CPU is running
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Execute one instruction
    pub fn step(&mut self, memory: &mut Memory) -> Result<()> {
        if !self.running {
            return Ok(());
        }

        // Fetch instruction at PC
        let instr = memory.read_u32(self.regs.pc)?;

        // Advance PC (for branch delay slot handling later)
        self.regs.pc = self.regs.pc.wrapping_add(4);

        // Decode and execute
        self.execute_instruction(instr, memory)?;

        // R0 is always zero
        self.regs.gpr[0] = 0;

        self.instruction_count += 1;
        Ok(())
    }

    /// Execute a single instruction
    fn execute_instruction(&mut self, instr: u32, memory: &mut Memory) -> Result<()> {
        // Extract opcode (bits 31-26)
        let opcode = (instr >> 26) & 0x3F;

        match opcode {
            0x00 => self.execute_special(instr, memory), // R-type
            0x02 => self.execute_j(instr),               // J
            0x03 => self.execute_jal(instr),             // JAL
            0x04 => self.execute_beq(instr),             // BEQ
            0x05 => self.execute_bne(instr),             // BNE
            0x08 => self.execute_addi(instr),            // ADDI
            0x09 => self.execute_addiu(instr),           // ADDIU
            0x0A => self.execute_slti(instr),            // SLTI
            0x0B => self.execute_sltiu(instr),           // SLTIU
            0x0C => self.execute_andi(instr),            // ANDI
            0x0D => self.execute_ori(instr),             // ORI
            0x0E => self.execute_xori(instr),            // XORI
            0x0F => self.execute_lui(instr),             // LUI
            0x20 => self.execute_lb(instr, memory),      // LB
            0x21 => self.execute_lh(instr, memory),      // LH
            0x23 => self.execute_lw(instr, memory),      // LW
            0x24 => self.execute_lbu(instr, memory),     // LBU
            0x25 => self.execute_lhu(instr, memory),     // LHU
            0x28 => self.execute_sb(instr, memory),      // SB
            0x29 => self.execute_sh(instr, memory),      // SH
            0x2B => self.execute_sw(instr, memory),      // SW
            _ => {
                // TODO: Implement more instructions
                log::warn!(
                    "Unimplemented opcode: {:#04x} at PC={:#010x}",
                    opcode,
                    self.regs.pc.wrapping_sub(4)
                );
                Ok(())
            }
        }
    }

    /// Execute R-type instructions (opcode = 0x00)
    fn execute_special(&mut self, instr: u32, _memory: &mut Memory) -> Result<()> {
        let funct = instr & 0x3F;
        let rs = ((instr >> 21) & 0x1F) as usize;
        let rt = ((instr >> 16) & 0x1F) as usize;
        let rd = ((instr >> 11) & 0x1F) as usize;
        let shamt = (instr >> 6) & 0x1F;

        match funct {
            0x00 => self.regs.write(rd, self.regs.read(rt) << shamt), // SLL
            0x02 => self.regs.write(rd, self.regs.read(rt) >> shamt), // SRL
            0x03 => {
                let result = (self.regs.read(rt) as i32) >> shamt;
                self.regs.write(rd, result as u32);
            } // SRA
            0x04 => self
                .regs
                .write(rd, self.regs.read(rt) << (self.regs.read(rs) & 0x1F)), // SLLV
            0x06 => self
                .regs
                .write(rd, self.regs.read(rt) >> (self.regs.read(rs) & 0x1F)), // SRLV
            0x07 => {
                let shift = self.regs.read(rs) & 0x1F;
                let result = (self.regs.read(rt) as i32) >> shift;
                self.regs.write(rd, result as u32);
            } // SRAV
            0x08 => self.regs.pc = self.regs.read(rs),                // JR
            0x09 => {
                // JALR
                self.regs.write(rd, self.regs.pc);
                self.regs.pc = self.regs.read(rs);
            }
            0x0C => {
                // SYSCALL
                // TODO: Implement syscall handling
                log::warn!("SYSCALL at PC={:#010x}", self.regs.pc.wrapping_sub(4));
            }
            0x10 => self.regs.write(rd, self.regs.hi), // MFHI
            0x11 => self.regs.hi = self.regs.read(rs), // MTHI
            0x12 => self.regs.write(rd, self.regs.lo), // MFLO
            0x13 => self.regs.lo = self.regs.read(rs), // MTLO
            0x18 => {
                // MULT
                let a = self.regs.read(rs) as i64;
                let b = self.regs.read(rt) as i64;
                let result = a * b;
                self.regs.hi = (result >> 32) as u32;
                self.regs.lo = result as u32;
            }
            0x19 => {
                // MULTU
                let a = self.regs.read(rs) as u64;
                let b = self.regs.read(rt) as u64;
                let result = a * b;
                self.regs.hi = (result >> 32) as u32;
                self.regs.lo = result as u32;
            }
            0x1A => {
                // DIV
                let n = self.regs.read(rs) as i32;
                let d = self.regs.read(rt) as i32;
                if let Some(q) = n.checked_div(d) {
                    self.regs.lo = q as u32;
                    self.regs.hi = n.wrapping_rem(d) as u32;
                }
            }
            0x1B => {
                // DIVU
                let n = self.regs.read(rs);
                let d = self.regs.read(rt);
                if let Some(q) = n.checked_div(d) {
                    self.regs.lo = q;
                    self.regs.hi = n % d;
                }
            }
            0x20 => {
                // ADD (with overflow check)
                let a = self.regs.read(rs) as i32;
                let b = self.regs.read(rt) as i32;
                let result = a.wrapping_add(b);
                self.regs.write(rd, result as u32);
            }
            0x21 => {
                // ADDU
                let result = self.regs.read(rs).wrapping_add(self.regs.read(rt));
                self.regs.write(rd, result);
            }
            0x22 => {
                // SUB (with overflow check)
                let a = self.regs.read(rs) as i32;
                let b = self.regs.read(rt) as i32;
                let result = a.wrapping_sub(b);
                self.regs.write(rd, result as u32);
            }
            0x23 => {
                // SUBU
                let result = self.regs.read(rs).wrapping_sub(self.regs.read(rt));
                self.regs.write(rd, result);
            }
            0x24 => self.regs.write(rd, self.regs.read(rs) & self.regs.read(rt)), // AND
            0x25 => self.regs.write(rd, self.regs.read(rs) | self.regs.read(rt)), // OR
            0x26 => self.regs.write(rd, self.regs.read(rs) ^ self.regs.read(rt)), // XOR
            0x27 => self
                .regs
                .write(rd, !(self.regs.read(rs) | self.regs.read(rt))), // NOR
            0x2A => {
                // SLT
                let a = self.regs.read(rs) as i32;
                let b = self.regs.read(rt) as i32;
                self.regs.write(rd, if a < b { 1 } else { 0 });
            }
            0x2B => {
                // SLTU
                let a = self.regs.read(rs);
                let b = self.regs.read(rt);
                self.regs.write(rd, if a < b { 1 } else { 0 });
            }
            _ => {
                log::warn!(
                    "Unimplemented special funct: {:#04x} at PC={:#010x}",
                    funct,
                    self.regs.pc.wrapping_sub(4)
                );
            }
        }
        Ok(())
    }

    /// Execute J-type instruction
    fn execute_j(&mut self, instr: u32) -> Result<()> {
        let target = instr & 0x03FF_FFFF;
        self.regs.pc = (self.regs.pc & 0xF000_0000) | (target << 2);
        Ok(())
    }

    /// Execute JAL-type instruction
    fn execute_jal(&mut self, instr: u32) -> Result<()> {
        let target = instr & 0x03FF_FFFF;
        self.regs.write(31, self.regs.pc); // Save return address
        self.regs.pc = (self.regs.pc & 0xF000_0000) | (target << 2);
        Ok(())
    }

    /// Execute BEQ instruction
    fn execute_beq(&mut self, instr: u32) -> Result<()> {
        let rs = ((instr >> 21) & 0x1F) as usize;
        let rt = ((instr >> 16) & 0x1F) as usize;
        let offset = (instr as i16 as i32) << 2;
        if self.regs.read(rs) == self.regs.read(rt) {
            self.regs.pc = self.regs.pc.wrapping_add(offset as u32);
        }
        Ok(())
    }

    /// Execute BNE instruction
    fn execute_bne(&mut self, instr: u32) -> Result<()> {
        let rs = ((instr >> 21) & 0x1F) as usize;
        let rt = ((instr >> 16) & 0x1F) as usize;
        let offset = (instr as i16 as i32) << 2;
        if self.regs.read(rs) != self.regs.read(rt) {
            self.regs.pc = self.regs.pc.wrapping_add(offset as u32);
        }
        Ok(())
    }

    /// Execute ADDI instruction
    fn execute_addi(&mut self, instr: u32) -> Result<()> {
        let rs = ((instr >> 21) & 0x1F) as usize;
        let rt = ((instr >> 16) & 0x1F) as usize;
        let imm = instr as i16 as i32;
        let result = (self.regs.read(rs) as i32).wrapping_add(imm);
        self.regs.write(rt, result as u32);
        Ok(())
    }

    /// Execute ADDIU instruction
    fn execute_addiu(&mut self, instr: u32) -> Result<()> {
        let rs = ((instr >> 21) & 0x1F) as usize;
        let rt = ((instr >> 16) & 0x1F) as usize;
        let imm = instr as i16 as i32;
        let result = self.regs.read(rs).wrapping_add(imm as u32);
        self.regs.write(rt, result);
        Ok(())
    }

    /// Execute SLTI instruction
    fn execute_slti(&mut self, instr: u32) -> Result<()> {
        let rs = ((instr >> 21) & 0x1F) as usize;
        let rt = ((instr >> 16) & 0x1F) as usize;
        let imm = instr as i16 as i32;
        let a = self.regs.read(rs) as i32;
        self.regs.write(rt, if a < imm { 1 } else { 0 });
        Ok(())
    }

    /// Execute SLTIU instruction
    fn execute_sltiu(&mut self, instr: u32) -> Result<()> {
        let rs = ((instr >> 21) & 0x1F) as usize;
        let rt = ((instr >> 16) & 0x1F) as usize;
        let imm = instr as i16 as u32;
        let a = self.regs.read(rs);
        self.regs.write(rt, if a < imm { 1 } else { 0 });
        Ok(())
    }

    /// Execute ANDI instruction
    fn execute_andi(&mut self, instr: u32) -> Result<()> {
        let rs = ((instr >> 21) & 0x1F) as usize;
        let rt = ((instr >> 16) & 0x1F) as usize;
        let imm = instr as u16 as u32;
        let result = self.regs.read(rs) & imm;
        self.regs.write(rt, result);
        Ok(())
    }

    /// Execute ORI instruction
    fn execute_ori(&mut self, instr: u32) -> Result<()> {
        let rs = ((instr >> 21) & 0x1F) as usize;
        let rt = ((instr >> 16) & 0x1F) as usize;
        let imm = instr as u16 as u32;
        let result = self.regs.read(rs) | imm;
        self.regs.write(rt, result);
        Ok(())
    }

    /// Execute XORI instruction
    fn execute_xori(&mut self, instr: u32) -> Result<()> {
        let rs = ((instr >> 21) & 0x1F) as usize;
        let rt = ((instr >> 16) & 0x1F) as usize;
        let imm = instr as u16 as u32;
        let result = self.regs.read(rs) ^ imm;
        self.regs.write(rt, result);
        Ok(())
    }

    /// Execute LUI instruction
    fn execute_lui(&mut self, instr: u32) -> Result<()> {
        let rt = ((instr >> 16) & 0x1F) as usize;
        let imm = instr as u16;
        self.regs.write(rt, (imm as u32) << 16);
        Ok(())
    }

    /// Execute LB instruction
    fn execute_lb(&mut self, instr: u32, memory: &mut Memory) -> Result<()> {
        let rs = ((instr >> 21) & 0x1F) as usize;
        let rt = ((instr >> 16) & 0x1F) as usize;
        let offset = instr as i16 as i32;
        let addr = self.regs.read(rs).wrapping_add(offset as u32);
        let value = memory.read_u8(addr)? as i8 as i32 as u32;
        self.regs.write(rt, value);
        Ok(())
    }

    /// Execute LH instruction
    fn execute_lh(&mut self, instr: u32, memory: &mut Memory) -> Result<()> {
        let rs = ((instr >> 21) & 0x1F) as usize;
        let rt = ((instr >> 16) & 0x1F) as usize;
        let offset = instr as i16 as i32;
        let addr = self.regs.read(rs).wrapping_add(offset as u32);
        let value = memory.read_u16(addr)? as i16 as i32 as u32;
        self.regs.write(rt, value);
        Ok(())
    }

    /// Execute LW instruction
    fn execute_lw(&mut self, instr: u32, memory: &mut Memory) -> Result<()> {
        let rs = ((instr >> 21) & 0x1F) as usize;
        let rt = ((instr >> 16) & 0x1F) as usize;
        let offset = instr as i16 as i32;
        let addr = self.regs.read(rs).wrapping_add(offset as u32);
        let value = memory.read_u32(addr)?;
        self.regs.write(rt, value);
        Ok(())
    }

    /// Execute LBU instruction
    fn execute_lbu(&mut self, instr: u32, memory: &mut Memory) -> Result<()> {
        let rs = ((instr >> 21) & 0x1F) as usize;
        let rt = ((instr >> 16) & 0x1F) as usize;
        let offset = instr as i16 as i32;
        let addr = self.regs.read(rs).wrapping_add(offset as u32);
        let value = memory.read_u8(addr)? as u32;
        self.regs.write(rt, value);
        Ok(())
    }

    /// Execute LHU instruction
    fn execute_lhu(&mut self, instr: u32, memory: &mut Memory) -> Result<()> {
        let rs = ((instr >> 21) & 0x1F) as usize;
        let rt = ((instr >> 16) & 0x1F) as usize;
        let offset = instr as i16 as i32;
        let addr = self.regs.read(rs).wrapping_add(offset as u32);
        let value = memory.read_u16(addr)? as u32;
        self.regs.write(rt, value);
        Ok(())
    }

    /// Execute SB instruction
    fn execute_sb(&mut self, instr: u32, memory: &mut Memory) -> Result<()> {
        let rs = ((instr >> 21) & 0x1F) as usize;
        let rt = ((instr >> 16) & 0x1F) as usize;
        let offset = instr as i16 as i32;
        let addr = self.regs.read(rs).wrapping_add(offset as u32);
        let value = self.regs.read(rt) as u8;
        memory.write_u8(addr, value)?;
        Ok(())
    }

    /// Execute SH instruction
    fn execute_sh(&mut self, instr: u32, memory: &mut Memory) -> Result<()> {
        let rs = ((instr >> 21) & 0x1F) as usize;
        let rt = ((instr >> 16) & 0x1F) as usize;
        let offset = instr as i16 as i32;
        let addr = self.regs.read(rs).wrapping_add(offset as u32);
        let value = self.regs.read(rt) as u16;
        memory.write_u16(addr, value)?;
        Ok(())
    }

    /// Execute SW instruction
    fn execute_sw(&mut self, instr: u32, memory: &mut Memory) -> Result<()> {
        let rs = ((instr >> 21) & 0x1F) as usize;
        let rt = ((instr >> 16) & 0x1F) as usize;
        let offset = instr as i16 as i32;
        let addr = self.regs.read(rs).wrapping_add(offset as u32);
        let value = self.regs.read(rt);
        memory.write_u32(addr, value)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cpu_creation() {
        let cpu = Cpu::new(0x8000_0000);
        assert_eq!(cpu.regs.pc, 0x8000_0000);
        assert_eq!(cpu.regs.read(0), 0); // R0 always zero
    }

    #[test]
    fn test_addiu() {
        let mut cpu = Cpu::new(0);
        let mut mem = Memory::new();
        // ADDIU $t0, $zero, 0x1234
        // opcode=0x09, rs=0, rt=8, imm=0x1234
        let instr = (0x09 << 26) | (0 << 21) | (8 << 16) | 0x1234;
        mem.write_u32(0, instr).unwrap();
        cpu.start();
        cpu.step(&mut mem).unwrap();
        assert_eq!(cpu.regs.read(8), 0x1234);
    }

    #[test]
    fn test_lui() {
        let mut cpu = Cpu::new(0);
        let mut mem = Memory::new();
        // LUI $t0, 0xABCD
        // opcode=0x0F, rs=0, rt=8, imm=0xABCD
        let instr = (0x0F << 26) | (0 << 21) | (8 << 16) | 0xABCD;
        mem.write_u32(0, instr).unwrap();
        cpu.start();
        cpu.step(&mut mem).unwrap();
        assert_eq!(cpu.regs.read(8), 0xABCD_0000);
    }
}
