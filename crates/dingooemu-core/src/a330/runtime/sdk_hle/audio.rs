use super::super::*;

const DVC_AUDIO_MAGIC: u32 = 0x4155_4449;
const DVC_AUDIO_MAX_VOLUME: u32 = 30;

impl RuntimeBus<'_> {
    fn dvc_audio_handle_valid(&self, handle: u32) -> bool {
        self.heap
            .blocks
            .iter()
            .any(|block| block.address == handle && !block.free && block.size >= 12)
            && self.memory.read32(handle).ok() == Some(DVC_AUDIO_MAGIC)
    }

    fn open_legacy_audio_device(&mut self, name_address: u32) -> Result<u32> {
        if self.read_c_string(name_address, 128)? != "ROOT\\DVC\\IIS\\IIS0" {
            return Ok(0);
        }
        let handle = self.allocate(12);
        if handle != 0 {
            self.write_memory(handle, &DVC_AUDIO_MAGIC.to_le_bytes())?;
            self.write_memory(handle + 4, &44_100_u32.to_le_bytes())?;
            self.audio.set_volume(255);
            *self.current_audio_producer = true;
        }
        Ok(handle)
    }

    fn control_legacy_audio_device(
        &mut self,
        handle: u32,
        command: u32,
        argument: u32,
    ) -> Result<u32> {
        if !self.dvc_audio_handle_valid(handle) {
            return Ok(u32::MAX);
        }
        match command {
            0x0d => {
                let rate = self.memory.read32(argument)?;
                if AudioConfig::new(rate, 16, 2, 255).is_none() {
                    return Ok(u32::MAX);
                }
                self.write_memory(handle + 4, &rate.to_le_bytes())?;
            }
            0x0b => {
                let enabled = self.memory.read32(argument)? != 0;
                if enabled {
                    let rate = self.memory.read32(handle + 4)?;
                    let opened = AudioConfig::new(rate, 16, 2, self.audio.device_volume())
                        .is_some_and(|config| self.audio.open(config));
                    if !opened {
                        return Ok(u32::MAX);
                    }
                } else {
                    self.audio.close();
                }
                self.write_memory(handle + 8, &u32::from(enabled).to_le_bytes())?;
            }
            _ => {}
        }
        Ok(0)
    }

    fn write_legacy_audio_device(&mut self, cpu: &mut Cpu) -> Result<()> {
        let buffer = cpu.r[0];
        let count = cpu.r[1];
        let handle = cpu.r[2];
        *self.current_audio_producer = true;
        cpu.r[0] = u32::MAX;
        if !self.dvc_audio_handle_valid(handle)
            || self.memory.read32(handle + 8)? == 0
            || count == 0
            || count > 4 * 1024 * 1024
        {
            return Ok(());
        }
        if !self.audio.can_write() {
            // Retry inline once the device has consumed its queued samples.
            cpu.r[0] = buffer;
            cpu.r[15] = cpu.r[15].wrapping_sub(4);
            self.requested_delay_ticks = 1;
            self.sleep_requested = true;
            return Ok(());
        }
        let data = self.memory.read_bytes(buffer, count as usize)?;
        let written = self.audio.write(data);
        *self.audio_written |= written;
        if written {
            cpu.r[0] = count;
        }
        Ok(())
    }

    fn dispatch_legacy_audio(&mut self, cpu: &mut Cpu, name: &str) -> Result<bool> {
        match name {
            "DVCOpenDevice" => cpu.r[0] = self.open_legacy_audio_device(cpu.r[0])?,
            "DVCControlDevice" => {
                cpu.r[0] = self.control_legacy_audio_device(cpu.r[0], cpu.r[2], cpu.r[3])?
            }
            "DVCWriteDevice" => self.write_legacy_audio_device(cpu)?,
            "DVCCloseDevice" => {
                let handle = cpu.r[0];
                cpu.r[0] = u32::MAX;
                if self.dvc_audio_handle_valid(handle) {
                    self.audio.close();
                    self.write_memory(handle, &0_u32.to_le_bytes())?;
                    self.deallocate(handle);
                    cpu.r[0] = 0;
                }
            }
            "wavaopen" | "waveioc" | "waveclose" => cpu.r[0] = 0,
            "SYSSetVolume" => {
                let volume = cpu.r[0].min(DVC_AUDIO_MAX_VOLUME);
                self.audio
                    .set_volume((volume * 255 + DVC_AUDIO_MAX_VOLUME / 2) / DVC_AUDIO_MAX_VOLUME);
                cpu.r[0] = 0;
            }
            "SYSGetVolume" | "get_game_vol" => {
                cpu.r[0] =
                    (u32::from(self.audio.device_volume()) * DVC_AUDIO_MAX_VOLUME + 127) / 255;
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    pub(super) fn dispatch_audio(&mut self, cpu: &mut Cpu, name: &str) -> Result<bool> {
        if self.dispatch_legacy_audio(cpu, name)? {
            return Ok(true);
        }
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
            _ => return Ok(false),
        }
        Ok(true)
    }
}
