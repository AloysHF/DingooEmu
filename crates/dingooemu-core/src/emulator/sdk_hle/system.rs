use super::super::{Emulator, CPU_CLOCK_HZ, OS_TICKS_PER_SECOND};
use super::HandlerResult;
use crate::error::Result;

pub(super) fn handle(emu: &mut Emulator, func_name: &str) -> Result<HandlerResult> {
    match func_name {
        "malloc" => {
            let size = emu.cpu.regs.read(4);
            let pointer = emu.memory.malloc(size);
            emu.cpu.regs.write(2, pointer);
            log::info!(
                "  malloc({size}) = {pointer:#010x} (heap_ptr={:#010x})",
                emu.memory.heap_ptr()
            );
        }
        "free" => {
            let pointer = emu.cpu.regs.read(4);
            emu.memory.free(pointer);
            log::trace!("  free({pointer:#010x})");
        }
        "realloc" => {
            let pointer = emu.cpu.regs.read(4);
            let size = emu.cpu.regs.read(5);
            let new_pointer = emu.memory.realloc(pointer, size);
            emu.cpu.regs.write(2, new_pointer);
            log::trace!("  realloc({pointer:#010x}, {size}) = {new_pointer:#010x}");
        }
        "memset" => {
            let pointer = emu.cpu.regs.read(4);
            let value = emu.cpu.regs.read(5) as u8;
            let size = emu.cpu.regs.read(6);
            emu.memory.memset(pointer, value, size);
            emu.cpu.regs.write(2, pointer);
            log::trace!("  memset({pointer:#010x}, {value:#04x}, {size})");
        }
        "memcpy" => {
            let destination = emu.cpu.regs.read(4);
            let source = emu.cpu.regs.read(5);
            let size = emu.cpu.regs.read(6);
            emu.memory.memcpy(destination, source, size)?;
            emu.cpu.regs.write(2, destination);
            log::trace!("  memcpy({destination:#010x}, {source:#010x}, {size})");
        }
        "strlen" => {
            let pointer = emu.cpu.regs.read(4);
            let length = emu.memory.read_string_len(pointer);
            emu.cpu.regs.write(2, length);
            log::trace!("  strlen({pointer:#010x}) = {length}");
        }
        "__to_locale_ansi" => {
            let pointer = emu.cpu.regs.read(4);
            let result = emu.convert_guest_w_string_to_ansi(pointer);
            emu.cpu.regs.write(2, result);
            log::trace!("  __to_locale_ansi({pointer:#010x}) = {result:#010x}");
        }
        "cmGetSysModel" => {
            let pointer = emu.cpu.regs.read(4);
            for (index, word) in "A320\0".encode_utf16().enumerate() {
                emu.memory
                    .write_u16(pointer.wrapping_add((index * 2) as u32), word)?;
            }
            emu.cpu.regs.write(2, 0);
            log::trace!("  cmGetSysModel({pointer:#010x}) = 0");
        }
        "U8TOU16" | "U8TOX16" => {
            let pointer = emu.cpu.regs.read(4);
            let value = emu.memory.read_u16(pointer)? as u32;
            emu.cpu.regs.write(2, value);
            log::trace!("  {func_name}({pointer:#010x}) = {value:#06x}");
        }
        "U8TOU32" | "U8TOX32" => {
            let pointer = emu.cpu.regs.read(4);
            let value = emu.memory.read_u32(pointer)?;
            emu.cpu.regs.write(2, value);
            log::trace!("  {func_name}({pointer:#010x}) = {value:#010x}");
        }
        "OSTimeGet" => {
            let ticks = emu
                .cycle_count
                .saturating_mul(OS_TICKS_PER_SECOND)
                .checked_div(CPU_CLOCK_HZ)
                .unwrap_or(0) as u32;
            emu.cpu.regs.write(2, ticks);
            log::trace!("  OSTimeGet() = {ticks}");
        }
        "GetTickCount" => {
            let micros = emu
                .cycle_count
                .saturating_mul(1_000_000)
                .checked_div(CPU_CLOCK_HZ)
                .unwrap_or(0) as u32;
            emu.cpu.regs.write(2, micros);
            log::trace!("  GetTickCount() = {micros}");
        }
        "delay_ms" | "mdelay" => {
            let milliseconds = emu.cpu.regs.read(4);
            let delay_cycles = (milliseconds as u64).saturating_mul(CPU_CLOCK_HZ) / 1_000;
            emu.delay_active_until(emu.cycle_count.saturating_add(delay_cycles));
            log::trace!("  delay_ms({milliseconds})");
        }
        "StartSwTimer" => {
            emu.cpu.regs.write(2, 0);
            log::trace!("  StartSwTimer() = 0");
        }
        "OSTimeDly" => {
            let ticks = emu.cpu.regs.read(4);
            let delay_cycles = (ticks as u64).saturating_mul(CPU_CLOCK_HZ) / OS_TICKS_PER_SECOND;
            emu.delay_active_until(emu.cycle_count.saturating_add(delay_cycles));
            log::trace!("  OSTimeDly({ticks})");
        }
        "udelay" => {
            let microseconds = emu.cpu.regs.read(4);
            let delay_cycles = (microseconds as u64).saturating_mul(CPU_CLOCK_HZ) / 1_000_000;
            emu.delay_active_until(emu.cycle_count.saturating_add(delay_cycles));
            log::trace!("  udelay({microseconds})");
        }
        "vxGoHome" | "abort" | "TaskMediaFunStop" => {
            emu.cpu.stop();
            log::trace!("  {func_name} -> stopping");
        }
        "sprintf" => {
            let destination = emu.cpu.regs.read(4);
            let format = emu.read_guest_c_string(emu.cpu.regs.read(5));
            let rendered = emu.format_guest_printf(&format)?;
            let mut bytes = rendered.as_bytes().to_vec();
            bytes.push(0);
            emu.memory.load_data(destination, &bytes)?;
            emu.cpu.regs.write(2, rendered.len() as u32);
            log::trace!("  sprintf({format}) = {}", rendered.len());
        }
        "printf" | "fprintf" => {
            emu.cpu.regs.write(2, 0);
            log::trace!("  {func_name}() = 0 (stub)");
        }
        "__icache_invalidate_all" => {
            emu.clear_instruction_cache();
            log::trace!("  {func_name} (instruction cache cleared)");
        }
        "__dcache_writeback_all" => {
            log::trace!("  {func_name} (no-op)");
        }
        _ => return Ok(HandlerResult::NotHandled),
    }
    Ok(HandlerResult::Complete)
}
