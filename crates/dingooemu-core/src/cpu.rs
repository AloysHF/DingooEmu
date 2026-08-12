use crate::error::{Result, SimulatorError};
use crate::memory::Memory;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum UnknownInstructionPolicy {
    Stop,
    #[default]
    Skip,
}

/// MIPS32 register file
#[repr(C)]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Cpu {
    /// Register file
    pub regs: Registers,
    /// Instruction count (for debugging/profiling)
    pub instruction_count: u64,
    /// Running state
    running: bool,
    /// Branch delay slot: true if we need to apply branch target after this instruction
    pub branch_delay: bool,
    /// Branch target address (set when branch is taken)
    pub branch_target: u32,
    /// Whether branch_delay was set in the CURRENT step (not from previous)
    branch_delay_pending: bool,
    unknown_instruction_policy: UnknownInstructionPolicy,
}

impl Cpu {
    /// Create a new CPU with the specified entry point
    pub fn new(entry_point: u32) -> Self {
        Self {
            regs: Registers::new(entry_point),
            instruction_count: 0,
            running: false,
            branch_delay: false,
            branch_target: 0,
            branch_delay_pending: false,
            unknown_instruction_policy: UnknownInstructionPolicy::default(),
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

    pub fn set_unknown_instruction_policy(&mut self, policy: UnknownInstructionPolicy) {
        self.unknown_instruction_policy = policy;
    }

    fn handle_unknown_instruction(&self, instr: u32) -> Result<()> {
        let pc = self.regs.pc.wrapping_sub(4);
        match self.unknown_instruction_policy {
            UnknownInstructionPolicy::Stop => Err(SimulatorError::InvalidInstruction { pc, instr }),
            UnknownInstructionPolicy::Skip => {
                log::warn!("Skipping unimplemented instruction {instr:#010x} at PC={pc:#010x}");
                Ok(())
            }
        }
    }

    /// Execute one instruction
    pub fn step(&mut self, memory: &mut Memory) -> Result<()> {
        if !self.running {
            return Ok(());
        }

        let instr = memory.fetch_instruction(self.regs.pc)?;
        self.step_fetched(instr, memory)
    }

    /// Execute an instruction that was fetched by the runtime.
    pub(crate) fn step_fetched(&mut self, instr: u32, memory: &mut Memory) -> Result<()> {
        if self.step_fetched_unaccounted(instr, memory)? {
            self.instruction_count += 1;
        }
        Ok(())
    }

    /// Execute a fetched instruction without updating the profiling counter.
    pub(crate) fn step_fetched_unaccounted(
        &mut self,
        instr: u32,
        memory: &mut Memory,
    ) -> Result<bool> {
        if !self.running {
            return Ok(false);
        }
        // If we have a pending branch (from previous instruction),
        // the delay slot is the NEXT instruction to execute
        if self.branch_delay {
            // The delay slot is at PC (which is already pointing to the delay slot)
            // Execute it first
            self.regs.pc = self.regs.pc.wrapping_add(4);
            self.execute_instruction(instr, memory)?;

            // After delay slot executes, apply the branch target
            self.branch_delay = false;
            self.regs.pc = self.branch_target;
            self.regs.gpr[0] = 0;
            return Ok(true);
        }

        // Normal instruction execution
        self.regs.pc = self.regs.pc.wrapping_add(4);
        self.execute_instruction(instr, memory)?;

        // R0 is always zero
        self.regs.gpr[0] = 0;

        Ok(true)
    }

    pub(crate) fn account_instructions(&mut self, count: u64) {
        self.instruction_count += count;
    }

    /// Take a branch (sets delay slot)
    fn branch(&mut self, offset: i32) {
        // Calculate branch target: PC + (offset << 2)
        // Note: PC has already been incremented by 4 in step()
        let target = self.regs.pc.wrapping_add((offset << 2) as u32);
        self.branch_delay = true;
        self.branch_target = target;
        self.branch_delay_pending = true; // Mark that this branch was set in current step
    }

    /// Execute a single instruction
    fn execute_instruction(&mut self, instr: u32, memory: &mut Memory) -> Result<()> {
        if instr == 0 {
            return Ok(());
        }
        // Extract opcode (bits 31-26)
        let opcode = (instr >> 26) & 0x3F;

        match opcode {
            0x00 => self.execute_special(instr, memory),  // R-type
            0x01 => self.execute_regimm(instr),           // REGIMM
            0x02 => self.execute_j(instr),                // J
            0x03 => self.execute_jal(instr),              // JAL
            0x04 => self.execute_beq(instr),              // BEQ
            0x05 => self.execute_bne(instr),              // BNE
            0x06 => self.execute_blez(instr),             // BLEZ
            0x07 => self.execute_bgtz(instr),             // BGTZ
            0x08 => self.execute_addi(instr),             // ADDI
            0x09 => self.execute_addiu(instr),            // ADDIU
            0x0A => self.execute_slti(instr),             // SLTI
            0x0B => self.execute_sltiu(instr),            // SLTIU
            0x0C => self.execute_andi(instr),             // ANDI
            0x0D => self.execute_ori(instr),              // ORI
            0x0E => self.execute_xori(instr),             // XORI
            0x0F => self.execute_lui(instr),              // LUI
            0x1C => self.execute_special2(instr, memory), // SPECIAL2
            0x20 => self.execute_lb(instr, memory),       // LB
            0x21 => self.execute_lh(instr, memory),       // LH
            0x22 => self.execute_lwl(instr, memory),      // LWL
            0x23 => self.execute_lw(instr, memory),       // LW
            0x24 => self.execute_lbu(instr, memory),      // LBU
            0x25 => self.execute_lhu(instr, memory),      // LHU
            0x26 => self.execute_lwr(instr, memory),      // LWR
            0x28 => self.execute_sb(instr, memory),       // SB
            0x29 => self.execute_sh(instr, memory),       // SH
            0x2A => self.execute_swl(instr, memory),      // SWL
            0x2B => self.execute_sw(instr, memory),       // SW
            0x2E => self.execute_swr(instr, memory),      // SWR
            0x30 => self.execute_ll(instr, memory),       // LL
            0x33 => Ok(()),                               // PREF
            0x38 => self.execute_sc(instr, memory),       // SC
            _ => self.handle_unknown_instruction(instr),
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
            0x08 => {
                // JR - Jump Register
                let target = self.regs.read(rs);
                self.branch_delay = true;
                self.branch_target = target;
            }
            0x09 => {
                // JALR - Jump and Link Register
                let target = self.regs.read(rs);
                self.regs.write(rd, self.regs.pc.wrapping_add(4)); // Save return address
                self.branch_delay = true;
                self.branch_target = target;
            }
            0x0A => {
                // MOVZ - Move Conditional on Zero
                if self.regs.read(rt) == 0 {
                    self.regs.write(rd, self.regs.read(rs));
                }
            }
            0x0B => {
                // MOVN - Move Conditional on Not Zero
                if self.regs.read(rt) != 0 {
                    self.regs.write(rd, self.regs.read(rs));
                }
            }
            0x0C => {
                // SYSCALL
                // TODO: Implement syscall handling
                log::warn!("SYSCALL at PC={:#010x}", self.regs.pc.wrapping_sub(4));
            }
            0x0D => {
                // BREAK
                log::warn!("BREAK at PC={:#010x}", self.regs.pc.wrapping_sub(4));
                self.running = false;
            }
            0x0F => {
                // SYNC
                // No-op in interpreter
            }
            0x10 => self.regs.write(rd, self.regs.hi), // MFHI
            0x11 => self.regs.hi = self.regs.read(rs), // MTHI
            0x12 => self.regs.write(rd, self.regs.lo), // MFLO
            0x13 => self.regs.lo = self.regs.read(rs), // MTLO
            0x18 => {
                // MULT
                let a = self.regs.read(rs) as i32 as i64;
                let b = self.regs.read(rt) as i32 as i64;
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
            0x34 => {
                // TEQ
                if self.regs.read(rs) == self.regs.read(rt) {
                    return Err(SimulatorError::CpuError {
                        pc: self.regs.pc.wrapping_sub(4),
                        message: "trap on equal".to_string(),
                    });
                }
            }
            _ => return self.handle_unknown_instruction(instr),
        }
        Ok(())
    }

    /// Execute J-type instruction
    fn execute_j(&mut self, instr: u32) -> Result<()> {
        let target = instr & 0x03FF_FFFF;
        let pc = self.regs.pc.wrapping_sub(4); // Get branch instruction address
        let jump_target = (pc & 0xF000_0000) | (target << 2);
        self.branch_delay = true;
        self.branch_target = jump_target;
        Ok(())
    }

    /// Execute JAL-type instruction
    fn execute_jal(&mut self, instr: u32) -> Result<()> {
        let target = instr & 0x03FF_FFFF;
        let pc = self.regs.pc.wrapping_sub(4); // Get branch instruction address
        let jump_target = (pc & 0xF000_0000) | (target << 2);
        self.regs.write(31, self.regs.pc.wrapping_add(4)); // Save return address
        self.branch_delay = true;
        self.branch_target = jump_target;
        Ok(())
    }

    /// Execute BEQ instruction
    fn execute_beq(&mut self, instr: u32) -> Result<()> {
        let rs = ((instr >> 21) & 0x1F) as usize;
        let rt = ((instr >> 16) & 0x1F) as usize;
        let offset = instr as i16 as i32;
        if self.regs.read(rs) == self.regs.read(rt) {
            self.branch(offset);
        }
        Ok(())
    }

    /// Execute BNE instruction
    fn execute_bne(&mut self, instr: u32) -> Result<()> {
        let rs = ((instr >> 21) & 0x1F) as usize;
        let rt = ((instr >> 16) & 0x1F) as usize;
        let offset = instr as i16 as i32;
        if self.regs.read(rs) != self.regs.read(rt) {
            self.branch(offset);
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

    /// Execute REGIMM instructions (opcode = 0x01)
    fn execute_regimm(&mut self, instr: u32) -> Result<()> {
        let rt = ((instr >> 16) & 0x1F) as usize;
        let rs = ((instr >> 21) & 0x1F) as usize;
        let offset = instr as i16 as i32;

        match rt {
            0x00 => {
                // BLTZ - Branch on Less Than Zero
                if (self.regs.read(rs) as i32) < 0 {
                    self.branch(offset);
                }
            }
            0x01 => {
                // BGEZ - Branch on Greater Than or Equal to Zero
                if (self.regs.read(rs) as i32) >= 0 {
                    self.branch(offset);
                }
            }
            0x10 => {
                // BLTZAL - Branch on Less Than Zero and Link
                self.regs.write(31, self.regs.pc.wrapping_add(4));
                if (self.regs.read(rs) as i32) < 0 {
                    self.branch(offset);
                }
            }
            0x11 => {
                // BGEZAL - Branch on Greater Than or Equal to Zero and Link
                self.regs.write(31, self.regs.pc.wrapping_add(4));
                if (self.regs.read(rs) as i32) >= 0 {
                    self.branch(offset);
                }
            }
            _ => return self.handle_unknown_instruction(instr),
        }
        Ok(())
    }

    /// Execute BLEZ instruction
    fn execute_blez(&mut self, instr: u32) -> Result<()> {
        let rs = ((instr >> 21) & 0x1F) as usize;
        let offset = instr as i16 as i32;
        if (self.regs.read(rs) as i32) <= 0 {
            self.branch(offset);
        }
        Ok(())
    }

    /// Execute BGTZ instruction
    fn execute_bgtz(&mut self, instr: u32) -> Result<()> {
        let rs = ((instr >> 21) & 0x1F) as usize;
        let offset = instr as i16 as i32;
        if (self.regs.read(rs) as i32) > 0 {
            self.branch(offset);
        }
        Ok(())
    }

    /// Execute LWL instruction (Load Word Left)
    fn execute_lwl(&mut self, instr: u32, memory: &mut Memory) -> Result<()> {
        let rs = ((instr >> 21) & 0x1F) as usize;
        let rt = ((instr >> 16) & 0x1F) as usize;
        let offset = instr as i16 as i32;
        let addr = self.regs.read(rs).wrapping_add(offset as u32);
        let aligned_addr = addr & !3;
        let byte = addr & 3;

        let mem_val = memory.read_u32(aligned_addr)?;
        let rt_val = self.regs.read(rt);

        // Combine bytes based on alignment
        let result = match byte {
            0 => (mem_val << 24) | (rt_val & 0x00FFFFFF),
            1 => (mem_val << 16) | (rt_val & 0x0000FFFF),
            2 => (mem_val << 8) | (rt_val & 0x000000FF),
            3 => mem_val,
            _ => unreachable!(),
        };

        self.regs.write(rt, result);
        Ok(())
    }

    /// Execute LWR instruction (Load Word Right)
    fn execute_lwr(&mut self, instr: u32, memory: &mut Memory) -> Result<()> {
        let rs = ((instr >> 21) & 0x1F) as usize;
        let rt = ((instr >> 16) & 0x1F) as usize;
        let offset = instr as i16 as i32;
        let addr = self.regs.read(rs).wrapping_add(offset as u32);
        let aligned_addr = addr & !3;
        let byte = addr & 3;

        let mem_val = memory.read_u32(aligned_addr)?;
        let rt_val = self.regs.read(rt);

        // Combine bytes based on alignment
        let result = match byte {
            0 => mem_val,
            1 => (rt_val & 0xFF000000) | (mem_val >> 8),
            2 => (rt_val & 0xFFFF0000) | (mem_val >> 16),
            3 => (rt_val & 0xFFFFFF00) | (mem_val >> 24),
            _ => unreachable!(),
        };

        self.regs.write(rt, result);
        Ok(())
    }

    /// Execute SWL instruction (Store Word Left)
    fn execute_swl(&mut self, instr: u32, memory: &mut Memory) -> Result<()> {
        let rs = ((instr >> 21) & 0x1F) as usize;
        let rt = ((instr >> 16) & 0x1F) as usize;
        let offset = instr as i16 as i32;
        let addr = self.regs.read(rs).wrapping_add(offset as u32);
        let aligned_addr = addr & !3;
        let byte = addr & 3;

        let mem_val = memory.read_u32(aligned_addr)?;
        let rt_val = self.regs.read(rt);

        let result = match byte {
            0 => (mem_val & 0xFFFFFF00) | (rt_val >> 24),
            1 => (mem_val & 0xFFFF0000) | (rt_val >> 16),
            2 => (mem_val & 0xFF000000) | (rt_val >> 8),
            3 => rt_val,
            _ => unreachable!(),
        };

        memory.write_u32(aligned_addr, result)?;
        Ok(())
    }

    /// Execute SWR instruction (Store Word Right)
    fn execute_swr(&mut self, instr: u32, memory: &mut Memory) -> Result<()> {
        let rs = ((instr >> 21) & 0x1F) as usize;
        let rt = ((instr >> 16) & 0x1F) as usize;
        let offset = instr as i16 as i32;
        let addr = self.regs.read(rs).wrapping_add(offset as u32);
        let aligned_addr = addr & !3;
        let byte = addr & 3;

        let mem_val = memory.read_u32(aligned_addr)?;
        let rt_val = self.regs.read(rt);

        let result = match byte {
            0 => rt_val,
            1 => (mem_val & 0x000000FF) | (rt_val << 8),
            2 => (mem_val & 0x0000FFFF) | (rt_val << 16),
            3 => (mem_val & 0x00FFFFFF) | (rt_val << 24),
            _ => unreachable!(),
        };

        memory.write_u32(aligned_addr, result)?;
        Ok(())
    }

    /// Execute LL instruction (Load Linked).
    fn execute_ll(&mut self, instr: u32, memory: &mut Memory) -> Result<()> {
        let rs = ((instr >> 21) & 0x1F) as usize;
        let rt = ((instr >> 16) & 0x1F) as usize;
        let offset = instr as i16 as i32;
        let addr = self.regs.read(rs).wrapping_add(offset as u32);
        let value = memory.read_u32(addr)?;
        self.regs.write(rt, value);
        Ok(())
    }

    /// Execute SC instruction (Store Conditional).
    fn execute_sc(&mut self, instr: u32, memory: &mut Memory) -> Result<()> {
        let rs = ((instr >> 21) & 0x1F) as usize;
        let rt = ((instr >> 16) & 0x1F) as usize;
        let offset = instr as i16 as i32;
        let addr = self.regs.read(rs).wrapping_add(offset as u32);
        let value = self.regs.read(rt);

        // Guest contexts execute serially, so model the reservation as
        // uncontended and report a successful conditional store.
        memory.write_u32(addr, value)?;
        self.regs.write(rt, 1);
        Ok(())
    }

    /// Execute SPECIAL2 instructions (opcode = 0x1c)
    fn execute_special2(&mut self, instr: u32, _memory: &mut Memory) -> Result<()> {
        let funct = instr & 0x3F;
        let rs = ((instr >> 21) & 0x1F) as usize;
        let rt = ((instr >> 16) & 0x1F) as usize;
        let rd = ((instr >> 11) & 0x1F) as usize;

        match funct {
            0x00 => {
                // MADD - Multiply and Add
                let a = self.regs.read(rs) as i64;
                let b = self.regs.read(rt) as i64;
                let acc = ((self.regs.hi as i64) << 32) | (self.regs.lo as i64);
                let result = acc.wrapping_add(a * b);
                self.regs.hi = (result >> 32) as u32;
                self.regs.lo = result as u32;
            }
            0x01 => {
                // MADDU - Multiply and Add Unsigned
                let a = self.regs.read(rs) as u64;
                let b = self.regs.read(rt) as u64;
                let acc = ((self.regs.hi as u64) << 32) | (self.regs.lo as u64);
                let result = acc.wrapping_add(a * b);
                self.regs.hi = (result >> 32) as u32;
                self.regs.lo = result as u32;
            }
            0x02 => {
                // MUL - Multiply (result in Rd)
                let a = self.regs.read(rs) as i32;
                let b = self.regs.read(rt) as i32;
                let result = a.wrapping_mul(b);
                self.regs.write(rd, result as u32);
            }
            0x04 => {
                // MSUB - Multiply and Subtract
                let a = self.regs.read(rs) as i64;
                let b = self.regs.read(rt) as i64;
                let acc = ((self.regs.hi as i64) << 32) | (self.regs.lo as i64);
                let result = acc.wrapping_sub(a * b);
                self.regs.hi = (result >> 32) as u32;
                self.regs.lo = result as u32;
            }
            0x05 => {
                // MSUBU - Multiply and Subtract Unsigned
                let a = self.regs.read(rs) as u64;
                let b = self.regs.read(rt) as u64;
                let acc = ((self.regs.hi as u64) << 32) | (self.regs.lo as u64);
                let result = acc.wrapping_sub(a * b);
                self.regs.hi = (result >> 32) as u32;
                self.regs.lo = result as u32;
            }
            0x20 => {
                // CLZ - Count Leading Zeros
                let val = self.regs.read(rs);
                let count = val.leading_zeros();
                self.regs.write(rd, count);
            }
            0x21 => {
                // CLO - Count Leading Ones
                let val = self.regs.read(rs);
                let count = (!val).leading_zeros();
                self.regs.write(rd, count);
            }
            _ => return self.handle_unknown_instruction(instr),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_instruction_policy_can_stop_or_skip() {
        let instruction = 0xfc00_0000;
        let mut memory = Memory::new();
        memory.write_u32(0, instruction).unwrap();

        let mut skip_cpu = Cpu::new(0);
        skip_cpu.start();
        assert!(skip_cpu.step(&mut memory).is_ok());
        assert_eq!(skip_cpu.regs.pc, 4);

        let mut stop_cpu = Cpu::new(0);
        stop_cpu.set_unknown_instruction_policy(UnknownInstructionPolicy::Stop);
        stop_cpu.start();
        assert!(matches!(
            stop_cpu.step(&mut memory),
            Err(SimulatorError::InvalidInstruction { pc: 0, instr }) if instr == instruction
        ));
    }

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
        let instr = (0x09 << 26) | (8 << 16) | 0x1234;
        mem.write_u32(0, instr).unwrap();
        cpu.start();
        cpu.step(&mut mem).unwrap();
        assert_eq!(cpu.regs.read(8), 0x1234);
    }

    #[test]
    fn test_nop_advances_pc_and_instruction_count() {
        let mut cpu = Cpu::new(0);
        let mut mem = Memory::new();
        cpu.start();

        cpu.step(&mut mem).unwrap();

        assert_eq!(cpu.regs.pc, 4);
        assert_eq!(cpu.instruction_count, 1);
    }

    #[test]
    fn test_jal_sets_ra_after_delay_slot() {
        let mut cpu = Cpu::new(0);
        let mut mem = Memory::new();

        // JAL 0x10
        let jal = (0x03 << 26) | 0x04;
        mem.write_u32(0, jal).unwrap();
        mem.write_u32(4, 0).unwrap(); // delay slot
        mem.write_u32(0x10, 0).unwrap();

        cpu.start();
        cpu.step(&mut mem).unwrap();
        assert_eq!(cpu.regs.read(31), 8);

        cpu.step(&mut mem).unwrap();
        assert_eq!(cpu.regs.pc, 0x10);
    }

    #[test]
    fn test_lwl_lwr_loads_unaligned_little_endian_word() {
        let mut cpu = Cpu::new(0);
        let mut mem = Memory::new();
        mem.load_data(0x100, &[0x11, 0x22, 0x33, 0x44, 0x55])
            .unwrap();

        let lwl = (0x22 << 26) | (4 << 21) | (8 << 16) | 3;
        let lwr = (0x26 << 26) | (4 << 21) | (8 << 16);
        mem.write_u32(0, lwl).unwrap();
        mem.write_u32(4, lwr).unwrap();
        cpu.regs.write(4, 0x101);
        cpu.regs.write(8, 0xDEAD_BEEF);
        cpu.start();

        cpu.step(&mut mem).unwrap();
        cpu.step(&mut mem).unwrap();

        assert_eq!(cpu.regs.read(8), 0x5544_3322);
    }

    #[test]
    fn test_swl_swr_stores_unaligned_little_endian_word() {
        let mut cpu = Cpu::new(0);
        let mut mem = Memory::new();
        mem.load_data(0x100, &[0x11, 0x22, 0x33, 0x44, 0x55])
            .unwrap();

        let swl = (0x2A << 26) | (4 << 21) | (8 << 16) | 3;
        let swr = (0x2E << 26) | (4 << 21) | (8 << 16);
        mem.write_u32(0, swl).unwrap();
        mem.write_u32(4, swr).unwrap();
        cpu.regs.write(4, 0x101);
        cpu.regs.write(8, 0xA1B2_C3D4);
        cpu.start();

        cpu.step(&mut mem).unwrap();
        cpu.step(&mut mem).unwrap();

        assert_eq!(
            (0..5)
                .map(|offset| mem.read_u8(0x100 + offset).unwrap())
                .collect::<Vec<_>>(),
            vec![0x11, 0xD4, 0xC3, 0xB2, 0xA1]
        );
    }

    #[test]
    fn test_ll_sc_updates_memory_and_reports_success() {
        let mut cpu = Cpu::new(0);
        let mut mem = Memory::new();
        mem.write_u32(0x100, 41).unwrap();

        let ll = (0x30 << 26) | (4 << 21) | (2 << 16);
        let addiu = (0x09 << 26) | (2 << 21) | (2 << 16) | 1;
        let sc = (0x38 << 26) | (4 << 21) | (2 << 16);
        mem.write_u32(0, ll).unwrap();
        mem.write_u32(4, addiu).unwrap();
        mem.write_u32(8, sc).unwrap();
        cpu.regs.write(4, 0x100);
        cpu.start();

        cpu.step(&mut mem).unwrap();
        cpu.step(&mut mem).unwrap();
        cpu.step(&mut mem).unwrap();

        assert_eq!(mem.read_u32(0x100).unwrap(), 42);
        assert_eq!(cpu.regs.read(2), 1);
    }

    #[test]
    fn test_teq_only_traps_equal_operands() {
        let teq = (4 << 21) | (5 << 16) | 0x34;
        let mut mem = Memory::new();
        mem.write_u32(0, teq).unwrap();

        let mut unequal = Cpu::new(0);
        unequal.regs.write(4, 1);
        unequal.regs.write(5, 2);
        unequal.start();
        assert!(unequal.step(&mut mem).is_ok());

        let mut equal = Cpu::new(0);
        equal.regs.write(4, 1);
        equal.regs.write(5, 1);
        equal.start();
        assert!(matches!(
            equal.step(&mut mem),
            Err(SimulatorError::CpuError { pc: 0, .. })
        ));
    }

    #[test]
    fn test_mult_sign_extends_negative_operands() {
        let mut cpu = Cpu::new(0);
        let mut mem = Memory::new();
        cpu.regs.write(8, (-7_i32) as u32);
        cpu.regs.write(9, 3);
        mem.write_u32(0, (8 << 21) | (9 << 16) | 0x18).unwrap();
        cpu.start();

        cpu.step(&mut mem).unwrap();

        assert_eq!(cpu.regs.hi, u32::MAX);
        assert_eq!(cpu.regs.lo, (-21_i32) as u32);
    }

    #[test]
    fn test_lui() {
        let mut cpu = Cpu::new(0);
        let mut mem = Memory::new();
        // LUI $t0, 0xABCD
        // opcode=0x0F, rs=0, rt=8, imm=0xABCD
        let instr = (0x0F << 26) | (8 << 16) | 0xABCD;
        mem.write_u32(0, instr).unwrap();
        cpu.start();
        cpu.step(&mut mem).unwrap();
        assert_eq!(cpu.regs.read(8), 0xABCD_0000);
    }

    #[test]
    fn test_pref_is_ignored() {
        let mut cpu = Cpu::new(0);
        let mut mem = Memory::new();
        let pref = (0x33 << 26) | (4 << 21) | (7 << 16) | 0x1234;
        mem.write_u32(0, pref).unwrap();
        cpu.regs.write(4, 0x1000);
        cpu.start();

        cpu.step(&mut mem).unwrap();

        assert_eq!(cpu.regs.pc, 4);
        assert_eq!(cpu.regs.read(4), 0x1000);
        assert_eq!(cpu.instruction_count, 1);
    }
}
