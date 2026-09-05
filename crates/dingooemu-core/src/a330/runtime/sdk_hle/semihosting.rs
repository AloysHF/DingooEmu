use super::super::*;

impl RuntimeBus<'_> {
    pub(super) fn dispatch_semihosting(&mut self, cpu: &mut Cpu) -> Result<()> {
        match cpu.r[0] {
            0x03 => {
                let value = self.memory.read8(cpu.r[1])?;
                log::trace!("ARM semihosting SYS_WRITEC: {:?}", char::from(value));
                self.append_console_output(&[value]);
                cpu.r[0] = 0;
            }
            0x04 => {
                let value = self.read_c_string(cpu.r[1], 4096)?;
                log::trace!("ARM semihosting SYS_WRITE0: {value:?}");
                self.append_console_output(value.as_bytes());
                cpu.r[0] = 0;
            }
            0x18 | 0x20 => {
                log::trace!(
                    "ARM semihosting exit operation={:#04x}, reason={:#010x}",
                    cpu.r[0],
                    cpu.r[1]
                );
                self.stop_requested = true;
                cpu.r[0] = 0;
            }
            operation => {
                return Err(SimulatorError::SdkHleError(format!(
                    "unsupported ARM semihosting operation {operation:#010x} with parameter {:#010x}",
                    cpu.r[1]
                )));
            }
        }
        Ok(())
    }

    pub(super) fn append_console_output(&mut self, output: &[u8]) {
        if output.len() >= CONSOLE_OUTPUT_LIMIT {
            self.console_output.clear();
            self.console_output
                .extend_from_slice(&output[output.len() - CONSOLE_OUTPUT_LIMIT..]);
            return;
        }
        let overflow = self
            .console_output
            .len()
            .saturating_add(output.len())
            .saturating_sub(CONSOLE_OUTPUT_LIMIT);
        if overflow != 0 {
            self.console_output.drain(..overflow);
        }
        self.console_output.extend_from_slice(output);
    }
}
