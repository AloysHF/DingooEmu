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
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

const INSTRUCTIONS_PER_SLICE: u64 = 1_000_000;
const APP_PATH_ADDRESS: u32 = STACK_BASE + 0x200;
const LOCALE_ADDRESS: u32 = STACK_BASE + 0x600;

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
        let mut replacement = Self::from_package(self.package.clone(), PathBuf::new())?;
        replacement.unknown_hle_policy = policy;
        replacement.unknown_hle_allowlist = allowlist;
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
        let imports = &self.package.imports;
        let profile = self.memory.profile();
        let mut frame_address = None;
        let initial = self.cpu.instruction_count;
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
                        continue;
                    }
                }
                self.stop();
                break;
            }
            let mut bus = RuntimeBus {
                memory: &mut self.memory,
                imports,
                profile,
                unknown_hle_calls: &mut self.unknown_hle_calls,
                unknown_hle_policy: self.unknown_hle_policy,
                unknown_hle_allowlist: &self.unknown_hle_allowlist,
                next_heap: &mut self.next_heap,
                frame_address: &mut frame_address,
                stop_requested: false,
                dynamic_imports: &mut self.dynamic_imports,
            };
            let pc = self.cpu.r[15];
            if let Err(error) = self.cpu.step(&mut bus) {
                return match error {
                    SimulatorError::MemoryError { .. } => Err(SimulatorError::CpuError {
                        pc,
                        message: format!("{:?} state: {error}", self.cpu.execution_state()),
                    }),
                    other => Err(other),
                };
            }
            if bus.stop_requested {
                self.stop();
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
            "vxGoHome" | "abort" | "av_end_thread" | "av_queue_abort" => self.stop_requested = true,
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
        self.memory.write32(address, value)
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
}
