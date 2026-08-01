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
            let buttons = emu.input.buttons();
            let key = if buttons & crate::input::BUTTON_UP != 0 {
                20
            } else if buttons & crate::input::BUTTON_DOWN != 0 {
                27
            } else if buttons & crate::input::BUTTON_LEFT != 0 {
                28
            } else if buttons & crate::input::BUTTON_RIGHT != 0 {
                18
            } else if buttons & crate::input::BUTTON_A != 0 {
                31
            } else if buttons & crate::input::BUTTON_B != 0 {
                21
            } else if buttons & crate::input::BUTTON_X != 0 {
                16
            } else if buttons & crate::input::BUTTON_Y != 0 {
                6
            } else if buttons & crate::input::BUTTON_START != 0 {
                11
            } else if buttons & crate::input::BUTTON_SELECT != 0 {
                10
            } else if buttons & crate::input::BUTTON_L != 0 {
                8
            } else if buttons & crate::input::BUTTON_R != 0 {
                29
            } else {
                0
            };
            emu.cpu.regs.write(2, key);
            log::trace!("  kbd_get_key() = {key}");
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
