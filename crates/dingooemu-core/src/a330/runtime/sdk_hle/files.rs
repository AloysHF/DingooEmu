use super::super::*;

impl RuntimeBus<'_> {
    pub(super) fn dispatch_files(&mut self, cpu: &mut Cpu, name: &str) -> Result<bool> {
        match name {
            "fopen" | "fsys_fopen" => {
                let name = self.read_c_string(cpu.r[0], 1024)?;
                let mode = self.read_c_string(cpu.r[1], 16)?;
                cpu.r[0] = self.open_file(&name, &mode);
            }
            "fsys_fopenW" => {
                let name = self.read_wide_string(cpu.r[0], 1024)?;
                let mode = self.read_c_string(cpu.r[1], 16)?;
                cpu.r[0] = self.open_file(&name, &mode);
            }
            "fclose" | "fsys_fclose" | "fsys_fcloseW" => {
                cpu.r[0] = self.close_file(cpu.r[0]);
            }
            "fread" | "fsys_fread" => {
                cpu.r[0] = self.read_file(cpu.r[0], cpu.r[1], cpu.r[2], cpu.r[3])?;
            }
            "fwrite" | "fsys_fwrite" => {
                cpu.r[0] = self.write_file(cpu.r[0], cpu.r[1], cpu.r[2], cpu.r[3])?;
            }
            "fseek" | "fsys_fseek" => {
                cpu.r[0] = self.seek_file(cpu.r[0], cpu.r[1] as i32, cpu.r[2]);
            }
            "ftell" | "fsys_ftell" => {
                cpu.r[0] = self
                    .files
                    .get(&cpu.r[0])
                    .map_or(u32::MAX, |file| file.position as u32);
            }
            "feof" | "fsys_feof" => {
                cpu.r[0] = self
                    .files
                    .get(&cpu.r[0])
                    .map_or(1, |file| u32::from(file.position >= file.data.len()));
            }
            "ferror" | "fsys_ferror" => {
                cpu.r[0] = u32::from(!self.files.contains_key(&cpu.r[0]));
            }
            "fsys_findfirst" => {
                let pattern = self.read_c_string(cpu.r[0], 1024)?;
                cpu.r[0] = self.begin_file_search(&pattern, cpu.r[1], cpu.r[2])?;
            }
            "fsys_findnext" => cpu.r[0] = self.next_file_search(cpu.r[0])?,
            "fsys_findclose" => {
                self.file_searches.remove(&cpu.r[0]);
                cpu.r[0] = 0;
            }
            "dl_res_open" => cpu.r[0] = self.open_resource([cpu.r[2], cpu.r[1], cpu.r[0]]),
            "dl_res_get_size" => {
                cpu.r[0] = self
                    .files
                    .get(&cpu.r[0])
                    .map_or(0, |file| file.data.len() as u32);
            }
            "dl_res_get_data" => {
                cpu.r[0] = self.read_resource(cpu.r[0], cpu.r[1], cpu.r[2], cpu.r[3])?;
            }
            "dl_res_close" => {
                let handle = cpu.r[0];
                if self.files.remove(&handle).is_some() {
                    self.deallocate(handle);
                }
                cpu.r[0] = 0;
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    pub(super) fn open_file(&mut self, name: &str, mode: &str) -> u32 {
        let operation = mode.as_bytes().first().copied().unwrap_or(b'r');
        let writable = matches!(operation, b'w' | b'a') || mode.contains('+');
        if !matches!(operation, b'r' | b'w' | b'a') {
            log::trace!("ARM file open rejected mode: {name:?} ({mode:?})");
            return 0;
        }
        let save_path = self
            .save_directory
            .and_then(|directory| resolve_guest_path(directory, name));
        if writable && save_path.is_none() {
            log::trace!("ARM file open has no safe save path: {name:?} ({mode:?})");
            return 0;
        }
        if operation == b'w' {
            log::trace!("ARM file create save path: {name:?}");
            return self.insert_file(Vec::new(), save_path, 0, true, true);
        }
        if operation == b'a' {
            let data = save_path
                .as_ref()
                .and_then(|path| std::fs::read(path).ok())
                .unwrap_or_default();
            let position = data.len();
            log::trace!("ARM file append save path: {name:?}");
            return self.insert_file(data, save_path, position, true, true);
        }
        if let Some((path, data)) = save_path
            .as_ref()
            .and_then(|path| std::fs::read(path).ok().map(|data| (path.clone(), data)))
        {
            log::trace!("ARM file open save path: {name:?} -> {}", path.display());
            return self.insert_file(data, writable.then_some(path), 0, writable, false);
        }
        let Some(path) = resolve_guest_path(self.content_directory, name) else {
            log::trace!("ARM file open rejected path: {name:?}");
            return 0;
        };
        let data = match std::fs::read(&path) {
            Ok(data) => {
                log::trace!("ARM file open host path: {name:?} -> {}", path.display());
                data
            }
            Err(error) => {
                if let Some(resource) = self.package.find_resource(name) {
                    log::trace!("ARM file open package resource: {name:?}");
                    self.package.get_resource_data(resource)
                } else if let Some(data) = self.package.get_embedded_file_data(name) {
                    log::trace!("ARM file open appended package payload: {name:?}");
                    data
                } else if let Some(data) =
                    self.firmware_archive.and_then(|archive| archive.read(name))
                {
                    log::trace!("ARM file open firmware resource: {name:?}");
                    data
                } else {
                    log::trace!("ARM file open failed: {name:?} ({error})");
                    return 0;
                }
            }
        };
        let persisted_path = if writable { save_path } else { None };
        self.insert_file(data, persisted_path, 0, writable, false)
    }

    pub(super) fn insert_file(
        &mut self,
        data: Vec<u8>,
        save_path: Option<PathBuf>,
        position: usize,
        writable: bool,
        dirty: bool,
    ) -> u32 {
        let stream = *self.next_file_handle;
        *self.next_file_handle = stream.wrapping_add(1).max(1);
        let handle = if self.profile == ArmProfile::Homebrew {
            let address = self.allocate(16);
            if address == 0 {
                return 0;
            }
            if self.memory.write32(address, stream).is_err() {
                self.deallocate(address);
                return 0;
            }
            address
        } else {
            stream
        };
        self.files.insert(
            handle,
            GuestFile {
                data,
                position,
                data_address: 0,
                save_path,
                writable,
                dirty,
            },
        );
        handle
    }

    pub(super) fn open_resource(&mut self, candidates: [u32; 3]) -> u32 {
        let found = candidates.into_iter().find_map(|address| {
            if address < 0x1_0000 {
                return None;
            }
            let name = self.read_c_string(address, 1024).ok()?;
            let resource = self.package.find_resource(&name)?;
            Some((name, self.package.get_resource_data(resource)))
        });
        let Some((name, data)) = found else {
            return 0;
        };
        let handle = self.allocate(16);
        if handle == 0 {
            return 0;
        }
        log::trace!("ARM resource open {name:?} -> {handle:#010x}");
        self.files.insert(
            handle,
            GuestFile {
                data,
                position: 0,
                data_address: 0,
                save_path: None,
                writable: false,
                dirty: false,
            },
        );
        handle
    }

    pub(super) fn read_resource(
        &mut self,
        handle: u32,
        destination: u32,
        buffer_len: u32,
        read_len: u32,
    ) -> Result<u32> {
        if destination == 0 {
            let existing = self.files.get(&handle).map_or(0, |file| file.data_address);
            if existing != 0 {
                return Ok(existing);
            }
            let Some(size) = self.files.get(&handle).map(|file| file.data.len() as u32) else {
                return Ok(0);
            };
            let address = self.allocate(size);
            if address == 0 {
                return Ok(0);
            }
            let data = self.files[&handle].data.clone();
            self.memory.write_bytes(address, &data)?;
            self.files.get_mut(&handle).unwrap().data_address = address;
            return Ok(address);
        }

        let data = {
            let Some(file) = self.files.get_mut(&handle) else {
                return Ok(0);
            };
            let available = file.data.len().saturating_sub(file.position);
            let requested = if read_len != 0 && buffer_len > 1 {
                (read_len as usize).saturating_mul(buffer_len as usize)
            } else if read_len != 0 {
                read_len as usize
            } else {
                buffer_len as usize
            };
            let length = if requested == 0 || requested > available {
                available
            } else {
                requested
            };
            let data = file.data[file.position..file.position + length].to_vec();
            file.position += length;
            data
        };
        self.memory.write_bytes(destination, &data)?;
        Ok(if read_len != 0 {
            (data.len() / read_len as usize) as u32
        } else {
            data.len() as u32
        })
    }

    pub(super) fn read_file(
        &mut self,
        destination: u32,
        size: u32,
        count: u32,
        handle: u32,
    ) -> Result<u32> {
        let Some(requested) = (size as usize).checked_mul(count as usize) else {
            return Ok(0);
        };
        if size == 0 || requested == 0 {
            return Ok(0);
        }
        let data = {
            let Some(file) = self.files.get_mut(&handle) else {
                return Ok(0);
            };
            if file.position >= file.data.len() {
                return Ok(0);
            }
            let available = file.data.len().saturating_sub(file.position);
            let length = requested.min(available);
            let data = file.data[file.position..file.position + length].to_vec();
            file.position += length;
            data
        };
        self.memory.write_bytes(destination, &data)?;
        Ok(data.len() as u32 / size)
    }

    pub(super) fn write_file(
        &mut self,
        source: u32,
        size: u32,
        count: u32,
        handle: u32,
    ) -> Result<u32> {
        let Some(requested) = (size as usize).checked_mul(count as usize) else {
            return Ok(0);
        };
        if size == 0 || requested == 0 {
            return Ok(0);
        }
        let data = self.memory.read_bytes(source, requested)?.to_vec();
        let Some(file) = self.files.get_mut(&handle) else {
            return Ok(0);
        };
        if !file.writable {
            return Ok(0);
        }
        let Some(end) = file.position.checked_add(data.len()) else {
            return Ok(0);
        };
        if file.data.len() < end {
            file.data.resize(end, 0);
        }
        file.data[file.position..end].copy_from_slice(&data);
        file.position = end;
        file.dirty = true;
        Ok(count)
    }

    pub(super) fn close_file(&mut self, handle: u32) -> u32 {
        let Some(file) = self.files.get_mut(&handle) else {
            return u32::MAX;
        };
        let result = flush_guest_file(file);
        self.files.remove(&handle);
        self.deallocate(handle);
        if let Err(error) = result {
            log::error!("Failed to close ARM guest save file {handle}: {error}");
            u32::MAX
        } else {
            0
        }
    }

    pub(super) fn begin_file_search(
        &mut self,
        pattern: &str,
        attributes: u32,
        data_address: u32,
    ) -> Result<u32> {
        self.file_searches.remove(&data_address);
        if data_address == 0 {
            return Ok(u32::MAX);
        }
        let Some(entries) = self.collect_file_search_entries(pattern, attributes) else {
            return Ok(u32::MAX);
        };
        let Some(first) = entries.first().cloned() else {
            return Ok(u32::MAX);
        };
        self.write_file_search_name(data_address, &first)?;
        self.file_searches.insert(
            data_address,
            FileSearch {
                entries,
                next_index: 1,
            },
        );
        Ok(0)
    }

    pub(super) fn next_file_search(&mut self, data_address: u32) -> Result<u32> {
        let Some(name) = self
            .file_searches
            .get_mut(&data_address)
            .and_then(|search| {
                let name = search.entries.get(search.next_index)?.clone();
                search.next_index += 1;
                Some(name)
            })
        else {
            return Ok(u32::MAX);
        };
        self.write_file_search_name(data_address, &name)?;
        Ok(0)
    }

    pub(super) fn collect_file_search_entries(
        &self,
        pattern: &str,
        attributes: u32,
    ) -> Option<Vec<String>> {
        let (directory, file_pattern) = normalize_guest_search_pattern(pattern)?;
        let root = self.content_directory.canonicalize().ok()?;
        let search_directory = if directory.as_os_str().is_empty() {
            root.clone()
        } else {
            root.join(directory).canonicalize().ok()?
        };
        if !search_directory.starts_with(&root) {
            return None;
        }

        let include_directories = attributes & 0x10 != 0;
        let include_files = attributes & 0x20 != 0 || !include_directories;
        let mut entries = std::fs::read_dir(search_directory)
            .ok()?
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                let file_type = entry.file_type().ok()?;
                if (file_type.is_dir() && !include_directories)
                    || (file_type.is_file() && !include_files)
                    || (!file_type.is_dir() && !file_type.is_file())
                {
                    return None;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                wildcard_matches(&file_pattern, &name).then_some(name)
            })
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| {
            left.to_ascii_lowercase()
                .cmp(&right.to_ascii_lowercase())
                .then_with(|| left.cmp(right))
        });
        Some(entries)
    }

    pub(super) fn write_file_search_name(&mut self, data_address: u32, name: &str) -> Result<()> {
        let name = name.as_bytes();
        let length = name.len().min(FILE_SEARCH_NAME_CAPACITY - 1);
        let destination = data_address.wrapping_add(FILE_SEARCH_NAME_OFFSET);
        self.memory.write_bytes(destination, &name[..length])?;
        self.memory.write8(destination + length as u32, 0)
    }

    pub(super) fn seek_file(&mut self, handle: u32, offset: i32, origin: u32) -> u32 {
        let Some(file) = self.files.get_mut(&handle) else {
            return u32::MAX;
        };
        let base = match origin {
            0 => 0,
            1 => file.position as i128,
            2 => file.data.len() as i128,
            _ => return u32::MAX,
        };
        let position = base + i128::from(offset);
        let Ok(position) = usize::try_from(position) else {
            return u32::MAX;
        };
        file.position = position;
        0
    }

    pub(super) fn read_c_string(&self, address: u32, limit: usize) -> Result<String> {
        let mut bytes = Vec::new();
        for offset in 0..limit {
            let value = self.memory.read8(address.wrapping_add(offset as u32))?;
            if value == 0 {
                return Ok(String::from_utf8_lossy(&bytes).into_owned());
            }
            bytes.push(value);
        }
        Err(SimulatorError::SdkHleError(
            "unterminated ARM guest string".into(),
        ))
    }

    pub(super) fn read_wide_string(&self, address: u32, limit: usize) -> Result<String> {
        let mut words = Vec::new();
        for offset in 0..limit {
            let value = self
                .memory
                .read16(address.wrapping_add((offset * 2) as u32))?;
            if value == 0 {
                return Ok(String::from_utf16_lossy(&words));
            }
            words.push(value);
        }
        Err(SimulatorError::SdkHleError(
            "unterminated ARM guest wide string".into(),
        ))
    }

    pub(super) fn write_guest_string(
        &mut self,
        address: u32,
        capacity: u32,
        value: &str,
    ) -> Result<bool> {
        if address == 0 || capacity == 0 {
            return Ok(false);
        }
        let count = value.len().min(capacity.saturating_sub(1) as usize);
        self.memory
            .write_bytes(address, &value.as_bytes()[..count])?;
        self.memory.write8(address + count as u32, 0)?;
        Ok(count == value.len())
    }
}
