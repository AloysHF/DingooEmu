use super::super::*;

impl RuntimeBus<'_> {
    pub(super) fn dispatch_system(&mut self, cpu: &mut Cpu, name: &str) -> Result<bool> {
        match name {
            "malloc" | "OSMalloc" | "jmalloc" => cpu.r[0] = self.allocate(cpu.r[0]),
            "calloc" => {
                cpu.r[0] = match cpu.r[0].checked_mul(cpu.r[1]) {
                    Some(size) => self.allocate_zeroed(size)?,
                    None => 0,
                };
            }
            "realloc" => cpu.r[0] = self.reallocate(cpu.r[0], cpu.r[1])?,
            "free" | "OSFree" | "jfree" => {
                self.deallocate(cpu.r[0]);
                cpu.r[0] = 0;
            }
            "memset" => {
                let data = vec![cpu.r[1] as u8; cpu.r[2] as usize];
                self.write_memory(cpu.r[0], &data)?;
            }
            "memcpy" | "memmove" => {
                let data = self
                    .memory
                    .read_bytes(cpu.r[1], cpu.r[2] as usize)?
                    .to_vec();
                self.write_memory(cpu.r[0], &data)?;
            }
            "printf" => {
                if let Ok(format) = self.read_c_string(cpu.r[0], 256) {
                    let mut line = format.clone();
                    if format.contains("%s") {
                        for reg in [cpu.r[1], cpu.r[2], cpu.r[3]] {
                            if let Ok(value) = self.read_c_string(reg, 128) {
                                if value
                                    .bytes()
                                    .all(|b| b.is_ascii_graphic() || b == b' ' || b == b'.')
                                    && !value.is_empty()
                                {
                                    if let Some(pos) = line.find("%s") {
                                        line.replace_range(pos..pos + 2, &value);
                                    }
                                }
                            }
                        }
                    }
                    if std::env::var_os("DINGOOEMU_HLE_TRACE").is_some() {
                        eprintln!("ARM printf: {line}");
                    }
                    self.append_console_output(line.as_bytes());
                    self.append_console_output(b"\n");
                }
                cpu.r[0] = 0;
            }
            "fprintf" => {
                if let Ok(format) = self.read_c_string(cpu.r[1], 256) {
                    if std::env::var_os("DINGOOEMU_HLE_TRACE").is_some() {
                        eprintln!("ARM fprintf: {format}");
                    }
                    self.append_console_output(format.as_bytes());
                    self.append_console_output(b"\n");
                }
                cpu.r[0] = 0;
            }
            "stricmp" | "strcasecmp" => {
                let left = self.read_c_string(cpu.r[0], 4096)?;
                let right = self.read_c_string(cpu.r[1], 4096)?;
                cpu.r[0] = compare_ascii_case_insensitive(&left, &right) as u32;
            }
            "TaskMediaFunStop" | "get_current_language" => cpu.r[0] = 0,
            "GetDLHandle" | "get_dl_handle" => cpu.r[0] = STACK_BASE + 0x100,
            "__to_locale_ansi" | "_to_locale_ansi" => cpu.r[0] = LOCALE_ADDRESS,
            "dl_get_proc" => {
                if std::env::var_os("DINGOOEMU_HLE_TRACE").is_some() {
                    if let Ok(name) = self.read_c_string(cpu.r[1], 128) {
                        eprintln!("ARM dl_get_proc {name:?}");
                    }
                }
                cpu.r[0] = self.resolve_dynamic_import_by_name(cpu.r[1])?;
            }
            "dl_load" | "dl_free" => cpu.r[0] = 0,
            "USB_No_Connect" | "udc_attached" | "USB_Connect" | "usb_connect"
            | "usb_disconnect" => cpu.r[0] = 0,
            "Tp_Get_Pos"
            | "serial_putc"
            | "serial_getc"
            | "OSCPURestoreSR"
            | "OSCPUSaveSR"
            | "isTVON"
            | "free_irq"
            | "StartSwTimer"
            | "LcdGetDisMode"
            | "av_wo_create"
            | "av_wo_write"
            | "av_wo_destroy"
            | "pcm_ioctl"
            | "waveopen"
            | "OSTaskChangePrio"
            | "OSTaskResume"
            | "OSTaskSuspend"
            | "SysSetLastBrightness"
            | "TVOUTInit"
            | "TVOUTExit"
            | "LCDYUVDraw"
            | "DVCReadDevice"
            | "fsys_chdir"
            | "fsys_mkdir"
            | "fsys_flush_cache"
            | "fsys_rename"
            | "fsys_remove"
            | "fsys_RefreshCache"
            | "__to_unicode_le"
            | "heap_get_block_size" => cpu.r[0] = 0,
            "LCDGetRefreshRate" => cpu.r[0] = 60,
            "cmGetSysModel" => {
                cpu.r[0] = u32::from(!self.write_guest_string(cpu.r[0], cpu.r[1], "CC1800")?);
            }
            "cmGetSysVersion" => {
                cpu.r[0] = u32::from(!self.write_guest_string(cpu.r[0], cpu.r[1], "1.0")?);
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    pub(super) fn resolve_dynamic_import_by_name(&mut self, name_address: u32) -> Result<u32> {
        let name = self.read_c_string(name_address, 256)?;
        if let Some(symbol) = self
            .package
            .exports
            .iter()
            .find(|symbol| symbol.name == name)
        {
            return Ok(symbol.address);
        }
        if let Some(symbol) = self
            .package
            .imports
            .iter()
            .find(|symbol| symbol.name == name)
        {
            return Ok(symbol.address);
        }
        self.dynamic_import(name_address)
    }

    pub(super) fn allocate(&mut self, size: u32) -> u32 {
        self.heap.allocate(self.memory, size)
    }

    pub(super) fn allocate_zeroed(&mut self, size: u32) -> Result<u32> {
        let address = self.allocate(size);
        if address != 0 && size != 0 {
            self.write_memory(address, &vec![0; size as usize])?;
        }
        Ok(address)
    }

    pub(super) fn deallocate(&mut self, address: u32) {
        self.heap.deallocate(address);
    }

    pub(super) fn reallocate(&mut self, address: u32, size: u32) -> Result<u32> {
        self.heap.reallocate(self.memory, address, size)
    }

    pub(super) fn dynamic_import(&mut self, name_address: u32) -> Result<u32> {
        let name = self.read_c_string(name_address, 256)?;
        let index = match self.dynamic_imports.iter().position(|item| *item == name) {
            Some(index) => index,
            None => {
                self.dynamic_imports.push(name);
                self.dynamic_imports.len() - 1
            }
        };
        let address = DYNAMIC_THUNK_BASE + index as u32 * 8;
        // Dynamic thunks continue the static import index space:
        // SVC #(import_count + slot), not a bit-flag encoding.
        let svc_index = self.imports.len() as u32 + index as u32;
        self.write_memory(address, &(0xef00_0000 | svc_index).to_le_bytes())?;
        self.write_memory(address + 4, &0xe12f_ff1e_u32.to_le_bytes())?;
        Ok(address)
    }
}
