use crate::a330_memory::{
    A330Memory, DYNAMIC_THUNK_BASE, EXIT_ADDRESS, FRAMEBUFFER_BASE, HEAP_SIZE, STACK_BASE,
    STACK_SIZE,
};
use crate::app_loader::PackageImage;
use crate::arm_cpu::{ArmBus, ArmCpu};
use crate::audio::{Audio, AudioConfig};
use crate::cheats::{CheatManager, CheatParseError, CheatRule};
use crate::content::{ArmProfile, ContentFormat};
use crate::cpu::UnknownInstructionPolicy;
use crate::emulator::{UnknownHleCall, UnknownHlePolicy};
use crate::error::{Result, SimulatorError};
use crate::firmware_archive::FirmwareArchive;
use crate::input::{
    Input, BUTTON_A, BUTTON_B, BUTTON_DOWN, BUTTON_L, BUTTON_LEFT, BUTTON_R, BUTTON_RIGHT,
    BUTTON_SELECT, BUTTON_START, BUTTON_UP, BUTTON_X, BUTTON_Y,
};
use crate::video::{Video, FRAMEBUFFER_SIZE, SCREEN_HEIGHT, SCREEN_WIDTH};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::PathBuf;

const INSTRUCTIONS_PER_SLICE: u64 = 1_000_000;
const APP_PATH_ADDRESS: u32 = STACK_BASE + 0x200;
const LOCALE_ADDRESS: u32 = STACK_BASE + 0x600;
const LEGACY_FRAMEBUFFER_ADDRESS: u32 = 0x1180_0000;
const LEGACY_GRAPHICS_SURFACE: u32 = 0x0930_201c;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct GuestFile {
    data: Vec<u8>,
    position: usize,
    data_address: u32,
    save_path: Option<PathBuf>,
    writable: bool,
    dirty: bool,
}

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
struct HeapBlock {
    address: u32,
    size: u32,
    free: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct GuestHeap {
    base: u32,
    cursor: u32,
    blocks: Vec<HeapBlock>,
}

impl GuestHeap {
    fn new(base: u32) -> Self {
        Self {
            base,
            cursor: base,
            blocks: Vec::new(),
        }
    }

    fn allocate(&mut self, requested: u32) -> u32 {
        let Some(size) = requested.max(1).checked_add(7).map(|size| size & !7) else {
            return 0;
        };
        if let Some(index) = self
            .blocks
            .iter()
            .position(|block| block.free && block.size >= size)
        {
            let address = self.blocks[index].address;
            let remaining = self.blocks[index].size - size;
            self.blocks[index].size = size;
            self.blocks[index].free = false;
            if remaining >= 8 {
                self.blocks.insert(
                    index + 1,
                    HeapBlock {
                        address: address + size,
                        size: remaining,
                        free: true,
                    },
                );
            } else {
                self.blocks[index].size += remaining;
            }
            return address;
        }

        let heap_end = self.base.saturating_add(HEAP_SIZE as u32);
        let Some(end) = self.cursor.checked_add(size) else {
            return 0;
        };
        if end > heap_end {
            return 0;
        }
        let address = self.cursor;
        self.cursor = end;
        self.blocks.push(HeapBlock {
            address,
            size,
            free: false,
        });
        address
    }

    fn deallocate(&mut self, address: u32) {
        let Some(mut index) = self
            .blocks
            .iter()
            .position(|block| block.address == address && !block.free)
        else {
            return;
        };
        self.blocks[index].free = true;
        if index + 1 < self.blocks.len() && self.blocks[index + 1].free {
            let next = self.blocks.remove(index + 1);
            self.blocks[index].size += next.size;
        }
        if index > 0 && self.blocks[index - 1].free {
            let current = self.blocks.remove(index);
            index -= 1;
            self.blocks[index].size += current.size;
        }
        while self.blocks.last().is_some_and(|block| {
            block.free && block.address.checked_add(block.size) == Some(self.cursor)
        }) {
            let block = self.blocks.pop().unwrap();
            self.cursor = block.address;
        }
    }

    fn reallocate(&mut self, memory: &mut A330Memory, address: u32, requested: u32) -> Result<u32> {
        if address == 0 {
            return Ok(self.allocate(requested));
        }
        if requested == 0 {
            self.deallocate(address);
            return Ok(0);
        }
        let Some(size) = requested.checked_add(7).map(|size| size & !7) else {
            return Ok(0);
        };
        let Some(index) = self
            .blocks
            .iter()
            .position(|block| block.address == address && !block.free)
        else {
            return Ok(0);
        };
        if self.blocks[index].size >= size {
            return Ok(address);
        }
        if index + 1 < self.blocks.len() && self.blocks[index + 1].free {
            let combined = self.blocks[index]
                .size
                .saturating_add(self.blocks[index + 1].size);
            if combined >= size {
                self.blocks.remove(index + 1);
                let remaining = combined - size;
                self.blocks[index].size = size;
                if remaining >= 8 {
                    self.blocks.insert(
                        index + 1,
                        HeapBlock {
                            address: address + size,
                            size: remaining,
                            free: true,
                        },
                    );
                } else {
                    self.blocks[index].size = combined;
                }
                return Ok(address);
            }
        }

        let old_size = self.blocks[index].size;
        let new_address = self.allocate(requested);
        if new_address == 0 {
            return Ok(0);
        }
        let length = old_size.min(requested) as usize;
        let data = memory.read_bytes(address, length)?.to_vec();
        memory.write_bytes(new_address, &data)?;
        self.deallocate(address);
        Ok(new_address)
    }

    fn snapshot_layout_is_valid(&self, base: u32) -> bool {
        if self.base != base || self.cursor < base || self.cursor > base + HEAP_SIZE as u32 {
            return false;
        }
        let mut expected_address = base;
        let mut previous_was_free = false;
        for block in &self.blocks {
            if block.address != expected_address
                || block.size < 8
                || block.size % 8 != 0
                || previous_was_free && block.free
            {
                return false;
            }
            let Some(end) = block.address.checked_add(block.size) else {
                return false;
            };
            if end > self.cursor {
                return false;
            }
            expected_address = end;
            previous_was_free = block.free;
        }
        expected_address == self.cursor
    }
}

#[derive(serde::Serialize)]
struct A330StateRef<'a> {
    cpu: &'a ArmCpu,
    memory: &'a A330Memory,
    video: &'a Video,
    audio: &'a Audio,
    input: &'a Input,
    heap: &'a GuestHeap,
    running: bool,
    boot_complete: bool,
    dynamic_imports: &'a [String],
    tasks: &'a VecDeque<(ArmCpu, u32)>,
    current_priority: u32,
    files: BTreeMap<u32, GuestFile>,
    next_file_handle: u32,
    semaphores: &'a BTreeMap<u32, u32>,
    active_framebuffer: u32,
    framebuffer_bits: u32,
}

#[derive(serde::Deserialize)]
struct A330State {
    cpu: ArmCpu,
    memory: A330Memory,
    video: Video,
    audio: Audio,
    input: Input,
    heap: GuestHeap,
    running: bool,
    boot_complete: bool,
    dynamic_imports: Vec<String>,
    tasks: VecDeque<(ArmCpu, u32)>,
    current_priority: u32,
    files: BTreeMap<u32, GuestFile>,
    next_file_handle: u32,
    semaphores: BTreeMap<u32, u32>,
    active_framebuffer: u32,
    framebuffer_bits: u32,
}

pub(crate) struct A330Runtime {
    package: PackageImage,
    pub(crate) cpu: ArmCpu,
    pub(crate) memory: A330Memory,
    pub(crate) video: Video,
    pub(crate) audio: Audio,
    pub(crate) input: Input,
    cheats: CheatManager,
    unknown_hle_calls: BTreeMap<String, UnknownHleCall>,
    unknown_hle_policy: UnknownHlePolicy,
    unknown_hle_allowlist: BTreeSet<String>,
    heap: GuestHeap,
    running: bool,
    boot_complete: bool,
    app_main: Option<u32>,
    dynamic_imports: Vec<String>,
    tasks: VecDeque<(ArmCpu, u32)>,
    current_priority: u32,
    content_directory: PathBuf,
    save_directory: Option<PathBuf>,
    files: BTreeMap<u32, GuestFile>,
    next_file_handle: u32,
    semaphores: BTreeMap<u32, u32>,
    active_framebuffer: u32,
    framebuffer_bits: u32,
    firmware_archive: Option<FirmwareArchive>,
}

impl A330Runtime {
    pub(crate) fn from_package(package: PackageImage, _path: PathBuf) -> Result<Self> {
        let mut memory = A330Memory::from_package(&package)?;
        let cpu = ArmCpu::new(
            package.entry_point(),
            STACK_BASE + STACK_SIZE as u32 - 0x1000,
            EXIT_ADDRESS,
        );
        let heap = GuestHeap::new(memory.heap_base());
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
        let save_directory =
            (!content_directory.as_os_str().is_empty()).then(|| content_directory.clone());
        let firmware_archive = FirmwareArchive::discover(&content_directory);
        Ok(Self {
            package,
            cpu,
            memory,
            video: Video::new(),
            audio: Audio::new(),
            input: Input::new(),
            cheats: CheatManager::default(),
            unknown_hle_calls: BTreeMap::new(),
            unknown_hle_policy: UnknownHlePolicy::default(),
            unknown_hle_allowlist: BTreeSet::new(),
            heap,
            running: false,
            boot_complete: false,
            app_main,
            dynamic_imports: Vec::new(),
            tasks: VecDeque::new(),
            current_priority: 0,
            content_directory,
            save_directory,
            files: BTreeMap::new(),
            next_file_handle: 1,
            semaphores: BTreeMap::new(),
            active_framebuffer: FRAMEBUFFER_BASE,
            framebuffer_bits: 16,
            firmware_archive,
        })
    }

    pub(crate) fn start(&mut self) {
        self.running = true;
        self.cpu.start();
    }
    pub(crate) fn stop(&mut self) {
        self.running = false;
        self.cpu.stop();
        self.flush_save_files();
    }
    pub(crate) fn reset(&mut self) -> Result<()> {
        self.flush_save_files();
        let policy = self.unknown_hle_policy;
        let allowlist = self.unknown_hle_allowlist.clone();
        let content_directory = self.content_directory.clone();
        let save_directory = self.save_directory.clone();
        let cheats = self.cheats.clone();
        let instruction_policy = self.cpu.unknown_instruction_policy();
        let mut replacement = Self::from_package(self.package.clone(), PathBuf::new())?;
        replacement.unknown_hle_policy = policy;
        replacement.unknown_hle_allowlist = allowlist;
        replacement.content_directory = content_directory;
        replacement.save_directory = save_directory;
        replacement.firmware_archive = self.firmware_archive.clone();
        replacement.cheats = cheats;
        replacement
            .cpu
            .set_unknown_instruction_policy(instruction_policy);
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

    pub(crate) fn set_unknown_instruction_policy(&mut self, policy: UnknownInstructionPolicy) {
        self.cpu.set_unknown_instruction_policy(policy);
        for (cpu, _) in &mut self.tasks {
            cpu.set_unknown_instruction_policy(policy);
        }
    }

    pub(crate) fn flush_save_files(&mut self) {
        for (handle, file) in &mut self.files {
            if let Err(error) = flush_guest_file(file) {
                log::error!("Failed to flush ARM guest save file {handle}: {error}");
            }
        }
    }

    pub(crate) fn set_save_directory<P: Into<PathBuf>>(&mut self, directory: P) {
        self.flush_save_files();
        self.save_directory = Some(directory.into());
    }

    pub(crate) fn serialized_state_size(&self) -> usize {
        crate::save_state::A330_SERIALIZED_SIZE
    }

    pub(crate) fn serialize_state(&self, output: &mut [u8]) -> anyhow::Result<()> {
        let mut files = self.files.clone();
        for file in files.values_mut() {
            let Some(path) = file.save_path.take() else {
                continue;
            };
            let root = self
                .save_directory
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("save file has no configured save directory"))?;
            let relative = path
                .strip_prefix(root)
                .map_err(|_| anyhow::anyhow!("save file is outside the configured directory"))?;
            if !safe_relative_path(relative) {
                anyhow::bail!("save file has an unsafe relative path");
            }
            file.save_path = Some(relative.to_path_buf());
        }
        let state = A330StateRef {
            cpu: &self.cpu,
            memory: &self.memory,
            video: &self.video,
            audio: &self.audio,
            input: &self.input,
            heap: &self.heap,
            running: self.running,
            boot_complete: self.boot_complete,
            dynamic_imports: &self.dynamic_imports,
            tasks: &self.tasks,
            current_priority: self.current_priority,
            files,
            next_file_handle: self.next_file_handle,
            semaphores: &self.semaphores,
            active_framebuffer: self.active_framebuffer,
            framebuffer_bits: self.framebuffer_bits,
        };
        crate::save_state::encode_a330(&state, crc32fast::hash(&self.package.data), output)
    }

    pub(crate) fn unserialize_state(&mut self, input: &[u8]) -> anyhow::Result<()> {
        let mut state: A330State =
            crate::save_state::decode_a330(input, crc32fast::hash(&self.package.data))?;
        let profile = self.profile();
        if !state.memory.snapshot_layout_is_valid(profile)
            || !state.video.snapshot_layout_is_valid()
            || !state
                .heap
                .snapshot_layout_is_valid(state.memory.heap_base())
        {
            anyhow::bail!("save state has an incompatible A330 memory layout");
        }
        let max_dynamic_imports = (EXIT_ADDRESS - DYNAMIC_THUNK_BASE) as usize / 8;
        if state.dynamic_imports.len() > max_dynamic_imports {
            anyhow::bail!("save state has too many dynamic imports");
        }
        if !matches!(state.framebuffer_bits, 16 | 32) {
            anyhow::bail!("save state has an invalid framebuffer depth");
        }
        for file in state.files.values_mut() {
            let Some(relative) = file.save_path.take() else {
                continue;
            };
            if !safe_relative_path(&relative) {
                anyhow::bail!("save state contains an unsafe save path");
            }
            let root = self
                .save_directory
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("save state requires a save directory"))?;
            file.save_path = Some(root.join(relative));
        }

        #[cfg(feature = "standalone")]
        let host_audio_output_enabled = self.audio.host_output_enabled();
        self.cpu = state.cpu;
        self.memory = state.memory;
        self.video = state.video;
        self.audio = state.audio;
        #[cfg(feature = "standalone")]
        self.audio
            .set_host_output_enabled(host_audio_output_enabled);
        self.audio.resume_after_state_load();
        self.input = state.input;
        self.heap = state.heap;
        self.running = state.running;
        self.boot_complete = state.boot_complete;
        self.dynamic_imports = state.dynamic_imports;
        self.tasks = state.tasks;
        self.current_priority = state.current_priority;
        self.files = state.files;
        self.next_file_handle = state.next_file_handle;
        self.semaphores = state.semaphores;
        self.active_framebuffer = state.active_framebuffer;
        self.framebuffer_bits = state.framebuffer_bits;
        Ok(())
    }

    pub(crate) fn set_cheat(
        &mut self,
        index: u32,
        enabled: bool,
        code: &str,
    ) -> std::result::Result<(), CheatParseError> {
        self.cheats.set_arm_slot(index, enabled, code, &self.memory)
    }

    pub(crate) fn set_parsed_cheat(
        &mut self,
        index: u32,
        enabled: bool,
        rule: CheatRule,
    ) -> std::result::Result<(), CheatParseError> {
        self.cheats
            .set_parsed_arm_rule(index, enabled, rule, &self.memory)
    }

    pub(crate) fn clear_cheats(&mut self) {
        self.cheats.clear();
    }

    pub(crate) fn tick(&mut self) -> Result<()> {
        if !self.is_running() {
            return Ok(());
        }
        self.cheats.apply_arm(&mut self.memory, &mut self.cpu);
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
                    package: &self.package,
                    imports: &self.package.imports,
                    profile,
                    unknown_hle_calls: &mut self.unknown_hle_calls,
                    unknown_hle_policy: self.unknown_hle_policy,
                    unknown_hle_allowlist: &self.unknown_hle_allowlist,
                    heap: &mut self.heap,
                    frame_address: &mut frame_address,
                    stop_requested: false,
                    dynamic_imports: &mut self.dynamic_imports,
                    tasks: &mut self.tasks,
                    current_priority: self.current_priority,
                    yield_requested: false,
                    finish_current: false,
                    content_directory: &self.content_directory,
                    save_directory: self.save_directory.as_deref(),
                    files: &mut self.files,
                    next_file_handle: &mut self.next_file_handle,
                    semaphores: &mut self.semaphores,
                    active_framebuffer: &mut self.active_framebuffer,
                    framebuffer_bits: &mut self.framebuffer_bits,
                    audio: &mut self.audio,
                    input: &mut self.input,
                    firmware_archive: self.firmware_archive.as_ref(),
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
            self.present_frame(address)?;
        }
        self.audio.advance_frame();
        Ok(())
    }

    fn present_frame(&mut self, address: u32) -> Result<()> {
        if self.framebuffer_bits == 32 {
            let source = self.memory.read_bytes(address, FRAMEBUFFER_SIZE * 2)?;
            let (source_pixels, _) = source.as_chunks::<4>();
            let (destination_pixels, _) = self.video.framebuffer_mut().as_chunks_mut::<2>();
            for (destination, pixel) in destination_pixels.iter_mut().zip(source_pixels) {
                let blue = u16::from(pixel[0]);
                let green = u16::from(pixel[1]);
                let red = u16::from(pixel[2]);
                let rgb565 = ((red >> 3) << 11) | ((green >> 2) << 5) | (blue >> 3);
                destination.copy_from_slice(&rgb565.to_le_bytes());
            }
        } else {
            let source = self.memory.read_bytes(address, FRAMEBUFFER_SIZE)?;
            self.video.framebuffer_mut().copy_from_slice(source);
        }
        self.video.advance_frame();
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
    package: &'a PackageImage,
    imports: &'a [crate::app_loader::SymbolEntry],
    profile: ArmProfile,
    unknown_hle_calls: &'a mut BTreeMap<String, UnknownHleCall>,
    unknown_hle_policy: UnknownHlePolicy,
    unknown_hle_allowlist: &'a BTreeSet<String>,
    heap: &'a mut GuestHeap,
    frame_address: &'a mut Option<u32>,
    stop_requested: bool,
    dynamic_imports: &'a mut Vec<String>,
    tasks: &'a mut VecDeque<(ArmCpu, u32)>,
    current_priority: u32,
    yield_requested: bool,
    finish_current: bool,
    content_directory: &'a std::path::Path,
    save_directory: Option<&'a std::path::Path>,
    files: &'a mut BTreeMap<u32, GuestFile>,
    next_file_handle: &'a mut u32,
    semaphores: &'a mut BTreeMap<u32, u32>,
    active_framebuffer: &'a mut u32,
    framebuffer_bits: &'a mut u32,
    audio: &'a mut Audio,
    input: &'a mut Input,
    firmware_archive: Option<&'a FirmwareArchive>,
}

impl RuntimeBus<'_> {
    fn dispatch(&mut self, cpu: &mut ArmCpu, immediate: u32) -> Result<()> {
        if immediate == 0x0012_3456 {
            return self.dispatch_semihosting(cpu);
        }
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
            "lcd_get_frame" | "_lcd_get_frame" | "LCDGetFB" => cpu.r[0] = *self.active_framebuffer,
            "LCDGetWidth" | "get_lcd_width" => cpu.r[0] = SCREEN_WIDTH,
            "LCDGetHeight" | "get_lcd_height" => cpu.r[0] = SCREEN_HEIGHT,
            "lcd_set_frame" | "_lcd_set_frame" | "LCDFlushFB" | "LCDFlushFBZoom" => {
                *self.frame_address = Some(*self.active_framebuffer);
                cpu.r[0] = 0;
            }
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
            "ferror" | "fsys_ferror" => cpu.r[0] = u32::from(!self.files.contains_key(&cpu.r[0])),
            "dl_res_open" => cpu.r[0] = self.open_resource([cpu.r[2], cpu.r[1], cpu.r[0]]),
            "dl_res_get_size" => {
                cpu.r[0] = self
                    .files
                    .get(&cpu.r[0])
                    .map_or(0, |file| file.data.len() as u32)
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
            "_kbd_get_status" | "kbd_get_status" | "_rmt_get_status" | "rmt_get_status" => {
                let address = cpu.r[0];
                let (pressed, released, status) = self.input.take_status();
                let pressed = map_a330_input(self.profile, pressed);
                let released = map_a330_input(self.profile, released);
                let status = map_a330_input(self.profile, status);
                self.memory.write32(address, pressed)?;
                self.memory.write32(address.wrapping_add(4), released)?;
                self.memory.write32(address.wrapping_add(8), status)?;
                cpu.r[0] = 0;
            }
            "_kbd_get_key" | "kbd_get_key" | "_rmt_get_key" | "rmt_get_key" | "sys_get_key"
            | "KBDGetSKey" | "KBDGetSKeyStatus" | "RMTGetSKey" => {
                cpu.r[0] = map_a330_input(self.profile, self.input.buttons());
            }
            "_sys_judge_event" | "sys_judge_event" => {
                cpu.r[0] = u32::from(self.input.take_pending_event());
            }
            "printf" | "fprintf" => cpu.r[0] = 0,
            "stricmp" | "strcasecmp" => {
                let left = self.read_c_string(cpu.r[0], 4096)?;
                let right = self.read_c_string(cpu.r[1], 4096)?;
                cpu.r[0] = compare_ascii_case_insensitive(&left, &right) as u32;
            }
            "_waveout_open" | "waveout_open" => {
                let address = cpu.r[0];
                let config = AudioConfig::new(
                    self.memory.read32(address)?,
                    self.memory.read16(address + 4)?,
                    self.memory.read8(address + 6)?,
                    self.memory.read8(address + 7)?,
                );
                cpu.r[0] = u32::from(config.is_some_and(|config| self.audio.open(config)));
            }
            "waveout_write" => {
                let buffer = cpu.r[1];
                let count = cpu.r[2];
                if count == 0 || count > 4 * 1024 * 1024 {
                    cpu.r[0] = 0;
                } else if !self.audio.can_write() && self.profile == ArmProfile::Retail {
                    cpu.r[15] = cpu.r[15].wrapping_sub(4);
                    self.yield_requested = true;
                } else {
                    let data = self.memory.read_bytes(buffer, count as usize)?;
                    cpu.r[0] = u32::from(self.audio.write(data));
                }
            }
            "waveout_try_write" => {
                let count = cpu.r[2];
                cpu.r[0] = if count == 0 || count > 4 * 1024 * 1024 || !self.audio.can_write() {
                    0
                } else {
                    let data = self.memory.read_bytes(cpu.r[1], count as usize)?;
                    u32::from(self.audio.write(data))
                };
            }
            "waveout_can_write" | "waveout_can_write_nonblocking" | "pcm_can_write" => {
                cpu.r[0] = u32::from(self.audio.can_write())
            }
            "waveout_close" | "waveout_close_at_once" => cpu.r[0] = u32::from(self.audio.close()),
            "_waveout_set_volume" | "waveout_set_volume" => {
                cpu.r[0] = u32::from(self.audio.set_volume(cpu.r[0]))
            }
            "HP_Mute_sw" | "waveout_mute" => {
                cpu.r[0] = u32::from(self.audio.set_muted(cpu.r[0] != 0))
            }
            "vxGoHome" | "abort" | "av_end_thread" | "av_queue_abort" => self.stop_requested = true,
            "OSTaskCreate" => {
                if cpu.r[0] != 0 && cpu.r[2] != 0 {
                    let mut task = ArmCpu::new(cpu.r[0], cpu.r[2], EXIT_ADDRESS);
                    task.set_unknown_instruction_policy(cpu.unknown_instruction_policy());
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
            "LCDSetFBBit" => {
                if matches!(cpu.r[0], 16 | 32) {
                    *self.framebuffer_bits = cpu.r[0];
                }
                cpu.r[0] = 0;
            }
            "BMF_SetLcdFramePtr" => {
                if self.memory.read_bytes(cpu.r[0], FRAMEBUFFER_SIZE).is_ok() {
                    *self.active_framebuffer = cpu.r[0];
                }
                cpu.r[0] = 0;
            }
            "SysLcdClear" => {
                let clear = vec![0; FRAMEBUFFER_SIZE];
                self.memory.write_bytes(FRAMEBUFFER_BASE, &clear)?;
                self.memory
                    .write_bytes(LEGACY_FRAMEBUFFER_ADDRESS, &clear)?;
                cpu.r[0] = 0;
            }
            "FlushDCache" | "__dcache_writeback_all" => {
                if self.memory.read_bytes(cpu.r[0], FRAMEBUFFER_SIZE).is_ok() {
                    *self.active_framebuffer = cpu.r[0];
                    *self.frame_address = Some(cpu.r[0]);
                }
                cpu.r[0] = 0;
            }
            "BMF_SelectPixelFunc" => cpu.r[0] = 0,
            "LCDEnableDoubleFB" | "LCDDisableDoubleFB" | "LCDSetFBFormat" | "LCDInit"
            | "LCDSetRefreshRate" | "LCDSetBrightness" | "InvalidICache" | "fsys_RefreshCache"
            | "consoleEnable" | "consoleDisable" | "PMSetMode" => cpu.r[0] = 0,
            _ => self.record_unknown(cpu, &symbol_name, symbol_address)?,
        }
        if self.profile == ArmProfile::Homebrew {
            cpu.r[15] = cpu.r[14] & !1;
        }
        Ok(())
    }

    fn dispatch_semihosting(&mut self, cpu: &mut ArmCpu) -> Result<()> {
        match cpu.r[0] {
            0x03 => {
                let value = self.memory.read8(cpu.r[1])?;
                log::trace!("ARM semihosting SYS_WRITEC: {:?}", char::from(value));
                cpu.r[0] = 0;
            }
            0x04 => {
                let value = self.read_c_string(cpu.r[1], 4096)?;
                log::trace!("ARM semihosting SYS_WRITE0: {value:?}");
                cpu.r[0] = 0;
            }
            0x18 | 0x20 => {
                log::trace!(
                    "ARM semihosting exit operation={:#04x}, reason={:#010x}",
                    cpu.r[0],
                    cpu.r[1]
                );
                self.stop_requested = true;
                cpu.r[0] = 0;
            }
            operation => {
                return Err(SimulatorError::SdkHleError(format!(
                    "unsupported ARM semihosting operation {operation:#010x} with parameter {:#010x}",
                    cpu.r[1]
                )));
            }
        }
        Ok(())
    }

    fn allocate(&mut self, size: u32) -> u32 {
        self.heap.allocate(size)
    }

    fn allocate_zeroed(&mut self, size: u32) -> Result<u32> {
        let address = self.allocate(size);
        if address != 0 && size != 0 {
            self.memory.write_bytes(address, &vec![0; size as usize])?;
        }
        Ok(address)
    }

    fn deallocate(&mut self, address: u32) {
        self.heap.deallocate(address);
    }

    fn reallocate(&mut self, address: u32, size: u32) -> Result<u32> {
        self.heap.reallocate(self.memory, address, size)
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

    fn insert_file(
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

    fn open_resource(&mut self, candidates: [u32; 3]) -> u32 {
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

    fn read_resource(
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

    fn read_file(&mut self, destination: u32, size: u32, count: u32, handle: u32) -> Result<u32> {
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

    fn write_file(&mut self, source: u32, size: u32, count: u32, handle: u32) -> Result<u32> {
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

    fn close_file(&mut self, handle: u32) -> u32 {
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

fn flush_guest_file(file: &mut GuestFile) -> std::io::Result<()> {
    if !file.dirty {
        return Ok(());
    }
    let Some(path) = file.save_path.as_ref() else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, &file.data)?;
    file.dirty = false;
    Ok(())
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

fn safe_relative_path(path: &std::path::Path) -> bool {
    !path.as_os_str().is_empty()
        && path.components().all(|component| {
            matches!(
                component,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        })
}

fn map_a330_input(profile: ArmProfile, input: u32) -> u32 {
    const POWER: u32 = 0x80;

    let mut mapped = 0;
    let mappings: &[(u32, u32)] = match profile {
        ArmProfile::Retail => &[
            (BUTTON_UP, 0x0010_0000),
            (BUTTON_DOWN, 0x0800_0000),
            (BUTTON_LEFT, 0x1000_0000),
            (BUTTON_RIGHT, 0x0004_0000),
            (BUTTON_A, 0x8000_0000),
            (BUTTON_B, 0x0000_1000),
            (BUTTON_X, 0x0001_0000),
            (BUTTON_Y, 0x2000_0000),
            (BUTTON_START, 0x0000_8000),
            (BUTTON_SELECT, 0x0080_0000),
            (BUTTON_L, 0x0002_0000),
            (BUTTON_R, 0x4000_0000),
            (POWER, POWER),
        ],
        ArmProfile::Homebrew => &[
            (BUTTON_UP, 0x0010_0000),
            (BUTTON_DOWN, 0x0800_0000),
            (BUTTON_LEFT, 0x1000_0000),
            (BUTTON_RIGHT, 0x0004_0000),
            (BUTTON_A, 0x8000_0000),
            (BUTTON_B, 0x0000_1000),
            (BUTTON_X, 0x2000_0000),
            (BUTTON_Y, 0x0001_0000),
            (BUTTON_START, 0x0000_0080),
            (BUTTON_SELECT, 0x0000_4000),
            (BUTTON_L, 0x0002_0000),
            (BUTTON_R, 0x4000_0000),
            (POWER, 0x0000_0001),
        ],
    };
    for &(source, destination) in mappings {
        if input & source != 0 {
            mapped |= destination;
        }
    }
    mapped
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
            *self.active_framebuffer = LEGACY_FRAMEBUFFER_ADDRESS;
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
    use crate::app_loader::{ChunkHeader, RawdHeader, ResourceEntry, ResourceKind, SymbolEntry};

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

    fn semihosting_package() -> PackageImage {
        let mut package = svc_package("unused");
        package.data[0x80..0x84].copy_from_slice(&0xef12_3456u32.to_le_bytes());
        package.imports.clear();
        package
    }

    #[test]
    fn a330_heap_reuses_and_coalesces_freed_blocks() {
        let mut heap = GuestHeap::new(0x2100_0000);
        let first = heap.allocate(16);
        let second = heap.allocate(16);
        let third = heap.allocate(16);

        heap.deallocate(second);
        assert_eq!(heap.allocate(8), second);
        heap.deallocate(first);
        heap.deallocate(second);
        heap.deallocate(third);

        assert_eq!(heap.allocate(48), first);
    }

    #[test]
    fn a330_realloc_preserves_data_and_the_original_on_failure() {
        let mut runtime =
            A330Runtime::from_package(svc_package("realloc"), PathBuf::new()).unwrap();
        let first = runtime.heap.allocate(8);
        runtime.memory.write32(first, 0x1234_5678).unwrap();
        runtime.heap.allocate(8);

        let moved = runtime
            .heap
            .reallocate(&mut runtime.memory, first, 16)
            .unwrap();
        assert_ne!(moved, first);
        assert_eq!(runtime.memory.read32(moved).unwrap(), 0x1234_5678);

        assert_eq!(
            runtime
                .heap
                .reallocate(&mut runtime.memory, moved, u32::MAX)
                .unwrap(),
            0
        );
        assert_eq!(runtime.memory.read32(moved).unwrap(), 0x1234_5678);
    }

    #[test]
    fn a330_cheats_apply_enabled_rules_and_survive_reset() {
        let mut runtime =
            A330Runtime::from_package(svc_package("LCDGetWidth"), PathBuf::new()).unwrap();
        runtime
            .set_cheat(0, true, "mem32:0x1ff00000=0x12345678")
            .unwrap();
        runtime.set_cheat(1, true, "reg:r4=0xfeedbeef").unwrap();
        runtime
            .set_cheat(2, false, "mem16:0x1ff00004=0xabcd")
            .unwrap();

        runtime.start();
        runtime.tick().unwrap();
        assert_eq!(runtime.memory.read32(STACK_BASE).unwrap(), 0x1234_5678);
        assert_eq!(runtime.memory.read16(STACK_BASE + 4).unwrap(), 0);
        assert_eq!(runtime.cpu.r[4], 0xfeed_beef);

        runtime.reset().unwrap();
        runtime.start();
        runtime.tick().unwrap();
        assert_eq!(runtime.memory.read32(STACK_BASE).unwrap(), 0x1234_5678);
        assert_eq!(runtime.cpu.r[4], 0xfeed_beef);
    }

    #[test]
    fn a330_cheats_validate_targets_and_can_be_removed() {
        let mut runtime =
            A330Runtime::from_package(svc_package("LCDGetWidth"), PathBuf::new()).unwrap();
        assert!(matches!(
            runtime.set_cheat(0, true, "mem32:0x04000000=1"),
            Err(CheatParseError::InvalidMemoryRange { .. })
        ));
        assert!(matches!(
            runtime.set_cheat(0, true, "reg:r16=1"),
            Err(CheatParseError::InvalidRegister(_))
        ));

        runtime
            .set_cheat(0, true, "mem32:0x1ff00000=0x12345678")
            .unwrap();
        runtime.set_cheat(0, true, "").unwrap();
        runtime.start();
        runtime.tick().unwrap();
        assert_eq!(runtime.memory.read32(STACK_BASE).unwrap(), 0);
    }

    #[test]
    fn a330_unknown_instruction_policy_updates_tasks_and_survives_reset() {
        let mut runtime =
            A330Runtime::from_package(svc_package("LCDGetWidth"), PathBuf::new()).unwrap();
        let mut task = ArmCpu::new(0x1010_1000, EXIT_ADDRESS - 0x100, EXIT_ADDRESS);
        task.start();
        runtime.tasks.push_back((task, 7));

        runtime.set_unknown_instruction_policy(UnknownInstructionPolicy::Stop);
        assert_eq!(
            runtime.cpu.unknown_instruction_policy(),
            UnknownInstructionPolicy::Stop
        );
        assert_eq!(
            runtime.tasks[0].0.unknown_instruction_policy(),
            UnknownInstructionPolicy::Stop
        );

        runtime.reset().unwrap();
        assert_eq!(
            runtime.cpu.unknown_instruction_policy(),
            UnknownInstructionPolicy::Stop
        );
    }

    #[test]
    fn a330_save_state_round_trip_is_complete_and_transactional() {
        let save_directory =
            std::env::temp_dir().join(format!("dingooemu-a330-state-{}", std::process::id()));
        let mut runtime =
            A330Runtime::from_package(svc_package("LCDGetWidth"), PathBuf::new()).unwrap();
        runtime.set_save_directory(&save_directory);
        runtime.cpu.r[4] = 0x1234_5678;
        runtime
            .memory
            .write32(STACK_BASE + 0x100, 0xaabb_ccdd)
            .unwrap();
        runtime.video.framebuffer_mut()[..2].copy_from_slice(&0xf800u16.to_le_bytes());
        runtime.input.set_buttons(BUTTON_A | BUTTON_START);
        let audio_config = AudioConfig::new(16_000, 16, 1, 40).unwrap();
        assert!(runtime.audio.open(audio_config));
        let allocation = runtime.heap.allocate(16);
        runtime.memory.write32(allocation, 0xfeed_beef).unwrap();
        runtime.running = true;
        runtime.cpu.start();
        runtime.boot_complete = true;
        runtime.dynamic_imports.push("dynamic_call".into());
        let mut task = ArmCpu::new(0x1010_1000, EXIT_ADDRESS - 0x100, EXIT_ADDRESS);
        task.r[5] = 0x55aa_55aa;
        task.start();
        runtime.tasks.push_back((task, 7));
        runtime.current_priority = 3;
        runtime.files.insert(
            17,
            GuestFile {
                data: vec![1, 2, 3, 4],
                position: 2,
                data_address: allocation,
                save_path: Some(save_directory.join("slot.dat")),
                writable: true,
                dirty: true,
            },
        );
        runtime.next_file_handle = 18;
        runtime.semaphores.insert(9, 2);
        runtime.active_framebuffer = FRAMEBUFFER_BASE + 0x1000;
        runtime.framebuffer_bits = 32;

        let mut state = vec![0; runtime.serialized_state_size()];
        runtime.serialize_state(&mut state).unwrap();

        runtime.cpu.r[4] = 0;
        runtime.memory.write32(STACK_BASE + 0x100, 0).unwrap();
        runtime.video.framebuffer_mut()[..2].fill(0);
        runtime.input.set_buttons(0);
        runtime.audio.close();
        runtime.heap = GuestHeap::new(runtime.memory.heap_base());
        runtime.running = false;
        runtime.cpu.stop();
        runtime.boot_complete = false;
        runtime.dynamic_imports.clear();
        runtime.tasks.clear();
        runtime.current_priority = 0;
        runtime.files.clear();
        runtime.next_file_handle = 1;
        runtime.semaphores.clear();
        runtime.active_framebuffer = FRAMEBUFFER_BASE;
        runtime.framebuffer_bits = 16;

        runtime.unserialize_state(&state).unwrap();
        assert_eq!(runtime.cpu.r[4], 0x1234_5678);
        assert_eq!(
            runtime.memory.read32(STACK_BASE + 0x100).unwrap(),
            0xaabb_ccdd
        );
        assert_eq!(&runtime.video.framebuffer()[..2], &0xf800u16.to_le_bytes());
        assert_eq!(runtime.input.buttons(), BUTTON_A | BUTTON_START);
        assert_eq!(runtime.audio.config(), Some(audio_config));
        assert_eq!(runtime.memory.read32(allocation).unwrap(), 0xfeed_beef);
        assert!(runtime.is_running());
        assert!(runtime.boot_complete);
        assert_eq!(runtime.dynamic_imports, ["dynamic_call"]);
        assert_eq!(runtime.tasks[0].0.r[5], 0x55aa_55aa);
        assert_eq!(runtime.tasks[0].1, 7);
        assert_eq!(runtime.current_priority, 3);
        assert_eq!(runtime.files[&17].data, [1, 2, 3, 4]);
        assert_eq!(runtime.files[&17].position, 2);
        assert_eq!(
            runtime.files[&17].save_path.as_deref(),
            Some(save_directory.join("slot.dat").as_path())
        );
        assert_eq!(runtime.next_file_handle, 18);
        assert_eq!(runtime.semaphores[&9], 2);
        assert_eq!(runtime.active_framebuffer, FRAMEBUFFER_BASE + 0x1000);
        assert_eq!(runtime.framebuffer_bits, 32);

        runtime
            .memory
            .write32(STACK_BASE + 0x100, 0xdead_beef)
            .unwrap();
        state[32] ^= 1;
        assert!(runtime.unserialize_state(&state).is_err());
        assert_eq!(
            runtime.memory.read32(STACK_BASE + 0x100).unwrap(),
            0xdead_beef
        );
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
    fn semihosting_console_output_returns_to_the_next_instruction() {
        let mut runtime = A330Runtime::from_package(semihosting_package(), PathBuf::new()).unwrap();
        runtime.memory.write8(STACK_BASE, b'X').unwrap();
        runtime.cpu.r[0] = 0x03;
        runtime.cpu.r[1] = STACK_BASE;
        runtime.start();
        runtime.tick().unwrap();

        assert_eq!(runtime.cpu.r[0], 0);
        assert_eq!(runtime.cpu.instruction_count, 2);
        assert!(!runtime.is_running());
    }

    #[test]
    fn semihosting_exit_stops_the_runtime() {
        let mut runtime = A330Runtime::from_package(semihosting_package(), PathBuf::new()).unwrap();
        runtime.cpu.r[0] = 0x18;
        runtime.cpu.r[1] = 0x0002_0026;
        runtime.start();
        runtime.tick().unwrap();

        assert_eq!(runtime.cpu.r[0], 0);
        assert_eq!(runtime.cpu.instruction_count, 1);
        assert!(!runtime.is_running());
    }

    #[test]
    fn a330_button_masks_follow_each_guest_profile() {
        let retail = [
            (BUTTON_UP, 0x0010_0000),
            (BUTTON_DOWN, 0x0800_0000),
            (BUTTON_LEFT, 0x1000_0000),
            (BUTTON_RIGHT, 0x0004_0000),
            (BUTTON_A, 0x8000_0000),
            (BUTTON_B, 0x0000_1000),
            (BUTTON_X, 0x0001_0000),
            (BUTTON_Y, 0x2000_0000),
            (BUTTON_START, 0x0000_8000),
            (BUTTON_SELECT, 0x0080_0000),
            (BUTTON_L, 0x0002_0000),
            (BUTTON_R, 0x4000_0000),
        ];
        let homebrew = [
            (BUTTON_UP, 0x0010_0000),
            (BUTTON_DOWN, 0x0800_0000),
            (BUTTON_LEFT, 0x1000_0000),
            (BUTTON_RIGHT, 0x0004_0000),
            (BUTTON_A, 0x8000_0000),
            (BUTTON_B, 0x0000_1000),
            (BUTTON_X, 0x2000_0000),
            (BUTTON_Y, 0x0001_0000),
            (BUTTON_START, 0x0000_0080),
            (BUTTON_SELECT, 0x0000_4000),
            (BUTTON_L, 0x0002_0000),
            (BUTTON_R, 0x4000_0000),
        ];
        for (source, expected) in retail {
            assert_eq!(map_a330_input(ArmProfile::Retail, source), expected);
        }
        for (source, expected) in homebrew {
            assert_eq!(map_a330_input(ArmProfile::Homebrew, source), expected);
        }
    }

    #[test]
    fn a330_status_poll_writes_translated_edges_and_state() {
        let mut runtime =
            A330Runtime::from_package(svc_package("kbd_get_status"), PathBuf::new()).unwrap();
        runtime.input.set_buttons(BUTTON_A | BUTTON_B);
        runtime.cpu.r[0] = STACK_BASE;
        runtime.start();
        runtime.tick().unwrap();
        assert_eq!(runtime.cpu.r[0], 0);
        assert_eq!(runtime.memory.read32(STACK_BASE).unwrap(), 0x8000_1000);
        assert_eq!(runtime.memory.read32(STACK_BASE + 4).unwrap(), 0);
        assert_eq!(runtime.memory.read32(STACK_BASE + 8).unwrap(), 0x8000_1000);
    }

    #[test]
    fn a330_event_poll_consumes_pending_input_event() {
        let mut runtime =
            A330Runtime::from_package(svc_package("sys_judge_event"), PathBuf::new()).unwrap();
        runtime.input.set_buttons(BUTTON_START);
        runtime.start();
        runtime.tick().unwrap();
        assert_eq!(runtime.cpu.r[0], 1);

        runtime.cpu = ArmCpu::new(ArmProfile::RETAIL_ORIGIN, EXIT_ADDRESS - 16, EXIT_ADDRESS);
        runtime.cpu.start();
        runtime.running = true;
        runtime.boot_complete = true;
        runtime.tick().unwrap();
        assert_eq!(runtime.cpu.r[0], 0);
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
    fn homebrew_file_handles_are_readable_guest_objects() {
        let directory = std::env::temp_dir().join(format!(
            "dingooemu-arm-homebrew-file-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("asset.bin"), [1, 2, 3, 4]).unwrap();
        let mut package = svc_package("fsys_fopen");
        package.format = ContentFormat::C2s;
        package.rawd.entry = ArmProfile::HOMEBREW_ORIGIN;
        package.rawd.origin = ArmProfile::HOMEBREW_ORIGIN;
        package.imports[0].address = ArmProfile::HOMEBREW_ORIGIN;
        let mut runtime = A330Runtime::from_package(package, directory.join("game.c2s")).unwrap();
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

        assert!(handle >= runtime.memory.heap_base());
        assert_eq!(runtime.memory.read32(handle).unwrap(), 1);
        assert_eq!(runtime.files[&handle].data, [1, 2, 3, 4]);

        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn writable_guest_files_persist_below_the_save_directory() {
        let directory = std::env::temp_dir().join(format!(
            "dingooemu-arm-save-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let save_directory = directory.join("saves");
        let mut runtime =
            A330Runtime::from_package(svc_package("fsys_fopen"), directory.join("game.c2s"))
                .unwrap();
        runtime.set_save_directory(&save_directory);
        runtime
            .memory
            .write_bytes(STACK_BASE, b"a:\\platform\\system.ini\0")
            .unwrap();
        runtime
            .memory
            .write_bytes(STACK_BASE + 64, b"wb\0")
            .unwrap();
        runtime.cpu.r[0] = STACK_BASE;
        runtime.cpu.r[1] = STACK_BASE + 64;
        runtime.start();
        runtime.tick().unwrap();
        let handle = runtime.cpu.r[0];
        assert_ne!(handle, 0);

        runtime.package.imports[0].name = "fsys_fwrite".into();
        runtime
            .memory
            .write_bytes(STACK_BASE + 128, b"language=0\n")
            .unwrap();
        runtime.cpu = ArmCpu::new(ArmProfile::RETAIL_ORIGIN, EXIT_ADDRESS - 16, EXIT_ADDRESS);
        runtime.cpu.r[0] = STACK_BASE + 128;
        runtime.cpu.r[1] = 1;
        runtime.cpu.r[2] = 11;
        runtime.cpu.r[3] = handle;
        runtime.cpu.start();
        runtime.running = true;
        runtime.boot_complete = true;
        runtime.tick().unwrap();
        assert_eq!(runtime.cpu.r[0], 11);

        runtime.package.imports[0].name = "fsys_fclose".into();
        runtime.cpu = ArmCpu::new(ArmProfile::RETAIL_ORIGIN, EXIT_ADDRESS - 16, EXIT_ADDRESS);
        runtime.cpu.r[0] = handle;
        runtime.cpu.start();
        runtime.running = true;
        runtime.tick().unwrap();
        assert_eq!(runtime.cpu.r[0], 0);
        assert_eq!(
            std::fs::read(save_directory.join("a/platform/system.ini")).unwrap(),
            b"language=0\n"
        );

        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn wide_guest_file_paths_use_an_ansi_mode_string() {
        let directory =
            std::env::temp_dir().join(format!("dingooemu-arm-wide-files-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("asset.bin"), [1, 2, 3, 4]).unwrap();

        let mut runtime =
            A330Runtime::from_package(svc_package("fsys_fopenW"), directory.join("game.c2s"))
                .unwrap();
        let mut wide_name = Vec::new();
        for value in "asset.bin".encode_utf16().chain(std::iter::once(0)) {
            wide_name.extend_from_slice(&value.to_le_bytes());
        }
        runtime.memory.write_bytes(STACK_BASE, &wide_name).unwrap();
        runtime
            .memory
            .write_bytes(STACK_BASE + 64, b"rb\0")
            .unwrap();
        runtime.cpu.r[0] = STACK_BASE;
        runtime.cpu.r[1] = STACK_BASE + 64;
        runtime.start();
        runtime.tick().unwrap();
        let handle = runtime.cpu.r[0];
        assert_ne!(handle, 0);
        assert_eq!(runtime.files[&handle].data, [1, 2, 3, 4]);

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

    #[test]
    fn cache_flush_submits_legacy_rgb565_frames() {
        let mut runtime =
            A330Runtime::from_package(svc_package("FlushDCache"), PathBuf::new()).unwrap();
        runtime
            .memory
            .write16(LEGACY_FRAMEBUFFER_ADDRESS, 0x07e0)
            .unwrap();
        runtime.cpu.r[0] = LEGACY_FRAMEBUFFER_ADDRESS;
        runtime.start();
        runtime.tick().unwrap();
        assert_eq!(&runtime.video.framebuffer()[..2], &[0xe0, 0x07]);
        assert_eq!(runtime.video.frame_count(), 1);
    }

    #[test]
    fn thirty_two_bit_guest_frames_are_converted_to_rgb565() {
        let mut runtime =
            A330Runtime::from_package(svc_package("FlushDCache"), PathBuf::new()).unwrap();
        runtime.framebuffer_bits = 32;
        runtime
            .memory
            .write32(LEGACY_FRAMEBUFFER_ADDRESS, 0x00ff_0000)
            .unwrap();
        runtime.cpu.r[0] = LEGACY_FRAMEBUFFER_ADDRESS;
        runtime.start();
        runtime.tick().unwrap();
        assert_eq!(&runtime.video.framebuffer()[..2], &[0x00, 0xf8]);
    }

    #[test]
    fn frame_submission_uses_the_configured_framebuffer() {
        let mut runtime =
            A330Runtime::from_package(svc_package("lcd_set_frame"), PathBuf::new()).unwrap();
        runtime.active_framebuffer = LEGACY_FRAMEBUFFER_ADDRESS;
        runtime
            .memory
            .write16(LEGACY_FRAMEBUFFER_ADDRESS, 0xf800)
            .unwrap();
        let unrelated_pointer = runtime.memory.heap_base();
        runtime.memory.write16(unrelated_pointer, 0x07e0).unwrap();
        runtime.cpu.r[0] = unrelated_pointer;
        runtime.start();
        runtime.tick().unwrap();
        assert_eq!(&runtime.video.framebuffer()[..2], &[0x00, 0xf8]);
        assert_eq!(runtime.active_framebuffer, LEGACY_FRAMEBUFFER_ADDRESS);
    }

    #[test]
    fn waveout_open_uses_the_guest_audio_configuration() {
        let mut runtime =
            A330Runtime::from_package(svc_package("waveout_open"), PathBuf::new()).unwrap();
        #[cfg(feature = "standalone")]
        runtime.audio.set_host_output_enabled(false);
        runtime.memory.write32(STACK_BASE, 16_000).unwrap();
        runtime.memory.write16(STACK_BASE + 4, 16).unwrap();
        runtime.memory.write8(STACK_BASE + 6, 1).unwrap();
        runtime.memory.write8(STACK_BASE + 7, 80).unwrap();
        runtime.cpu.r[0] = STACK_BASE;
        runtime.start();
        runtime.tick().unwrap();
        assert_eq!(runtime.cpu.r[0], 1);
        assert_eq!(runtime.audio.config(), AudioConfig::new(16_000, 16, 1, 80));
    }

    #[test]
    fn embedded_resources_are_found_and_decoded() {
        let mut package = svc_package("dl_res_open");
        let offset = package.data.len() as u32;
        package.data.extend_from_slice(&[0x41, 0x42, 0x43, 0x44]);
        package.resources.push(ResourceEntry {
            kind: ResourceKind::Erpt,
            name: "data/level.bin".into(),
            offset,
            size: 4,
            xor_key: 0x40,
        });
        let mut runtime = A330Runtime::from_package(package, PathBuf::new()).unwrap();
        runtime
            .memory
            .write_bytes(STACK_BASE, b"data\\level.bin\0")
            .unwrap();
        runtime.cpu.r[2] = STACK_BASE;
        runtime.start();
        runtime.tick().unwrap();
        let handle = runtime.cpu.r[0];
        assert_ne!(handle, 0);
        assert_eq!(runtime.files[&handle].data, [1, 2, 3, 4]);
    }
}
