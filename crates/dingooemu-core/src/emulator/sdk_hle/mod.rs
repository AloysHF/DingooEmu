use super::Emulator;
use crate::error::Result;

mod audio;
mod files;
mod graphics;
mod gui;
mod input;
mod system;
mod tasks;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum HandlerResult {
    NotHandled,
    Complete,
    Deferred,
    GuestCallback,
}

type Handler = fn(&mut Emulator, &str) -> Result<HandlerResult>;

const HANDLERS: [Handler; 7] = [
    graphics::handle,
    gui::handle,
    input::handle,
    audio::handle,
    files::handle,
    tasks::handle,
    system::handle,
];

pub(super) fn dispatch(emu: &mut Emulator, addr: u32, func_name: &str) -> Result<()> {
    log::trace!("SDK call: {addr:#010x} = {func_name}");
    let return_address = emu.cpu.regs.read(31);

    let mut result = HandlerResult::NotHandled;
    for handler in HANDLERS {
        result = handler(emu, func_name)?;
        if result != HandlerResult::NotHandled {
            break;
        }
    }

    match result {
        HandlerResult::Complete => emu.cpu.regs.pc = return_address,
        HandlerResult::Deferred => {}
        HandlerResult::GuestCallback => {}
        HandlerResult::NotHandled => {
            emu.record_unknown_hle(func_name, addr, return_address)?;
            emu.cpu.regs.write(2, 0);
            emu.cpu.regs.pc = return_address;
            log::trace!("  {func_name}() = 0 (compatibility stub)");
        }
    }
    emu.cpu.regs.gpr[0] = 0;
    Ok(())
}
