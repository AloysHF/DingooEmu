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
                self.memory.write_bytes(cpu.r[0], &data)?;
            }
            "memcpy" | "memmove" => {
                let data = self
                    .memory
                    .read_bytes(cpu.r[1], cpu.r[2] as usize)?
                    .to_vec();
                self.memory.write_bytes(cpu.r[0], &data)?;
            }
            "printf" | "fprintf" => cpu.r[0] = 0,
            "stricmp" | "strcasecmp" => {
                let left = self.read_c_string(cpu.r[0], 4096)?;
                let right = self.read_c_string(cpu.r[1], 4096)?;
                cpu.r[0] = compare_ascii_case_insensitive(&left, &right) as u32;
            }
            "TaskMediaFunStop" | "get_current_language" => cpu.r[0] = 0,
            "GetDLHandle" | "get_dl_handle" => cpu.r[0] = STACK_BASE + 0x100,
            "__to_locale_ansi" | "_to_locale_ansi" => cpu.r[0] = LOCALE_ADDRESS,
            "dl_get_proc" => cpu.r[0] = self.dynamic_import(cpu.r[1])?,
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

    pub(super) fn allocate(&mut self, size: u32) -> u32 {
        self.heap.allocate(size)
    }

    pub(super) fn allocate_zeroed(&mut self, size: u32) -> Result<u32> {
        let address = self.allocate(size);
        if address != 0 && size != 0 {
            self.memory.write_bytes(address, &vec![0; size as usize])?;
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
        let index = match self.dynamic_imports.iter().position(|item| item == &name) {
            Some(index) => index,
            None => {
                self.dynamic_imports.push(name);
                self.dynamic_imports.len() - 1
            }
        };
        let address = DYNAMIC_THUNK_BASE + index as u32 * 8;
        self.memory.write32(address, 0xef80_0000 | index as u32)?;
        self.memory.write32(address + 4, 0xe12f_ff1e)?;
        Ok(address)
    }
}
