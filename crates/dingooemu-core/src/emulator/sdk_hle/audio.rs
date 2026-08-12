use super::super::{AudioConfig, Emulator, TaskWait, MAX_AUDIO_WRITE_BYTES};
use super::HandlerResult;
use crate::error::Result;

pub(super) fn handle(emu: &mut Emulator, func_name: &str) -> Result<HandlerResult> {
    match func_name {
        "_waveout_open" | "waveout_open" => {
            let args_ptr = emu.cpu.regs.read(4);
            let config = AudioConfig::new(
                emu.memory.read_u32(args_ptr)?,
                emu.memory.read_u16(args_ptr.wrapping_add(4))?,
                emu.memory.read_u8(args_ptr.wrapping_add(6))?,
                emu.memory.read_u8(args_ptr.wrapping_add(7))?,
            );
            let opened = config.is_some_and(|config| emu.audio.open(config));
            emu.cpu.regs.write(2, u32::from(opened));
            log::trace!("  waveout_open({args_ptr:#010x}) = {opened}");
        }
        "waveout_write" => {
            let buffer_ptr = emu.cpu.regs.read(5);
            let count = emu.cpu.regs.read(6);
            let written = if count == 0 || count > MAX_AUDIO_WRITE_BYTES {
                false
            } else if !emu.audio.can_write() {
                emu.set_active_wait(TaskWait::AudioWrite);
                log::trace!(
                    "  waveout_write({buffer_ptr:#010x}, {count}) deferred until queue space is available"
                );
                return Ok(HandlerResult::Deferred);
            } else {
                let mut data = Vec::with_capacity(count as usize);
                for offset in 0..count {
                    data.push(emu.memory.read_u8(buffer_ptr.wrapping_add(offset))?);
                }
                emu.audio.write(&data)
            };
            emu.cpu.regs.write(2, u32::from(written));
            log::trace!(
                "  waveout_write({buffer_ptr:#010x}, {count}) = {}",
                u32::from(written)
            );
        }
        "waveout_can_write" | "pcm_can_write" => {
            let can_write = emu.audio.can_write();
            emu.cpu.regs.write(2, u32::from(can_write));
            log::trace!("  {func_name}() = {}", u32::from(can_write));
        }
        "waveout_close" | "waveout_close_at_once" => {
            let closed = emu.audio.close();
            emu.cpu.regs.write(2, u32::from(closed));
            log::trace!("  {func_name}() = {}", u32::from(closed));
        }
        "_waveout_set_volume" | "waveout_set_volume" => {
            let volume = emu.cpu.regs.read(4);
            let updated = emu.audio.set_volume(volume);
            emu.cpu.regs.write(2, u32::from(updated));
            log::trace!("  {func_name}({volume}) = {}", u32::from(updated));
        }
        "HP_Mute_sw" => {
            let muted = emu.cpu.regs.read(4) != 0;
            let updated = emu.audio.set_muted(muted);
            emu.cpu.regs.write(2, u32::from(updated));
            log::trace!("  HP_Mute_sw({muted}) = {}", u32::from(updated));
        }
        "pcm_ioctl" => {
            emu.cpu.regs.write(2, 0);
            log::trace!("  pcm_ioctl() = 0");
        }
        _ => return Ok(HandlerResult::NotHandled),
    }
    Ok(HandlerResult::Complete)
}
