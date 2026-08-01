use super::{Emulator, HandlerResult};
use crate::error::Result;

pub(super) fn handle(emu: &mut Emulator, func_name: &str) -> Result<HandlerResult> {
    match func_name {
        "get_dl_handle" => {
            emu.cpu.regs.write(2, u32::from(emu.app.is_some()));
        }
        "dl_res_open" => {
            let name = emu.resource_name_from_args(&[
                emu.cpu.regs.read(6),
                emu.cpu.regs.read(5),
                emu.cpu.regs.read(4),
            ]);
            let handle = name
                .as_deref()
                .map(|name| emu.open_resource_file(name))
                .unwrap_or(0);
            emu.cpu.regs.write(2, handle);
        }
        "dl_res_get_size" => {
            let handle = emu.cpu.regs.read(4);
            let size = emu
                .open_files
                .get(&handle)
                .map(|file| file.data.len() as u32)
                .unwrap_or(0);
            emu.cpu.regs.write(2, size);
        }
        "dl_res_get_data" => {
            let handle = emu.cpu.regs.read(4);
            let buffer = emu.cpu.regs.read(5);
            let buffer_len = emu.cpu.regs.read(6);
            let read_len = emu.cpu.regs.read(7);
            let result = emu.read_resource_data(handle, buffer, buffer_len, read_len)?;
            emu.cpu.regs.write(2, result);
        }
        "dl_res_close" => {
            let handle = emu.cpu.regs.read(4);
            emu.open_files.remove(&handle);
            emu.cpu.regs.write(2, 0);
        }
        "fopen" | "fsys_fopen" => {
            let name = emu.read_guest_c_string(emu.cpu.regs.read(4));
            let mode = emu.read_guest_c_string(emu.cpu.regs.read(5));
            let handle = emu.open_guest_file(&name, &mode);
            emu.cpu.regs.write(2, handle);
            log::trace!("  {func_name}({name}, {mode}) = {handle}");
        }
        "fsys_fopenW" => {
            let name = emu.read_guest_w_string(emu.cpu.regs.read(4));
            let mode = emu.read_guest_w_string(emu.cpu.regs.read(5));
            log::trace!("  fsys_fopenW({name}, {mode})");
            let handle = emu.open_guest_file(&name, &mode);
            emu.cpu.regs.write(2, handle);
        }
        "fclose" | "fsys_fclose" | "fsys_fcloseW" => {
            let handle = emu.cpu.regs.read(4);
            let result = emu.flush_save_file(handle);
            emu.open_files.remove(&handle);
            emu.cpu
                .regs
                .write(2, if result.is_ok() { 0 } else { u32::MAX });
        }
        "fread" | "fsys_fread" => {
            let dest = emu.cpu.regs.read(4);
            let size = emu.cpu.regs.read(5);
            let count = emu.cpu.regs.read(6);
            let handle = emu.cpu.regs.read(7);
            let read = emu.read_file(dest, size, count, handle)?;
            emu.cpu.regs.write(2, read);
        }
        "fseek" | "fsys_fseek" => {
            let handle = emu.cpu.regs.read(4);
            let offset = emu.cpu.regs.read(5) as i32;
            let origin = emu.cpu.regs.read(6);
            let result = emu.seek_file(handle, offset, origin);
            emu.cpu.regs.write(2, result);
        }
        "ftell" | "fsys_ftell" => {
            let handle = emu.cpu.regs.read(4);
            let position = emu
                .open_files
                .get(&handle)
                .map(|file| file.position as u32)
                .unwrap_or(u32::MAX);
            emu.cpu.regs.write(2, position);
        }
        "feof" | "fsys_feof" => {
            let handle = emu.cpu.regs.read(4);
            let eof = emu
                .open_files
                .get(&handle)
                .map(|file| u32::from(file.position >= file.data.len()))
                .unwrap_or(1);
            emu.cpu.regs.write(2, eof);
        }
        "fwrite" | "fsys_fwrite" => {
            let source = emu.cpu.regs.read(4);
            let size = emu.cpu.regs.read(5);
            let count = emu.cpu.regs.read(6);
            let handle = emu.cpu.regs.read(7);
            let written = emu.write_file(source, size, count, handle)?;
            emu.cpu.regs.write(2, written);
            log::trace!("  {func_name}() = {written}");
        }
        "fsys_findfirst" => {
            let pattern = emu.read_guest_c_string(emu.cpu.regs.read(4));
            let attributes = emu.cpu.regs.read(5);
            let data_ptr = emu.cpu.regs.read(6);
            let result = emu.begin_file_search(&pattern, attributes, data_ptr)?;
            emu.cpu.regs.write(2, result);
            log::trace!(
                "  fsys_findfirst({pattern}, {attributes:#x}, {data_ptr:#010x}) = {result:#010x}"
            );
        }
        "fsys_findnext" => {
            let data_ptr = emu.cpu.regs.read(4);
            let result = emu.next_file_search(data_ptr)?;
            emu.cpu.regs.write(2, result);
            log::trace!("  fsys_findnext({data_ptr:#010x}) = {result:#010x}");
        }
        "fsys_findclose" => {
            let data_ptr = emu.cpu.regs.read(4);
            let result = emu.close_file_search(data_ptr);
            emu.cpu.regs.write(2, result);
            log::trace!("  fsys_findclose({data_ptr:#010x}) = {result}");
        }
        _ => return Ok(HandlerResult::NotHandled),
    }
    Ok(HandlerResult::Complete)
}
