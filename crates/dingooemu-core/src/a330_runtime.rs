use crate::a330_memory::{
    A330Memory, DYNAMIC_THUNK_BASE, EXIT_ADDRESS, FRAMEBUFFER_BASE, HEAP_SIZE, STACK_BASE,
    STACK_SIZE,
};
use crate::app_loader::PackageImage;
use crate::arm_cpu::{ArmBus, ArmCpu};
use crate::audio::Audio;
use crate::content::{ArmProfile, ContentFormat};
use crate::emulator::{UnknownHleCall, UnknownHlePolicy};
use crate::error::{Result, SimulatorError};
use crate::input::Input;
use crate::video::{Video, FRAMEBUFFER_SIZE, SCREEN_HEIGHT, SCREEN_WIDTH};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::PathBuf;

const INSTRUCTIONS_PER_SLICE: u64 = 1_000_000;
const APP_PATH_ADDRESS: u32 = STACK_BASE + 0x200;
const LOCALE_ADDRESS: u32 = STACK_BASE + 0x600;
const LEGACY_FRAMEBUFFER_ADDRESS: u32 = 0x1180_0000;
const LEGACY_GRAPHICS_SURFACE: u32 = 0x0930_201c;

struct GuestFile {
    data: Vec<u8>,
    position: usize,
}

pub(crate) struct A330Runtime {
    package: PackageImage,
    pub(crate) cpu: ArmCpu,
    pub(crate) memory: A330Memory,
    pub(crate) video: Video,
    pub(crate) audio: Audio,
    pub(crate) input: Input,
    unknown_hle_calls: BTreeMap<String, UnknownHleCall>,
    unknown_hle_policy: UnknownHlePolicy,
    unknown_hle_allowlist: BTreeSet<String>,
    next_heap: u32,
    running: bool,
    boot_complete: bool,
    app_main: Option<u32>,
    dynamic_imports: Vec<String>,
    tasks: VecDeque<(ArmCpu, u32)>,
    current_priority: u32,
    content_directory: PathBuf,
    files: BTreeMap<u32, GuestFile>,
    next_file_handle: u32,
    semaphores: BTreeMap<u32, u32>,
}

impl A330Runtime {
    pub(crate) fn from_package(package: PackageImage, _path: PathBuf) -> Result<Self> {
        let mut memory = A330Memory::from_package(&package)?;
        let cpu = ArmCpu::new(
            package.entry_point(),
            STACK_BASE + STACK_SIZE as u32 - 0x1000,
            EXIT_ADDRESS,
        );
        let next_heap = memory.heap_base();
        let file_name = _path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("game.cc");
        let locale = format!(".\\{file_name}");
        memory.write_bytes(LOCALE_ADDRESS, locale.as_bytes())?;
        memory.write8(LOCALE_ADDRESS + locale.len() as u32, 0)?;
        let mut wide_path = Vec::with_capacity((locale.len() + 1) * 2);
        for value in locale.bytes().map(u16::from).chain(std::iter::once(0)) {
            wide_path.extend_from_slice(&value.to_le_bytes());
        }
        memory.write_bytes(APP_PATH_ADDRESS, &wide_path)?;
        let app_main = package
            .exports
            .iter()
            .find(|symbol| symbol.name == "AppMain")
            .map(|symbol| symbol.address);
        let content_directory = _path.parent().map(PathBuf::from).unwrap_or_default();
        Ok(Self {
            package,
            cpu,
            memory,
            video: Video::new(),
            audio: Audio::new(),
            input: Input::new(),
            unknown_hle_calls: BTreeMap::new(),
            unknown_hle_policy: UnknownHlePolicy::default(),
            unknown_hle_allowlist: BTreeSet::new(),
            next_heap,
            running: false,
            boot_complete: false,
            app_main,
            dynamic_imports: Vec::new(),
            tasks: VecDeque::new(),
            current_priority: 0,
            content_directory,
            files: BTreeMap::new(),
            next_file_handle: 1,
            semaphores: BTreeMap::new(),
        })
    }

    pub(crate) fn start(&mut self) {
        self.running = true;
        self.cpu.start();
    }
    pub(crate) fn stop(&mut self) {
        self.running = false;
        self.cpu.stop();
    }
    pub(crate) fn reset(&mut self) -> Result<()> {
        let policy = self.unknown_hle_policy;
        let allowlist = self.unknown_hle_allowlist.clone();
        let content_directory = self.content_directory.clone();
        let mut replacement = Self::from_package(self.package.clone(), PathBuf::new())?;
        replacement.unknown_hle_policy = policy;
        replacement.unknown_hle_allowlist = allowlist;
        replacement.content_directory = content_directory;
        *self = replacement;
        Ok(())
    }
    pub(crate) fn is_running(&self) -> bool {
        self.running && self.cpu.is_running()
    }
    pub(crate) fn format(&self) -> ContentFormat {
        self.package.format()
    }
    pub(crate) fn profile(&self) -> ArmProfile {
        self.memory.profile()
    }
    pub(crate) fn package(&self) -> &PackageImage {
        &self.package
    }
    pub(crate) fn unknown_hle_calls(&self) -> impl ExactSizeIterator<Item = &UnknownHleCall> {
        self.unknown_hle_calls.values()
    }
    pub(crate) fn clear_unknown_hle_calls(&mut self) {
        self.unknown_hle_calls.clear();
    }
    pub(crate) fn set_unknown_hle_policy(&mut self, policy: UnknownHlePolicy) {
        self.unknown_hle_policy = policy;
    }
    pub(crate) fn set_unknown_hle_allowlist<I, S>(&mut self, names: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.unknown_hle_allowlist = names.into_iter().map(Into::into).collect();
    }

    pub(crate) fn tick(&mut self) -> Result<()> {
        if !self.is_running() {
            return Ok(());
        }
        let profile = self.memory.profile();
        let mut frame_address = None;
        let initial = self.cpu.instruction_count;
        let mut previous_pc = self.cpu.r[15];
        while self.cpu.is_running() && self.cpu.instruction_count - initial < INSTRUCTIONS_PER_SLICE
        {
            if self.cpu.r[15] == EXIT_ADDRESS {
                if !self.boot_complete {
                    self.boot_complete = true;
                    if let Some(entry) = self.app_main {
                        let count = self.cpu.instruction_count;
                        self.cpu = ArmCpu::new(entry, EXIT_ADDRESS - 16, EXIT_ADDRESS);
                        self.cpu.instruction_count = count;
                        self.cpu.r[0] = APP_PATH_ADDRESS;
                        self.cpu.start();
                        self.current_priority = 0;
                        continue;
                    }
                }
                if !self.activate_next_task() {
                    self.stop();
                    break;
                }
                continue;
            }
            let (stop_requested, yield_requested, finish_current) = {
                let mut bus = RuntimeBus {
                    memory: &mut self.memory,
                    imports: &self.package.imports,
                    profile,
                    unknown_hle_calls: &mut self.unknown_hle_calls,
                    unknown_hle_policy: self.unknown_hle_policy,
                    unknown_hle_allowlist: &self.unknown_hle_allowlist,
                    next_heap: &mut self.next_heap,
                    frame_address: &mut frame_address,
                    stop_requested: false,
                    dynamic_imports: &mut self.dynamic_imports,
                    tasks: &mut self.tasks,
                    current_priority: self.current_priority,
                    yield_requested: false,
                    finish_current: false,
                    content_directory: &self.content_directory,
                    files: &mut self.files,
                    next_file_handle: &mut self.next_file_handle,
                    semaphores: &mut self.semaphores,
                };
                let pc = self.cpu.r[15];
                if let Err(error) = self.cpu.step(&mut bus) {
                    return match error {
                        SimulatorError::MemoryError { .. }
                        | SimulatorError::InvalidInstruction { .. } => Err(SimulatorError::CpuError {
                            pc,
                            message: format!(
                                "{:?} state: {error}; previous_pc={previous_pc:#010x}, r0={:#010x}, r1={:#010x}, r2={:#010x}, r3={:#010x}, sp={:#010x}, lr={:#010x}",
                                self.cpu.execution_state(),
                                self.cpu.r[0],
                                self.cpu.r[1],
                                self.cpu.r[2],
                                self.cpu.r[3],
                                self.cpu.r[13],
                                self.cpu.r[14]
                            ),
                        }),
                        other => Err(other),
                    };
                }
                previous_pc = pc;
                (bus.stop_requested, bus.yield_requested, bus.finish_current)
            };
            if stop_requested {
                self.stop();
                break;
            }
            if finish_current {
                self.cpu.r[15] = EXIT_ADDRESS;
            }
            if yield_requested {
                self.rotate_task();
                break;
            }
            if frame_address.is_some() {
                break;
            }
        }
        if let Some(address) = frame_address {
            let source = self.memory.read_bytes(address, FRAMEBUFFER_SIZE)?;
            self.video.framebuffer_mut().copy_from_slice(source);
            self.video.advance_frame();
        }
        Ok(())
    }

    fn activate_next_task(&mut self) -> bool {
        if let Some((cpu, priority)) = self.tasks.pop_front() {
            self.cpu = cpu;
            self.current_priority = priority;
            true
        } else {
            false
        }
    }

    fn rotate_task(&mut self) {
        if let Some((next, priority)) = self.tasks.pop_front() {
            let current = std::mem::replace(&mut self.cpu, next);
            self.tasks.push_back((current, self.current_priority));
            self.current_priority = priority;
        }
    }
}

struct RuntimeBus<'a> {
    memory: &'a mut A330Memory,
    imports: &'a [crate::app_loader::SymbolEntry],
    profile: ArmProfile,
    unknown_hle_calls: &'a mut BTreeMap<String, UnknownHleCall>,
    unknown_hle_policy: UnknownHlePolicy,
    unknown_hle_allowlist: &'a BTreeSet<String>,
    next_heap: &'a mut u32,
    frame_address: &'a mut Option<u32>,
    stop_requested: bool,
    dynamic_imports: &'a mut Vec<String>,
    tasks: &'a mut VecDeque<(ArmCpu, u32)>,
    current_priority: u32,
    yield_requested: bool,
    finish_current: bool,
    content_directory: &'a std::path::Path,
    files: &'a mut BTreeMap<u32, GuestFile>,
    next_file_handle: &'a mut u32,
    semaphores: &'a mut BTreeMap<u32, u32>,
}

impl RuntimeBus<'_> {
    fn dispatch(&mut self, cpu: &mut ArmCpu, immediate: u32) -> Result<()> {
        let (symbol_name, symbol_address) = if immediate & 0x0080_0000 != 0 {
            let index = (immediate & 0x007f_ffff) as usize;
            let name = self
                .dynamic_imports
                .get(index)
                .ok_or_else(|| SimulatorError::CpuError {
                    pc: cpu.r[15].wrapping_sub(4),
                    message: format!("dynamic ARM SVC index {index} is invalid"),
                })?;
            (name.clone(), DYNAMIC_THUNK_BASE + index as u32 * 8)
        } else {
            let symbol =
                self.imports
                    .get(immediate as usize)
                    .ok_or_else(|| SimulatorError::CpuError {
                        pc: cpu.r[15].wrapping_sub(4),
                        message: format!("ARM SVC index {immediate} is outside the import table"),
                    })?;
            (symbol.name.clone(), symbol.address)
        };
        let name = symbol_name.as_str();
        log::trace!(
            "ARM HLE {name}(r0={:#010x}, r1={:#010x}, r2={:#010x}, r3={:#010x})",
            cpu.r[0],
            cpu.r[1],
            cpu.r[2],
            cpu.r[3]
        );
        match name {
            "lcd_get_frame" | "_lcd_get_frame" | "LCDGetFB" => cpu.r[0] = FRAMEBUFFER_BASE,
            "LCDGetWidth" | "get_lcd_width" => cpu.r[0] = SCREEN_WIDTH,
            "LCDGetHeight" | "get_lcd_height" => cpu.r[0] = SCREEN_HEIGHT,
            "lcd_set_frame" | "_lcd_set_frame" | "LCDFlushFB" | "LCDFlushFBZoom" => {
                *self.frame_address = Some(if cpu.r[0] == 0 {
                    FRAMEBUFFER_BASE
                } else {
                    cpu.r[0]
                });
                cpu.r[0] = 0;
            }
            "malloc" | "OSMalloc" | "jmalloc" => cpu.r[0] = self.allocate(cpu.r[0]),
            "calloc" => cpu.r[0] = self.allocate(cpu.r[0].saturating_mul(cpu.r[1])),
            "free" | "OSFree" | "jfree" => cpu.r[0] = 0,
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
            "fopen" | "fsys_fopen" => {
                let name = self.read_c_string(cpu.r[0], 1024)?;
                let mode = self.read_c_string(cpu.r[1], 16)?;
                cpu.r[0] = self.open_file(&name, &mode);
            }
            "fsys_fopenW" => {
                let name = self.read_wide_string(cpu.r[0], 1024)?;
                let mode = self.read_wide_string(cpu.r[1], 16)?;
                cpu.r[0] = self.open_file(&name, &mode);
            }
            "fclose" | "fsys_fclose" | "fsys_fcloseW" => {
                cpu.r[0] = if self.files.remove(&cpu.r[0]).is_some() {
                    0
                } else {
                    u32::MAX
                };
            }
            "fread" | "fsys_fread" => {
                cpu.r[0] = self.read_file(cpu.r[0], cpu.r[1], cpu.r[2], cpu.r[3])?;
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
            "ferror" | "fsys_ferror" => cpu.r[0] = u32::from(!self.files.contains_key(&cpu.r[0])),
            "printf" | "fprintf" => cpu.r[0] = 0,
            "stricmp" | "strcasecmp" => {
                let left = self.read_c_string(cpu.r[0], 4096)?;
                let right = self.read_c_string(cpu.r[1], 4096)?;
                cpu.r[0] = compare_ascii_case_insensitive(&left, &right) as u32;
            }
            "vxGoHome" | "abort" | "av_end_thread" | "av_queue_abort" => self.stop_requested = true,
            "OSTaskCreate" => {
                if cpu.r[0] != 0 && cpu.r[2] != 0 {
                    let mut task = ArmCpu::new(cpu.r[0], cpu.r[2], EXIT_ADDRESS);
                    task.r[0] = cpu.r[1];
                    task.start();
                    self.tasks.push_back((task, cpu.r[3] & 0xff));
                }
                cpu.r[0] = 0;
            }
            "OSTaskQuery" => {
                let priority = cpu.r[0] & 0xff;
                cpu.r[0] = if priority == self.current_priority
                    || self.tasks.iter().any(|(_, value)| *value == priority)
                {
                    0
                } else {
                    41
                };
            }
            "OSTaskDel" => {
                let priority = cpu.r[0] & 0xff;
                if priority == self.current_priority {
                    self.finish_current = true;
                    cpu.r[0] = 0;
                } else if let Some(index) =
                    self.tasks.iter().position(|(_, value)| *value == priority)
                {
                    self.tasks.remove(index);
                    cpu.r[0] = 0;
                } else {
                    cpu.r[0] = 41;
                }
            }
            "OSSemCreate" => {
                let initial = cpu.r[0];
                let handle = self.allocate(16);
                if handle != 0 {
                    self.semaphores.insert(handle, initial);
                }
                cpu.r[0] = handle;
            }
            "OSSemPend" => {
                let handle = cpu.r[0];
                match self.semaphores.get_mut(&handle) {
                    Some(count) if *count > 0 => {
                        *count -= 1;
                        if cpu.r[2] != 0 {
                            self.memory.write8(cpu.r[2], 0)?;
                        }
                        cpu.r[0] = 0;
                    }
                    Some(_) if self.profile == ArmProfile::Retail => {
                        cpu.r[15] = cpu.r[15].wrapping_sub(4);
                        self.yield_requested = true;
                    }
                    Some(_) => {
                        self.yield_requested = true;
                        cpu.r[0] = 0;
                    }
                    None => {
                        if cpu.r[2] != 0 {
                            self.memory.write8(cpu.r[2], 4)?;
                        }
                        cpu.r[0] = 0;
                    }
                }
            }
            "OSSemPost" => {
                cpu.r[0] = match self.semaphores.get_mut(&cpu.r[0]) {
                    Some(count) => {
                        *count = count.saturating_add(1);
                        0
                    }
                    None => 41,
                };
            }
            "OSSemDel" => {
                cpu.r[0] = if self.semaphores.remove(&cpu.r[0]).is_some() {
                    0
                } else {
                    41
                };
            }
            "OSTimeDly" | "delay" | "delay_ms" | "OSTimeDlyHMSM" => {
                cpu.r[0] = 0;
                self.yield_requested = true;
            }
            "OSTimeGet" => cpu.r[0] = (cpu.instruction_count / 150_000) as u32,
            "GetTickCount" => cpu.r[0] = (cpu.instruction_count / 15_000) as u32,
            "OSTimerGetTickTimeus" => cpu.r[0] = (cpu.instruction_count / 15) as u32,
            "TaskMediaFunStop" => cpu.r[0] = 0,
            "get_current_language" => cpu.r[0] = 0,
            "GetDLHandle" | "get_dl_handle" => cpu.r[0] = STACK_BASE + 0x100,
            "__to_locale_ansi" | "_to_locale_ansi" => cpu.r[0] = LOCALE_ADDRESS,
            "dl_get_proc" => cpu.r[0] = self.dynamic_import(cpu.r[1])?,
            "cmGetSysModel" => {
                cpu.r[0] = u32::from(!self.write_guest_string(cpu.r[0], cpu.r[1], "CC1800")?)
            }
            "cmGetSysVersion" => {
                cpu.r[0] = u32::from(!self.write_guest_string(cpu.r[0], cpu.r[1], "1.0")?)
            }
            "LCDIsDoubleFBEnabled" => cpu.r[0] = 1,
            "LCDGetFBFormat" => cpu.r[0] = 0,
            "LCDEnableDoubleFB" | "LCDDisableDoubleFB" | "LCDSetFBFormat" | "LCDInit"
            | "LCDSetRefreshRate" | "LCDSetBrightness" | "FlushDCache" | "InvalidICache"
            | "fsys_RefreshCache" | "consoleEnable" | "consoleDisable" | "PMSetMode" => {
                cpu.r[0] = 0
            }
            _ => self.record_unknown(cpu, &symbol_name, symbol_address)?,
        }
        if self.profile == ArmProfile::Homebrew {
            cpu.r[15] = cpu.r[14] & !1;
        }
        Ok(())
    }

    fn allocate(&mut self, size: u32) -> u32 {
        let size = size.max(1).saturating_add(7) & !7;
        let address = *self.next_heap;
        let heap_end = self.memory.heap_base().saturating_add(HEAP_SIZE as u32);
        match address.checked_add(size) {
            Some(end) if end <= heap_end => {
                *self.next_heap = end;
                address
            }
            _ => 0,
        }
    }

    fn dynamic_import(&mut self, name_address: u32) -> Result<u32> {
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

    fn open_file(&mut self, name: &str, mode: &str) -> u32 {
        if mode.as_bytes().first().copied().unwrap_or(b'r') != b'r' {
            return 0;
        }
        let Some(path) = resolve_guest_path(self.content_directory, name) else {
            return 0;
        };
        let Ok(data) = std::fs::read(path) else {
            return 0;
        };
        let handle = *self.next_file_handle;
        *self.next_file_handle = handle.wrapping_add(1).max(1);
        self.files.insert(handle, GuestFile { data, position: 0 });
        handle
    }

    fn read_file(&mut self, destination: u32, size: u32, count: u32, handle: u32) -> Result<u32> {
        let requested = (size as usize).saturating_mul(count as usize);
        if size == 0 || requested == 0 {
            return Ok(0);
        }
        let data = {
            let Some(file) = self.files.get_mut(&handle) else {
                return Ok(0);
            };
            let available = file.data.len().saturating_sub(file.position);
            let length = requested.min(available);
            let data = file.data[file.position..file.position + length].to_vec();
            file.position += length;
            data
        };
        self.memory.write_bytes(destination, &data)?;
        Ok(data.len() as u32 / size)
    }

    fn seek_file(&mut self, handle: u32, offset: i32, origin: u32) -> u32 {
        let Some(file) = self.files.get_mut(&handle) else {
            return u32::MAX;
        };
        let base = match origin {
            0 => 0,
            1 => file.position as i64,
            2 => file.data.len() as i64,
            _ => return u32::MAX,
        };
        let position = base.saturating_add(i64::from(offset));
        if position < 0 || position > usize::MAX as i64 {
            return u32::MAX;
        }
        file.position = position as usize;
        0
    }

    fn read_c_string(&self, address: u32, limit: usize) -> Result<String> {
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

    fn read_wide_string(&self, address: u32, limit: usize) -> Result<String> {
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

    fn write_guest_string(&mut self, address: u32, capacity: u32, value: &str) -> Result<bool> {
        if address == 0 || capacity == 0 {
            return Ok(false);
        }
        let count = value.len().min(capacity.saturating_sub(1) as usize);
        self.memory
            .write_bytes(address, &value.as_bytes()[..count])?;
        self.memory.write8(address + count as u32, 0)?;
        Ok(count == value.len())
    }

    fn record_unknown(&mut self, cpu: &mut ArmCpu, name: &str, import_address: u32) -> Result<()> {
        let pc = cpu.r[15].wrapping_sub(4);
        let call = self
            .unknown_hle_calls
            .entry(name.to_string())
            .or_insert_with(|| UnknownHleCall {
                name: name.to_string(),
                count: 0,
                import_address,
                first_pc: pc,
                first_arguments: [cpu.r[0], cpu.r[1], cpu.r[2], cpu.r[3]],
            });
        call.count += 1;
        if self.unknown_hle_policy == UnknownHlePolicy::Stop
            && !self.unknown_hle_allowlist.contains(name)
        {
            return Err(SimulatorError::UnknownHle {
                name: name.to_string(),
                pc,
                import_address,
                arguments: [cpu.r[0], cpu.r[1], cpu.r[2], cpu.r[3]],
            });
        }
        cpu.r[0] = 0;
        Ok(())
    }
}

fn resolve_guest_path(root: &std::path::Path, name: &str) -> Option<PathBuf> {
    let normalized = name.replace('\\', "/");
    let mut relative = PathBuf::new();
    for (index, component) in normalized.split('/').enumerate() {
        if component.is_empty() || component == "." {
            continue;
        }
        if component == ".." || component.contains('\0') {
            return None;
        }
        let component = if index == 0
            && component.len() == 2
            && component.as_bytes()[0].is_ascii_alphabetic()
            && component.ends_with(':')
        {
            &component[..1]
        } else {
            component
        };
        if component.contains(':') {
            return None;
        }
        relative.push(component);
    }
    (!relative.as_os_str().is_empty()).then(|| root.join(relative))
}

fn compare_ascii_case_insensitive(left: &str, right: &str) -> i32 {
    let left = left.bytes().map(|value| value.to_ascii_lowercase());
    let right = right.bytes().map(|value| value.to_ascii_lowercase());
    match left.cmp(right) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

impl ArmBus for RuntimeBus<'_> {
    fn read8(&mut self, address: u32) -> Result<u8> {
        self.memory.read8(address)
    }
    fn read16(&mut self, address: u32) -> Result<u16> {
        self.memory.read16(address)
    }
    fn read32(&mut self, address: u32) -> Result<u32> {
        self.memory.read32(address)
    }
    fn write8(&mut self, address: u32, value: u8) -> Result<()> {
        self.memory.write8(address, value)
    }
    fn write16(&mut self, address: u32, value: u16) -> Result<()> {
        self.memory.write16(address, value)
    }
    fn write32(&mut self, address: u32, value: u32) -> Result<()> {
        self.memory.write32(address, value)?;
        if address == LEGACY_GRAPHICS_SURFACE {
            *self.frame_address = Some(LEGACY_FRAMEBUFFER_ADDRESS);
        }
        Ok(())
    }
    fn svc(&mut self, cpu: &mut ArmCpu, immediate: u32) -> Result<()> {
        self.dispatch(cpu, immediate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_loader::{ChunkHeader, RawdHeader, SymbolEntry};

    fn svc_package(name: &str) -> PackageImage {
        let origin = ArmProfile::RETAIL_ORIGIN;
        let mut data = vec![0; 0x88];
        data[0x80..0x84].copy_from_slice(&0xef00_0000u32.to_le_bytes());
        data[0x84..0x88].copy_from_slice(&0xe12f_ff1eu32.to_le_bytes());
        PackageImage {
            format: ContentFormat::Cc,
            data,
            impt: ChunkHeader::default(),
            expt: ChunkHeader::default(),
            rawd: RawdHeader {
                base: ChunkHeader {
                    ident: *b"RAWD",
                    chunk_type: 0,
                    offset: 0x80,
                    size: 8,
                },
                entry: origin,
                origin,
                program_size: 8,
            },
            has_erpt: false,
            erpt: ChunkHeader::default(),
            imports: vec![SymbolEntry {
                string_offset: 0,
                unknown0: 0,
                unknown1: 0,
                address: origin,
                name: name.into(),
            }],
            exports: Vec::new(),
            resources: Vec::new(),
        }
    }

    #[test]
    fn scheduler_dispatches_svc_and_stops_at_return_sentinel() {
        let mut runtime =
            A330Runtime::from_package(svc_package("LCDGetWidth"), PathBuf::new()).unwrap();
        runtime.start();
        runtime.tick().unwrap();
        assert_eq!(runtime.cpu.r[0], SCREEN_WIDTH);
        assert_eq!(runtime.cpu.instruction_count, 2);
        assert!(!runtime.is_running());
    }

    #[test]
    fn unknown_imports_are_aggregated_and_strict_mode_stops() {
        let mut runtime =
            A330Runtime::from_package(svc_package("unknown_call"), PathBuf::new()).unwrap();
        runtime.set_unknown_hle_policy(UnknownHlePolicy::Stop);
        runtime.start();
        assert!(matches!(
            runtime.tick(),
            Err(SimulatorError::UnknownHle { .. })
        ));
        let call = runtime.unknown_hle_calls().next().unwrap();
        assert_eq!(call.name, "unknown_call");
        assert_eq!(call.count, 1);
    }

    #[test]
    fn dynamic_imports_create_reusable_svc_thunks() {
        let mut runtime =
            A330Runtime::from_package(svc_package("dl_get_proc"), PathBuf::new()).unwrap();
        runtime
            .memory
            .write_bytes(STACK_BASE, b"dynamic_call\0")
            .unwrap();
        runtime.cpu.r[1] = STACK_BASE;
        runtime.start();
        runtime.tick().unwrap();
        assert_eq!(runtime.cpu.r[0], DYNAMIC_THUNK_BASE);
        assert_eq!(
            runtime.memory.read32(DYNAMIC_THUNK_BASE).unwrap(),
            0xef80_0000
        );
        assert_eq!(
            runtime.memory.read32(DYNAMIC_THUNK_BASE + 4).unwrap(),
            0xe12f_ff1e
        );

        runtime.cpu = ArmCpu::new(DYNAMIC_THUNK_BASE, EXIT_ADDRESS - 16, EXIT_ADDRESS);
        runtime.cpu.start();
        runtime.running = true;
        runtime.boot_complete = true;
        runtime.tick().unwrap();
        assert_eq!(
            runtime.unknown_hle_calls().next().unwrap().name,
            "dynamic_call"
        );
    }

    #[test]
    fn task_create_queues_and_runs_a_guest_task() {
        let mut runtime =
            A330Runtime::from_package(svc_package("OSTaskCreate"), PathBuf::new()).unwrap();
        runtime.cpu.r[0] = ArmProfile::RETAIL_ORIGIN + 4;
        runtime.cpu.r[1] = 0x1234;
        runtime.cpu.r[2] = EXIT_ADDRESS - 0x100;
        runtime.cpu.r[3] = 7;
        runtime.start();
        runtime.tick().unwrap();
        assert!(!runtime.is_running());
        assert!(runtime.tasks.is_empty());
        assert_eq!(runtime.current_priority, 7);
    }

    #[test]
    fn guest_files_are_opened_relative_to_the_package_and_read() {
        let directory =
            std::env::temp_dir().join(format!("dingooemu-arm-files-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("asset.bin"), [1, 2, 3, 4]).unwrap();

        let mut runtime =
            A330Runtime::from_package(svc_package("fsys_fopen"), directory.join("game.c2s"))
                .unwrap();
        runtime
            .memory
            .write_bytes(STACK_BASE, b"asset.bin\0")
            .unwrap();
        runtime
            .memory
            .write_bytes(STACK_BASE + 32, b"rb\0")
            .unwrap();
        runtime.cpu.r[0] = STACK_BASE;
        runtime.cpu.r[1] = STACK_BASE + 32;
        runtime.start();
        runtime.tick().unwrap();
        let handle = runtime.cpu.r[0];
        assert_ne!(handle, 0);
        assert_eq!(runtime.files[&handle].data, [1, 2, 3, 4]);

        runtime.package.imports[0].name = "fsys_fread".into();
        runtime.cpu = ArmCpu::new(ArmProfile::RETAIL_ORIGIN, EXIT_ADDRESS - 16, EXIT_ADDRESS);
        runtime.cpu.r[0] = STACK_BASE + 64;
        runtime.cpu.r[1] = 2;
        runtime.cpu.r[2] = 2;
        runtime.cpu.r[3] = handle;
        runtime.cpu.start();
        runtime.running = true;
        runtime.boot_complete = true;
        runtime.tick().unwrap();
        assert_eq!(runtime.cpu.r[0], 2);
        assert_eq!(
            runtime.memory.read_bytes(STACK_BASE + 64, 4).unwrap(),
            [1, 2, 3, 4]
        );

        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn guest_paths_cannot_escape_the_content_directory() {
        let root = std::path::Path::new("content");
        assert_eq!(
            resolve_guest_path(root, ".\\data\\level.bin"),
            Some(root.join("data").join("level.bin"))
        );
        assert!(resolve_guest_path(root, "..\\secret.bin").is_none());
        assert!(resolve_guest_path(root, "C:\\..\\secret.bin").is_none());
    }

    #[test]
    fn semaphore_waits_retry_after_another_task_posts() {
        let origin = ArmProfile::RETAIL_ORIGIN;
        let mut runtime =
            A330Runtime::from_package(svc_package("OSSemCreate"), PathBuf::new()).unwrap();
        runtime.cpu.r[0] = 1;
        runtime.start();
        runtime.tick().unwrap();
        let handle = runtime.cpu.r[0];
        assert_eq!(runtime.semaphores[&handle], 1);

        runtime.package.imports[0].name = "OSSemPend".into();
        runtime.cpu = ArmCpu::new(origin, EXIT_ADDRESS - 16, EXIT_ADDRESS);
        runtime.cpu.r[0] = handle;
        runtime.cpu.start();
        runtime.running = true;
        runtime.boot_complete = true;
        runtime.tick().unwrap();
        assert_eq!(runtime.semaphores[&handle], 0);

        runtime.cpu = ArmCpu::new(origin, EXIT_ADDRESS - 16, EXIT_ADDRESS);
        runtime.cpu.r[0] = handle;
        runtime.cpu.start();
        runtime.running = true;
        runtime.tick().unwrap();
        assert_eq!(runtime.cpu.r[15], origin);
        assert!(runtime.is_running());

        runtime.package.imports[0].name = "OSSemPost".into();
        runtime.cpu = ArmCpu::new(origin, EXIT_ADDRESS - 16, EXIT_ADDRESS);
        runtime.cpu.r[0] = handle;
        runtime.cpu.start();
        runtime.running = true;
        runtime.tick().unwrap();
        assert_eq!(runtime.semaphores[&handle], 1);
    }
}
