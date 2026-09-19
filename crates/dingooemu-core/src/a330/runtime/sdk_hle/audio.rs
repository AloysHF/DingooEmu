use super::super::*;

impl RuntimeBus<'_> {
    pub(super) fn dispatch_audio(&mut self, cpu: &mut Cpu, name: &str) -> Result<bool> {
        match name {
            "_waveout_open" | "waveout_open" => {
                *self.current_audio_producer = true;
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
                *self.current_audio_producer = true;
                let buffer = cpu.r[1];
                let count = cpu.r[2];
                if count == 0 || count > 4 * 1024 * 1024 {
                    cpu.r[0] = 0;
                } else if !self.audio.can_write() && self.profile == ArmProfile::Retail {
                    // The device buffer is full; the guest retries the same
                    // write once the consumed audio frees space. Waiting for
                    // playback must not burn a scheduler slice.
                    cpu.r[15] = cpu.r[15].wrapping_sub(4);
                    self.requested_delay_ticks = 1;
                    self.sleep_requested = true;
                } else {
                    let data = self.memory.read_bytes(buffer, count as usize)?;
                    let written = self.audio.write(data);
                    *self.audio_written |= written;
                    cpu.r[0] = u32::from(written);
                }
            }
            "waveout_try_write" => {
                *self.current_audio_producer = true;
                let count = cpu.r[2];
                cpu.r[0] = if count == 0 || count > 4 * 1024 * 1024 || !self.audio.can_write() {
                    0
                } else {
                    let data = self.memory.read_bytes(cpu.r[1], count as usize)?;
                    let written = self.audio.write(data);
                    *self.audio_written |= written;
                    u32::from(written)
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
            "DVCOpenDevice" => {
                *self.current_audio_producer = true;
                let device = self.read_c_string(cpu.r[0], 128).unwrap_or_default();
                cpu.r[0] = self.audio.dvc_open(&device);
            }
            "DVCControlDevice" => {
                let argument = if cpu.r[3] == 0 {
                    None
                } else {
                    self.memory.read32(cpu.r[3]).ok()
                };
                cpu.r[0] = self.audio.dvc_control(cpu.r[0], cpu.r[2], argument);
            }
            "DVCWriteDevice" => {
                *self.current_audio_producer = true;
                let count = cpu.r[1];
                if count == 0 || count > 4 * 1024 * 1024 || !self.audio.dvc_started() {
                    cpu.r[0] = u32::MAX;
                } else if !self.audio.can_write() {
                    cpu.r[15] = cpu.r[15].wrapping_sub(4);
                    self.requested_delay_ticks = 1;
                    self.sleep_requested = true;
                } else {
                    let data = self.memory.read_bytes(cpu.r[0], count as usize)?.to_vec();
                    let written = self.audio.dvc_write(cpu.r[2], &data);
                    *self.audio_written |= written != u32::MAX;
                    cpu.r[0] = written;
                }
            }
            "DVCCloseDevice" => {
                self.audio.dvc_close();
                cpu.r[0] = 0;
            }
            "SYSSetVolume" => {
                self.audio.dvc_set_volume(cpu.r[0]);
                cpu.r[0] = 0;
            }
            "SYSGetVolume" | "get_game_vol" => {
                cpu.r[0] = if self.audio.dvc_volume() == 0 {
                    30
                } else {
                    self.audio.dvc_volume()
                };
            }
            "wavaopen" | "waveioc" | "waveclose" => cpu.r[0] = 0,
            _ => return Ok(false),
        }
        Ok(true)
    }
}
