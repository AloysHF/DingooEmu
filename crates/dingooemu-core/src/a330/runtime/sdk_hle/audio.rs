use super::super::*;

impl RuntimeBus<'_> {
    pub(super) fn dispatch_audio(&mut self, cpu: &mut Cpu, name: &str) -> Result<bool> {
        match name {
            "_waveout_open" | "waveout_open" => {
                let address = cpu.r[0];
                let config = AudioConfig::new(
                    self.memory.read32(address)?,
                    self.memory.read16(address + 4)?,
                    self.memory.read8(address + 6)?,
                    self.memory.read8(address + 7)?,
                );
                cpu.r[0] = u32::from(config.is_some_and(|config| self.audio.open(config)));
            }
            "waveout_write" => {
                let buffer = cpu.r[1];
                let count = cpu.r[2];
                if count == 0 || count > 4 * 1024 * 1024 {
                    cpu.r[0] = 0;
                } else if !self.audio.can_write() && self.profile == ArmProfile::Retail {
                    cpu.r[15] = cpu.r[15].wrapping_sub(4);
                    self.yield_requested = true;
                } else {
                    let data = self.memory.read_bytes(buffer, count as usize)?;
                    cpu.r[0] = u32::from(self.audio.write(data));
                }
            }
            "waveout_try_write" => {
                let count = cpu.r[2];
                cpu.r[0] = if count == 0 || count > 4 * 1024 * 1024 || !self.audio.can_write() {
                    0
                } else {
                    let data = self.memory.read_bytes(cpu.r[1], count as usize)?;
                    u32::from(self.audio.write(data))
                };
            }
            "waveout_can_write" | "waveout_can_write_nonblocking" | "pcm_can_write" => {
                cpu.r[0] = u32::from(self.audio.can_write());
            }
            "waveout_close" | "waveout_close_at_once" => {
                cpu.r[0] = u32::from(self.audio.close());
            }
            "_waveout_set_volume" | "waveout_set_volume" => {
                cpu.r[0] = u32::from(self.audio.set_volume(cpu.r[0]));
            }
            "HP_Mute_sw" | "waveout_mute" => {
                cpu.r[0] = u32::from(self.audio.set_muted(cpu.r[0] != 0));
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
}
