mod audio;
mod display;
mod files;
mod input;
mod semihosting;
mod system;
mod tasks;

use super::*;

impl RuntimeBus<'_> {
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
        self.memory.write8(address, value)
    }
    fn write16(&mut self, address: u32, value: u16) -> Result<()> {
        self.memory.write16(address, value)
    }
    fn write32(&mut self, address: u32, value: u32) -> Result<()> {
        self.memory.write32(address, value)?;
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
        }
        Ok(())
    }
    fn svc(&mut self, cpu: &mut Cpu, immediate: u32) -> Result<()> {
        self.dispatch(cpu, immediate)
    }
}
