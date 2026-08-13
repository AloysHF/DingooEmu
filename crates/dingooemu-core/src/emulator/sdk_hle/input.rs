use super::{Emulator, HandlerResult};
use crate::error::Result;

pub(super) fn handle(emu: &mut Emulator, func_name: &str) -> Result<HandlerResult> {
    match func_name {
        "_kbd_get_status" | "kbd_get_status" => {
            let status_ptr = emu.cpu.regs.read(4);
            let (pressed, released, status) = emu.input.take_status();
            emu.memory.write_u32(status_ptr, pressed)?;
            emu.memory.write_u32(status_ptr.wrapping_add(4), released)?;
            emu.memory.write_u32(status_ptr.wrapping_add(8), status)?;
            log::trace!(
                "  kbd_get_status({status_ptr:#010x}) pressed={pressed:#010x} released={released:#010x} status={status:#010x}"
            );
        }
        "_kbd_get_key" | "kbd_get_key" => {
            let status = emu.input.buttons();
            emu.cpu.regs.write(2, status);
            log::trace!("  kbd_get_key() = {status:#010x}");
        }
        "_sys_judge_event" | "sys_judge_event" => {
            let pending = u32::from(emu.input.take_pending_event());
            emu.cpu.regs.write(2, pending);
            log::trace!("  sys_judge_event() = {pending}");
        }
        _ => return Ok(HandlerResult::NotHandled),
    }
    Ok(HandlerResult::Complete)
}
