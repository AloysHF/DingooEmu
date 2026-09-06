mod audio;
mod display;
mod files;
mod input;
mod semihosting;
mod system;
mod tasks;

use super::*;

impl RuntimeBus<'_> {
    fn clear_instruction_cache(&mut self) {
        for block in &mut *self.instruction_blocks {
            block.len = 0;
        }
        self.instruction_cache_pages.fill(0);
        self.instruction_cache_invalidated = true;
    }

    fn block_page_range(&self, start: u32, len: u8) -> std::ops::RangeInclusive<usize> {
        let offset = start - self.package.load_base();
        let end_offset = offset + u32::from(len) * 4 - 1;
        (offset >> INSTRUCTION_CACHE_PAGE_SHIFT) as usize
            ..=(end_offset >> INSTRUCTION_CACHE_PAGE_SHIFT) as usize
    }

    fn remove_instruction_block(&mut self, cache_index: usize) {
        let block = &self.instruction_blocks[cache_index];
        if block.len == 0 {
            return;
        }
        let pages = self.block_page_range(block.start, block.len);
        self.instruction_blocks[cache_index].len = 0;
        for page in pages {
            self.instruction_cache_pages[page] -= 1;
        }
    }

    fn add_instruction_block_pages(&mut self, cache_index: usize) {
        let block = &self.instruction_blocks[cache_index];
        for page in self.block_page_range(block.start, block.len) {
            self.instruction_cache_pages[page] += 1;
        }
    }

    fn invalidate_code_write(&mut self, address: u32, size: usize) {
        let program_start = self.package.load_base();
        let program_end = program_start.saturating_add(self.package.program_size());
        let write_end = address.saturating_add(size as u32);
        let overlap_start = address.max(program_start);
        let overlap_end = write_end.min(program_end);
        if overlap_start >= overlap_end {
            return;
        }
        *self.code_generation = (*self.code_generation).wrapping_add(1);
        let first_page = ((overlap_start - program_start) >> INSTRUCTION_CACHE_PAGE_SHIFT) as usize;
        let last_page =
            ((overlap_end - 1 - program_start) >> INSTRUCTION_CACHE_PAGE_SHIFT) as usize;
        if self.instruction_cache_pages[first_page..=last_page]
            .iter()
            .all(|count| *count == 0)
        {
            return;
        }

        let mut invalidated = false;
        for cache_index in 0..self.instruction_blocks.len() {
            let block = &self.instruction_blocks[cache_index];
            if block.len == 0 {
                continue;
            }
            let block_end = block.start + u32::from(block.len) * 4;
            if block.start < overlap_end && block_end > overlap_start {
                self.remove_instruction_block(cache_index);
                invalidated = true;
            }
        }
        self.instruction_cache_invalidated |= invalidated;
    }

    fn write_memory(&mut self, address: u32, data: &[u8]) -> Result<()> {
        self.memory.write_bytes(address, data)?;
        self.invalidate_code_write(address, data.len());
        Ok(())
    }

    pub(super) fn execute_cached_block(
        &mut self,
        cpu: &mut Cpu,
        instruction_limit: usize,
        previous_pc: &mut u32,
        error_pc: &mut u32,
        #[cfg(feature = "jit")] jit: &mut JitEngine,
    ) -> Result<usize> {
        if cpu.execution_state() != ExecutionState::Arm {
            *error_pc = cpu.r[15];
            cpu.step(self)?;
            *previous_pc = *error_pc;
            return Ok(1);
        }

        let address = cpu.r[15];
        let Some(offset) = address.checked_sub(self.package.load_base()) else {
            *error_pc = address;
            cpu.step(self)?;
            *previous_pc = address;
            return Ok(1);
        };
        if offset & 3 != 0 || offset >= self.package.program_size() {
            *error_pc = address;
            cpu.step(self)?;
            *previous_pc = address;
            return Ok(1);
        }

        let cache_index = instruction_block_cache_index(address);
        if self.instruction_blocks[cache_index].len == 0
            || self.instruction_blocks[cache_index].start != address
        {
            let program_end = self
                .package
                .load_base()
                .saturating_add(self.package.program_size());
            let mut instructions = [DecodedArmInstruction::default(); MAX_INSTRUCTION_BLOCK_LEN];
            let mut count = 0usize;
            let mut current = address;
            while count < MAX_INSTRUCTION_BLOCK_LEN && current < program_end {
                instructions[count] = DecodedArmInstruction::decode(self.memory.read32(current)?);
                count += 1;
                current = current.wrapping_add(4);
            }
            self.remove_instruction_block(cache_index);
            self.instruction_blocks[cache_index] = CachedInstructionBlock {
                start: address,
                len: count as u8,
                instructions,
            };
            self.add_instruction_block_pages(cache_index);
        }

        self.instruction_cache_invalidated = false;
        let block_len = self.instruction_blocks[cache_index].len as usize;
        #[cfg(feature = "jit")]
        if let Some(completed) = jit.execute(
            address,
            *self.code_generation,
            &self.instruction_blocks[cache_index].instructions[..block_len],
            instruction_limit,
            &mut cpu.r,
            self.memory as *const Memory as *mut u8,
        ) {
            cpu.instruction_count = cpu.instruction_count.wrapping_add(completed as u64);
            *previous_pc = cpu.r[15].wrapping_sub(4);
            *error_pc = *previous_pc;
            return Ok(completed);
        }
        let mut completed = 0;
        for instruction_index in 0..block_len.min(instruction_limit) {
            let instruction = self.instruction_blocks[cache_index].instructions[instruction_index];
            let pc = cpu.r[15];
            *error_pc = pc;
            cpu.step_fetched_arm(instruction, self)?;
            *previous_pc = pc;
            completed += 1;
            if instruction.may_exit_block
                && (self.instruction_cache_invalidated
                    || self.event_pending
                    || !cpu.is_running()
                    || cpu.execution_state() != ExecutionState::Arm
                    || cpu.r[15] != pc.wrapping_add(4))
            {
                break;
            }
        }
        Ok(completed)
    }

    fn dispatch(&mut self, cpu: &mut Cpu, immediate: u32) -> Result<()> {
        if immediate == 0x0012_3456 {
            return self.dispatch_semihosting(cpu);
        }
        let (symbol_name, symbol_address) = if immediate & 0x0080_0000 != 0 {
            let index = (immediate & 0x007f_ffff) as usize;
            let name = self
                .dynamic_imports
                .get(index)
                .ok_or_else(|| SimulatorError::CpuError {
                    pc: cpu.r[15].wrapping_sub(4),
                    message: format!("dynamic ARM SVC index {index} is invalid"),
                })?;
            (name.clone(), DYNAMIC_THUNK_BASE + index as u32 * 8)
        } else {
            let symbol =
                self.imports
                    .get(immediate as usize)
                    .ok_or_else(|| SimulatorError::CpuError {
                        pc: cpu.r[15].wrapping_sub(4),
                        message: format!("ARM SVC index {immediate} is outside the import table"),
                    })?;
            (symbol.name.clone(), symbol.address)
        };
        let name = symbol_name.as_str();
        log::trace!(
            "ARM HLE {name}(r0={:#010x}, r1={:#010x}, r2={:#010x}, r3={:#010x})",
            cpu.r[0],
            cpu.r[1],
            cpu.r[2],
            cpu.r[3]
        );
        if !self.dispatch_display(cpu, name)?
            && !self.dispatch_system(cpu, name)?
            && !self.dispatch_files(cpu, name)?
            && !self.dispatch_input(cpu, name)?
            && !self.dispatch_audio(cpu, name)?
            && !self.dispatch_tasks(cpu, name)?
        {
            self.record_unknown(cpu, &symbol_name, symbol_address)?;
        }
        if self.profile == ArmProfile::Homebrew {
            cpu.r[15] = cpu.r[14] & !1;
        }
        Ok(())
    }
}

#[cfg(feature = "jit")]
pub(crate) unsafe extern "C" fn jit_read8(memory: *const u8, address: u32) -> u64 {
    // SAFETY: JIT calls provide the live Memory pointer owned by the runtime.
    let memory = unsafe { &*(memory.cast::<Memory>()) };
    memory
        .read8(address)
        .map_or(0, |value| (1_u64 << 32) | u64::from(value))
}

#[cfg(feature = "jit")]
pub(crate) unsafe extern "C" fn jit_read32(memory: *const u8, address: u32) -> u64 {
    // SAFETY: JIT calls provide the live Memory pointer owned by the runtime.
    let memory = unsafe { &*(memory.cast::<Memory>()) };
    memory
        .read32(address & !3)
        .map_or(0, |value| (1_u64 << 32) | u64::from(value))
}

impl RuntimeBus<'_> {
    fn record_unknown(&mut self, cpu: &mut Cpu, name: &str, import_address: u32) -> Result<()> {
        let pc = cpu.r[15].wrapping_sub(4);
        let call = self
            .unknown_hle_calls
            .entry(name.to_string())
            .or_insert_with(|| UnknownHleCall {
                name: name.to_string(),
                count: 0,
                import_address,
                first_pc: pc,
                first_arguments: [cpu.r[0], cpu.r[1], cpu.r[2], cpu.r[3]],
            });
        call.count += 1;
        if self.unknown_hle_policy == UnknownHlePolicy::Stop
            && !self.unknown_hle_allowlist.contains(name)
        {
            return Err(SimulatorError::UnknownHle {
                name: name.to_string(),
                pc,
                import_address,
                arguments: [cpu.r[0], cpu.r[1], cpu.r[2], cpu.r[3]],
            });
        }
        cpu.r[0] = 0;
        Ok(())
    }
}

impl Bus for RuntimeBus<'_> {
    fn read8(&mut self, address: u32) -> Result<u8> {
        self.memory.read8(address)
    }
    fn read16(&mut self, address: u32) -> Result<u16> {
        self.memory.read16(address)
    }
    fn read32(&mut self, address: u32) -> Result<u32> {
        self.memory.read32(address)
    }
    fn write8(&mut self, address: u32, value: u8) -> Result<()> {
        self.memory.write8(address, value)?;
        self.invalidate_code_write(address, 1);
        Ok(())
    }
    fn write16(&mut self, address: u32, value: u16) -> Result<()> {
        self.memory.write16(address, value)?;
        self.invalidate_code_write(address, 2);
        Ok(())
    }
    fn write32(&mut self, address: u32, value: u32) -> Result<()> {
        self.memory.write32(address, value)?;
        self.invalidate_code_write(address, 4);
        if address == LEGACY_GRAPHICS_STRIDE {
            match value {
                value if value == SCREEN_WIDTH * 2 => {
                    *self.framebuffer_bits = 16 | FRAMEBUFFER_BITS_EXPLICIT;
                }
                value if value == SCREEN_WIDTH * 4 => {
                    *self.framebuffer_bits = 32 | FRAMEBUFFER_BITS_EXPLICIT;
                }
                _ => {}
            }
        }
        if address == LEGACY_GRAPHICS_SURFACE {
            *self.active_framebuffer = LEGACY_FRAMEBUFFER_ADDRESS;
            *self.frame_address = Some(LEGACY_FRAMEBUFFER_ADDRESS);
            self.event_pending = true;
        }
        Ok(())
    }
    fn svc(&mut self, cpu: &mut Cpu, immediate: u32) -> Result<()> {
        self.event_pending = true;
        self.dispatch(cpu, immediate)
    }
}
