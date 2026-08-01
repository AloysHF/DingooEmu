use super::{Emulator, HandlerResult};
use crate::error::Result;

pub(super) fn handle(emu: &mut Emulator, func_name: &str) -> Result<HandlerResult> {
    match func_name {
        "OSTaskCreate" => {
            let entry = emu.cpu.regs.read(4);
            let data_ptr = emu.cpu.regs.read(5);
            let stack_ptr = emu.cpu.regs.read(6);
            let priority = emu.cpu.regs.read(7);
            let created = emu.create_guest_task(entry, data_ptr, stack_ptr, priority);
            emu.cpu.regs.write(2, if created { 0 } else { u32::MAX });
            log::trace!(
                "  OSTaskCreate({entry:#010x}, {data_ptr:#010x}, {stack_ptr:#010x}, {priority}) = {}",
                emu.cpu.regs.read(2)
            );
        }
        "OSSemCreate" => {
            let count = emu.cpu.regs.read(4);
            let handle = emu.create_semaphore(count);
            emu.cpu.regs.write(2, handle);
            log::trace!("  OSSemCreate({count}) = {handle}");
        }
        "OSSemPend" => {
            let handle = emu.cpu.regs.read(4);
            let error_ptr = emu.cpu.regs.read(6);
            let pending = emu.pend_semaphore(handle);
            if error_ptr != 0 {
                emu.memory
                    .write_u8(error_ptr, if pending { 0 } else { 1 })?;
            }
            log::trace!("  OSSemPend({handle}) = {pending}");
        }
        "OSSemPost" => {
            let handle = emu.cpu.regs.read(4);
            let posted = emu.post_semaphore(handle);
            emu.cpu.regs.write(2, if posted { 0 } else { 1 });
            log::trace!("  OSSemPost({handle}) = {}", emu.cpu.regs.read(2));
        }
        "OSSemDel" => {
            let handle = emu.cpu.regs.read(4);
            let error_ptr = emu.cpu.regs.read(6);
            let removed = emu.semaphores.remove(&handle).is_some();
            if error_ptr != 0 {
                emu.memory
                    .write_u8(error_ptr, if removed { 0 } else { 1 })?;
            }
            emu.cpu.regs.write(2, 0);
            log::trace!("  OSSemDel({handle}) = {removed}");
        }
        _ => return Ok(HandlerResult::NotHandled),
    }
    Ok(HandlerResult::Complete)
}
