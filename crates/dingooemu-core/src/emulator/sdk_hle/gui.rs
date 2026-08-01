use super::{Emulator, HandlerResult};
use crate::error::Result;

const WM_KEY: u32 = 14;
const WM_MESSAGE_SIZE: u32 = 12;
const GUI_KEY_INFO_SIZE: u32 = 8;
const GUI_KEY_ENTER: u32 = 13;
const GUI_KEY_LEFT: u32 = 16;
const GUI_KEY_UP: u32 = 17;
const GUI_KEY_RIGHT: u32 = 18;
const GUI_KEY_DOWN: u32 = 19;
const GUI_KEY_ESCAPE: u32 = 27;

pub(super) fn handle(emu: &mut Emulator, func_name: &str) -> Result<HandlerResult> {
    match func_name {
        "open_gui_key_msg" => {
            emu.gui.key_messages_enabled = true;
            emu.cpu.regs.write(2, 0);
            log::trace!("  open_gui_key_msg() - key messages enabled");
        }
        "WM_CreateWindow" => create_window(emu)?,
        "WM_SetFocus" => set_focus(emu),
        "GUI_Exec" => return execute_gui(emu),
        _ => return Ok(HandlerResult::NotHandled),
    }
    Ok(HandlerResult::Complete)
}

fn create_window(emu: &mut Emulator) -> Result<()> {
    let stack_pointer = emu.cpu.regs.read(29);
    let callback = emu.memory.read_u32(stack_pointer.wrapping_add(20))?;
    if callback == 0 {
        emu.cpu.regs.write(2, 0);
        return Ok(());
    }

    let handle = emu.gui.next_window_handle;
    emu.gui.next_window_handle = emu.gui.next_window_handle.wrapping_add(1).max(1);
    emu.gui.windows.insert(handle, callback);
    emu.cpu.regs.write(2, handle);
    log::trace!("  WM_CreateWindow(callback={callback:#010x}) = {handle:#010x}");
    Ok(())
}

fn set_focus(emu: &mut Emulator) {
    let handle = emu.cpu.regs.read(4);
    if emu.gui.windows.contains_key(&handle) {
        emu.gui.focused_window = Some(handle);
    }
    emu.cpu.regs.write(2, 0);
    log::trace!("  WM_SetFocus({handle:#010x})");
}

fn execute_gui(emu: &mut Emulator) -> Result<HandlerResult> {
    emu.cpu.regs.write(2, 0);
    if !emu.gui.key_messages_enabled {
        return Ok(HandlerResult::Complete);
    }

    let current_key = gui_key_code(emu.input.buttons());
    let (key, pressed, next_reported_key) =
        if emu.gui.reported_key != 0 && emu.gui.reported_key != current_key {
            let key = emu.gui.reported_key;
            (key, 0, 0)
        } else if emu.gui.reported_key == 0 && current_key != 0 {
            (current_key, 1, current_key)
        } else {
            return Ok(HandlerResult::Complete);
        };

    let Some(window) = emu.gui.focused_window else {
        return Ok(HandlerResult::Complete);
    };
    let Some(&callback) = emu.gui.windows.get(&window) else {
        return Ok(HandlerResult::Complete);
    };

    let message = match emu.gui.message_buffer {
        Some(address) => address,
        None => {
            let address = emu.memory.malloc(WM_MESSAGE_SIZE);
            if address == 0 {
                return Ok(HandlerResult::Complete);
            }
            emu.gui.message_buffer = Some(address);
            address
        }
    };
    let key_info = match emu.gui.key_info_buffer {
        Some(address) => address,
        None => {
            let address = emu.memory.malloc(GUI_KEY_INFO_SIZE);
            if address == 0 {
                return Ok(HandlerResult::Complete);
            }
            emu.gui.key_info_buffer = Some(address);
            address
        }
    };

    emu.memory.write_u32(key_info, key)?;
    emu.memory.write_u32(key_info.wrapping_add(4), pressed)?;
    emu.memory.write_u32(message, WM_KEY)?;
    emu.memory.write_u32(message.wrapping_add(4), window)?;
    emu.memory.write_u32(message.wrapping_add(8), key_info)?;

    emu.gui.reported_key = next_reported_key;
    emu.cpu.regs.write(4, message);
    emu.cpu.regs.pc = callback;
    log::trace!(
        "  GUI_Exec() - dispatching WM_KEY key={key} pressed={pressed} to {callback:#010x}"
    );
    Ok(HandlerResult::GuestCallback)
}

fn gui_key_code(buttons: u32) -> u32 {
    if buttons & crate::input::BUTTON_LEFT != 0 {
        GUI_KEY_LEFT
    } else if buttons & crate::input::BUTTON_UP != 0 {
        GUI_KEY_UP
    } else if buttons & crate::input::BUTTON_RIGHT != 0 {
        GUI_KEY_RIGHT
    } else if buttons & crate::input::BUTTON_DOWN != 0 {
        GUI_KEY_DOWN
    } else if buttons & crate::input::BUTTON_A != 0 {
        GUI_KEY_ENTER
    } else if buttons & crate::input::BUTTON_B != 0 {
        GUI_KEY_ESCAPE
    } else {
        0
    }
}
