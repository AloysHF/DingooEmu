use super::cpu::{Bus, Cpu};
use super::firmware_archive::FirmwareArchive;
use super::memory::{
    Memory, DYNAMIC_THUNK_BASE, EXIT_ADDRESS, FRAMEBUFFER_BASE, HEAP_SIZE, LEGACY_GRAPHICS_STRIDE,
    LEGACY_GRAPHICS_SURFACE, STACK_BASE, STACK_SIZE,
};
use crate::common::audio::{Audio, AudioConfig};
use crate::common::cheats::{CheatManager, CheatParseError, CheatRule};
use crate::common::execution::UnknownInstructionPolicy;
use crate::common::hle::{UnknownHleCall, UnknownHlePolicy};
use crate::common::input::{
    Input, BUTTON_A, BUTTON_B, BUTTON_DOWN, BUTTON_L, BUTTON_LEFT, BUTTON_R, BUTTON_RIGHT,
    BUTTON_SELECT, BUTTON_START, BUTTON_UP, BUTTON_X, BUTTON_Y,
};
use crate::common::video::{Video, FRAMEBUFFER_SIZE, SCREEN_HEIGHT, SCREEN_WIDTH};
use crate::content::{ArmProfile, ContentFormat};
use crate::error::{Result, SimulatorError};
use crate::package::PackageImage;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::PathBuf;

mod sdk_hle;

const INSTRUCTIONS_PER_SLICE: u64 = 1_000_000;
const APP_PATH_ADDRESS: u32 = STACK_BASE + 0x200;
const LOCALE_ADDRESS: u32 = STACK_BASE + 0x600;
const LEGACY_FRAMEBUFFER_ADDRESS: u32 = 0x1180_0000;
const FRAMEBUFFER_BITS_EXPLICIT: u32 = 1 << 31;
const FILE_SEARCH_NAME_OFFSET: u32 = 0x12;
const FILE_SEARCH_NAME_CAPACITY: usize = 256;
const CONSOLE_OUTPUT_LIMIT: usize = 4096;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct GuestFile {
    data: Vec<u8>,
    position: usize,
    data_address: u32,
    save_path: Option<PathBuf>,
    writable: bool,
    dirty: bool,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct FileSearch {
    entries: Vec<String>,
    next_index: usize,
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

    fn reallocate(&mut self, memory: &mut Memory, address: u32, requested: u32) -> Result<u32> {
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
    cpu: &'a Cpu,
    memory: &'a Memory,
    video: &'a Video,
    audio: &'a Audio,
    input: &'a Input,
    heap: &'a GuestHeap,
    running: bool,
    boot_complete: bool,
    dynamic_imports: &'a [String],
    tasks: &'a VecDeque<(Cpu, u32)>,
    current_priority: u32,
    files: BTreeMap<u32, GuestFile>,
    file_searches: &'a BTreeMap<u32, FileSearch>,
    next_file_handle: u32,
    semaphores: &'a BTreeMap<u32, u32>,
    active_framebuffer: u32,
    framebuffer_bits: u32,
}

#[derive(serde::Deserialize)]
struct A330State {
    cpu: Cpu,
    memory: Memory,
    video: Video,
    audio: Audio,
    input: Input,
    heap: GuestHeap,
    running: bool,
    boot_complete: bool,
    dynamic_imports: Vec<String>,
    tasks: VecDeque<(Cpu, u32)>,
    current_priority: u32,
    files: BTreeMap<u32, GuestFile>,
    file_searches: BTreeMap<u32, FileSearch>,
    next_file_handle: u32,
    semaphores: BTreeMap<u32, u32>,
    active_framebuffer: u32,
    framebuffer_bits: u32,
}

pub(crate) struct Runtime {
    package: PackageImage,
    pub(crate) cpu: Cpu,
    pub(crate) memory: Memory,
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
    tasks: VecDeque<(Cpu, u32)>,
    current_priority: u32,
    content_path: PathBuf,
    content_directory: PathBuf,
    save_directory: Option<PathBuf>,
    files: BTreeMap<u32, GuestFile>,
    file_searches: BTreeMap<u32, FileSearch>,
    next_file_handle: u32,
    semaphores: BTreeMap<u32, u32>,
    active_framebuffer: u32,
    framebuffer_bits: u32,
    firmware_archive: Option<FirmwareArchive>,
    console_output: Vec<u8>,
}

impl Runtime {
    pub(crate) fn from_package(package: PackageImage, path: PathBuf) -> Result<Self> {
        let mut memory = Memory::from_package(&package)?;
        let cpu = Cpu::new(
            package.entry_point(),
            STACK_BASE + STACK_SIZE as u32 - 0x1000,
            EXIT_ADDRESS,
        );
        let framebuffer_bits = match memory.profile() {
            ArmProfile::Retail => 16,
            ArmProfile::Homebrew => 32,
        };
        let heap = GuestHeap::new(memory.heap_base());
        let file_name = path
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
        let content_directory = path.parent().map(PathBuf::from).unwrap_or_default();
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
            content_path: path,
            content_directory,
            save_directory,
            files: BTreeMap::new(),
            file_searches: BTreeMap::new(),
            next_file_handle: 1,
            semaphores: BTreeMap::new(),
            active_framebuffer: FRAMEBUFFER_BASE,
            framebuffer_bits,
            firmware_archive,
            console_output: Vec::new(),
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

    fn present_early_exit(&mut self) {
        if !framebuffer_is_solid(self.video.framebuffer()) {
            return;
        }
        let output = String::from_utf8_lossy(&self.console_output);
        let detail = output
            .lines()
            .rev()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or("NO VISIBLE VIDEO FRAME WAS PRESENTED");
        log::warn!("A330 guest exited without leaving a visible frame: {detail}");
        render_early_exit_frame(&mut self.video, detail);
    }
    pub(crate) fn reset(&mut self) -> Result<()> {
        self.flush_save_files();
        let was_running = self.is_running();
        let policy = self.unknown_hle_policy;
        let allowlist = self.unknown_hle_allowlist.clone();
        let save_directory = self.save_directory.clone();
        let cheats = self.cheats.clone();
        let instruction_policy = self.cpu.unknown_instruction_policy();
        let mut replacement = Self::from_package(self.package.clone(), self.content_path.clone())?;
        replacement.unknown_hle_policy = policy;
        replacement.unknown_hle_allowlist = allowlist;
        replacement.save_directory = save_directory;
        replacement.firmware_archive = self.firmware_archive.clone();
        replacement.cheats = cheats;
        replacement
            .cpu
            .set_unknown_instruction_policy(instruction_policy);
        self.memory.copy_state_from(&replacement.memory);
        std::mem::swap(&mut replacement.memory, &mut self.memory);
        if was_running {
            replacement.start();
        }
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
        crate::common::save_state::A330_SERIALIZED_SIZE
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
            file_searches: &self.file_searches,
            next_file_handle: self.next_file_handle,
            semaphores: &self.semaphores,
            active_framebuffer: self.active_framebuffer,
            framebuffer_bits: self.framebuffer_bits,
        };
        crate::common::save_state::encode_a330(&state, crc32fast::hash(&self.package.data), output)
    }

    pub(crate) fn unserialize_state(&mut self, input: &[u8]) -> anyhow::Result<()> {
        let mut state: A330State =
            crate::common::save_state::decode_a330(input, crc32fast::hash(&self.package.data))?;
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
        if !matches!(state.framebuffer_bits & !FRAMEBUFFER_BITS_EXPLICIT, 16 | 32)
            || state.framebuffer_bits & !(FRAMEBUFFER_BITS_EXPLICIT | 32 | 16) != 0
        {
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
        self.memory.copy_state_from(&state.memory);
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
        self.file_searches = state.file_searches;
        self.next_file_handle = state.next_file_handle;
        self.semaphores = state.semaphores;
        self.active_framebuffer = state.active_framebuffer;
        self.framebuffer_bits = state.framebuffer_bits;
        self.console_output.clear();
        Ok(())
    }

    pub(crate) fn set_cheat(
        &mut self,
        index: u32,
        enabled: bool,
        code: &str,
    ) -> std::result::Result<(), CheatParseError> {
        super::cheats::set_slot(&mut self.cheats, index, enabled, code, &self.memory)
    }

    pub(crate) fn set_parsed_cheat(
        &mut self,
        index: u32,
        enabled: bool,
        rule: CheatRule,
    ) -> std::result::Result<(), CheatParseError> {
        super::cheats::set_parsed_rule(&mut self.cheats, index, enabled, rule, &self.memory)
    }

    pub(crate) fn clear_cheats(&mut self) {
        self.cheats.clear();
    }

    pub(crate) fn tick(&mut self) -> Result<()> {
        if !self.is_running() {
            return Ok(());
        }
        super::cheats::apply(&self.cheats, &mut self.memory, &mut self.cpu);
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
                        self.cpu = Cpu::new(entry, EXIT_ADDRESS - 16, EXIT_ADDRESS);
                        self.cpu.instruction_count = count;
                        self.cpu.r[0] = APP_PATH_ADDRESS;
                        self.cpu.start();
                        self.current_priority = 0;
                        continue;
                    }
                }
                if !self.activate_next_task() {
                    self.present_early_exit();
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
                    file_searches: &mut self.file_searches,
                    next_file_handle: &mut self.next_file_handle,
                    semaphores: &mut self.semaphores,
                    active_framebuffer: &mut self.active_framebuffer,
                    framebuffer_bits: &mut self.framebuffer_bits,
                    audio: &mut self.audio,
                    input: &mut self.input,
                    firmware_archive: self.firmware_archive.as_ref(),
                    console_output: &mut self.console_output,
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
                self.present_early_exit();
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
        let explicit = self.framebuffer_bits & FRAMEBUFFER_BITS_EXPLICIT != 0;
        let bits = self.framebuffer_bits & !FRAMEBUFFER_BITS_EXPLICIT;
        if bits == 32 && (explicit || address != LEGACY_FRAMEBUFFER_ADDRESS) {
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
    memory: &'a mut Memory,
    package: &'a PackageImage,
    imports: &'a [crate::package::SymbolEntry],
    profile: ArmProfile,
    unknown_hle_calls: &'a mut BTreeMap<String, UnknownHleCall>,
    unknown_hle_policy: UnknownHlePolicy,
    unknown_hle_allowlist: &'a BTreeSet<String>,
    heap: &'a mut GuestHeap,
    frame_address: &'a mut Option<u32>,
    stop_requested: bool,
    dynamic_imports: &'a mut Vec<String>,
    tasks: &'a mut VecDeque<(Cpu, u32)>,
    current_priority: u32,
    yield_requested: bool,
    finish_current: bool,
    content_directory: &'a std::path::Path,
    save_directory: Option<&'a std::path::Path>,
    files: &'a mut BTreeMap<u32, GuestFile>,
    file_searches: &'a mut BTreeMap<u32, FileSearch>,
    next_file_handle: &'a mut u32,
    semaphores: &'a mut BTreeMap<u32, u32>,
    active_framebuffer: &'a mut u32,
    framebuffer_bits: &'a mut u32,
    audio: &'a mut Audio,
    input: &'a mut Input,
    firmware_archive: Option<&'a FirmwareArchive>,
    console_output: &'a mut Vec<u8>,
}

fn framebuffer_is_solid(framebuffer: &[u8]) -> bool {
    let pixels = framebuffer.as_chunks::<2>().0;
    pixels
        .first()
        .is_none_or(|first| pixels.iter().all(|pixel| pixel == first))
}

fn render_early_exit_frame(video: &mut Video, detail: &str) {
    const BACKGROUND: u16 = 0x0841;
    const HEADER: u16 = 0xa800;
    const PANEL: u16 = 0x18c3;
    const TEXT: u16 = 0xffff;
    const ACCENT: u16 = 0xffe0;

    let framebuffer = video.framebuffer_mut();
    fill_rgb565(framebuffer, BACKGROUND);
    fill_rect(framebuffer, 0, 0, SCREEN_WIDTH as usize, 62, HEADER);
    fill_rect(framebuffer, 14, 74, SCREEN_WIDTH as usize - 28, 148, PANEL);
    draw_rect(framebuffer, 14, 74, SCREEN_WIDTH as usize - 28, 148, ACCENT);
    draw_text(framebuffer, 38, 21, "GUEST PROGRAM EXITED", TEXT, 2, 0);
    draw_text(
        framebuffer,
        29,
        88,
        "THE GUEST STOPPED WITHOUT LEAVING A VISIBLE VIDEO FRAME.",
        TEXT,
        1,
        43,
    );
    draw_text(framebuffer, 29, 124, "LAST MESSAGE:", ACCENT, 1, 0);
    draw_text(framebuffer, 29, 140, detail, TEXT, 1, 43);
    video.mark_dirty();
    video.advance_frame();
}

fn fill_rgb565(framebuffer: &mut [u8], color: u16) {
    let color = color.to_le_bytes();
    for pixel in framebuffer.as_chunks_mut::<2>().0 {
        *pixel = color;
    }
}

fn fill_rect(framebuffer: &mut [u8], x: usize, y: usize, width: usize, height: usize, color: u16) {
    for row in y..(y + height).min(SCREEN_HEIGHT as usize) {
        for column in x..(x + width).min(SCREEN_WIDTH as usize) {
            set_rgb565(framebuffer, column, row, color);
        }
    }
}

fn draw_rect(framebuffer: &mut [u8], x: usize, y: usize, width: usize, height: usize, color: u16) {
    fill_rect(framebuffer, x, y, width, 2, color);
    fill_rect(
        framebuffer,
        x,
        y + height.saturating_sub(2),
        width,
        2,
        color,
    );
    fill_rect(framebuffer, x, y, 2, height, color);
    fill_rect(
        framebuffer,
        x + width.saturating_sub(2),
        y,
        2,
        height,
        color,
    );
}

fn set_rgb565(framebuffer: &mut [u8], x: usize, y: usize, color: u16) {
    if x >= SCREEN_WIDTH as usize || y >= SCREEN_HEIGHT as usize {
        return;
    }
    let offset = (y * SCREEN_WIDTH as usize + x) * 2;
    framebuffer[offset..offset + 2].copy_from_slice(&color.to_le_bytes());
}

fn draw_text(
    framebuffer: &mut [u8],
    start_x: usize,
    start_y: usize,
    text: &str,
    color: u16,
    scale: usize,
    wrap_columns: usize,
) {
    let mut column = 0;
    let mut row = 0;
    for character in text.chars().take(256) {
        if character == '\r' {
            continue;
        }
        if character == '\n' || wrap_columns != 0 && column >= wrap_columns {
            row += 1;
            column = 0;
            if character == '\n' {
                continue;
            }
        }
        draw_glyph(
            framebuffer,
            start_x + column * 6 * scale,
            start_y + row * 9 * scale,
            character,
            color,
            scale,
        );
        column += 1;
    }
}

fn draw_glyph(
    framebuffer: &mut [u8],
    x: usize,
    y: usize,
    character: char,
    color: u16,
    scale: usize,
) {
    for (row, bits) in glyph_rows(character).into_iter().enumerate() {
        for column in 0..5 {
            if bits & (1 << (4 - column)) != 0 {
                fill_rect(
                    framebuffer,
                    x + column * scale,
                    y + row * scale,
                    scale,
                    scale,
                    color,
                );
            }
        }
    }
}

fn glyph_rows(character: char) -> [u8; 7] {
    match character.to_ascii_uppercase() {
        'A' => [14, 17, 17, 31, 17, 17, 17],
        'B' => [30, 17, 17, 30, 17, 17, 30],
        'C' => [15, 16, 16, 16, 16, 16, 15],
        'D' => [30, 17, 17, 17, 17, 17, 30],
        'E' => [31, 16, 16, 30, 16, 16, 31],
        'F' => [31, 16, 16, 30, 16, 16, 16],
        'G' => [15, 16, 16, 19, 17, 17, 15],
        'H' => [17, 17, 17, 31, 17, 17, 17],
        'I' => [31, 4, 4, 4, 4, 4, 31],
        'J' => [1, 1, 1, 1, 17, 17, 14],
        'K' => [17, 18, 20, 24, 20, 18, 17],
        'L' => [16, 16, 16, 16, 16, 16, 31],
        'M' => [17, 27, 21, 21, 17, 17, 17],
        'N' => [17, 25, 21, 19, 17, 17, 17],
        'O' => [14, 17, 17, 17, 17, 17, 14],
        'P' => [30, 17, 17, 30, 16, 16, 16],
        'Q' => [14, 17, 17, 17, 21, 18, 13],
        'R' => [30, 17, 17, 30, 20, 18, 17],
        'S' => [15, 16, 16, 14, 1, 1, 30],
        'T' => [31, 4, 4, 4, 4, 4, 4],
        'U' => [17, 17, 17, 17, 17, 17, 14],
        'V' => [17, 17, 17, 17, 17, 10, 4],
        'W' => [17, 17, 17, 21, 21, 21, 10],
        'X' => [17, 17, 10, 4, 10, 17, 17],
        'Y' => [17, 17, 10, 4, 4, 4, 4],
        'Z' => [31, 1, 2, 4, 8, 16, 31],
        '0' => [14, 17, 19, 21, 25, 17, 14],
        '1' => [4, 12, 4, 4, 4, 4, 14],
        '2' => [14, 17, 1, 2, 4, 8, 31],
        '3' => [30, 1, 1, 14, 1, 1, 30],
        '4' => [2, 6, 10, 18, 31, 2, 2],
        '5' => [31, 16, 16, 30, 1, 1, 30],
        '6' => [14, 16, 16, 30, 17, 17, 14],
        '7' => [31, 1, 2, 4, 8, 8, 8],
        '8' => [14, 17, 17, 14, 17, 17, 14],
        '9' => [14, 17, 17, 15, 1, 1, 14],
        ':' => [0, 4, 4, 0, 4, 4, 0],
        '.' => [0, 0, 0, 0, 0, 12, 12],
        '-' => [0, 0, 0, 31, 0, 0, 0],
        '/' => [1, 1, 2, 4, 8, 16, 16],
        '\\' => [16, 16, 8, 4, 2, 1, 1],
        '_' => [0, 0, 0, 0, 0, 0, 31],
        ' ' => [0; 7],
        _ => [14, 17, 1, 2, 4, 0, 4],
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

fn normalize_guest_search_pattern(pattern: &str) -> Option<(PathBuf, String)> {
    let mut normalized = pattern.replace('\\', "/");
    if normalized.as_bytes().get(1) == Some(&b':')
        && normalized
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphabetic)
    {
        normalized.drain(..2);
    }
    let mut components = normalized
        .trim_start_matches('/')
        .split('/')
        .filter(|component| !component.is_empty() && *component != ".")
        .collect::<Vec<_>>();
    if components
        .iter()
        .any(|component| *component == ".." || component.contains(['\0', ':']))
    {
        return None;
    }
    let file_pattern = components.pop().unwrap_or("*");
    if components
        .first()
        .is_some_and(|component| component.eq_ignore_ascii_case("GAME"))
    {
        components.remove(0);
    }
    let directory = components.iter().collect::<PathBuf>();
    Some((
        directory,
        if file_pattern.is_empty() {
            "*".to_string()
        } else {
            file_pattern.to_string()
        },
    ))
}

fn wildcard_matches(pattern: &str, name: &str) -> bool {
    let pattern = if pattern.eq_ignore_ascii_case("*.*") {
        "*"
    } else {
        pattern
    };
    let pattern = pattern.as_bytes();
    let name = name.as_bytes();
    let (mut pattern_index, mut name_index) = (0, 0);
    let (mut star_index, mut retry_name_index) = (None, 0);

    while name_index < name.len() {
        if pattern_index < pattern.len()
            && (pattern[pattern_index] == b'?'
                || pattern[pattern_index].eq_ignore_ascii_case(&name[name_index]))
        {
            pattern_index += 1;
            name_index += 1;
        } else if pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
            star_index = Some(pattern_index);
            pattern_index += 1;
            retry_name_index = name_index;
        } else if let Some(star) = star_index {
            retry_name_index += 1;
            name_index = retry_name_index;
            pattern_index = star + 1;
        } else {
            return false;
        }
    }
    while pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
        pattern_index += 1;
    }
    pattern_index == pattern.len()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::TargetDevice;
    use crate::package::{ChunkHeader, RawdHeader, ResourceEntry, ResourceKind, SymbolEntry};

    fn svc_package(name: &str) -> PackageImage {
        let origin = ArmProfile::RETAIL_ORIGIN;
        let mut data = vec![0; 0x88];
        data[0x80..0x84].copy_from_slice(&0xef00_0000u32.to_le_bytes());
        data[0x84..0x88].copy_from_slice(&0xe12f_ff1eu32.to_le_bytes());
        PackageImage {
            format: ContentFormat::Cc,
            target: TargetDevice::GemeiA330(ArmProfile::Retail),
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

    fn memory_c_string(memory: &Memory, address: u32) -> String {
        let bytes = (0..256)
            .map(|offset| memory.read8(address + offset).unwrap())
            .take_while(|&byte| byte != 0)
            .collect::<Vec<_>>();
        String::from_utf8(bytes).unwrap()
    }

    fn legacy_stride_package(stride: u32) -> PackageImage {
        let mut package = svc_package("unused");
        let origin = ArmProfile::HOMEBREW_ORIGIN;
        let words = [
            0xe59f_0008,
            0xe59f_1008,
            0xe580_1000,
            0xe12f_ff1e,
            LEGACY_GRAPHICS_STRIDE,
            stride,
        ];
        package.data.resize(0x80 + words.len() * 4, 0);
        for (index, word) in words.iter().enumerate() {
            let offset = 0x80 + index * 4;
            package.data[offset..offset + 4].copy_from_slice(&word.to_le_bytes());
        }
        package.rawd.base.size = words.len() as u32 * 4;
        package.target = TargetDevice::GemeiA330(ArmProfile::Homebrew);
        package.rawd.entry = origin;
        package.rawd.origin = origin;
        package.rawd.program_size = words.len() as u32 * 4;
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
        let mut runtime = Runtime::from_package(svc_package("realloc"), PathBuf::new()).unwrap();
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
            Runtime::from_package(svc_package("LCDGetWidth"), PathBuf::new()).unwrap();
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
            Runtime::from_package(svc_package("LCDGetWidth"), PathBuf::new()).unwrap();
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
            Runtime::from_package(svc_package("LCDGetWidth"), PathBuf::new()).unwrap();
        let system_ram_pointer = runtime.memory.system_ram().as_ptr();
        let video_ram_pointer = runtime.memory.framebuffer().as_ptr();
        let mut task = Cpu::new(0x1010_1000, EXIT_ADDRESS - 0x100, EXIT_ADDRESS);
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
        assert_eq!(runtime.memory.system_ram().as_ptr(), system_ram_pointer);
        assert_eq!(runtime.memory.framebuffer().as_ptr(), video_ram_pointer);
        assert_eq!(
            runtime.cpu.unknown_instruction_policy(),
            UnknownInstructionPolicy::Stop
        );
    }

    #[test]
    fn a330_reset_preserves_the_guest_content_path() {
        let content_path = PathBuf::from("games").join("original.c2s");
        let mut runtime =
            Runtime::from_package(svc_package("LCDGetWidth"), content_path.clone()).unwrap();

        assert_eq!(
            memory_c_string(&runtime.memory, LOCALE_ADDRESS),
            r".\original.c2s"
        );

        runtime.reset().unwrap();

        assert_eq!(runtime.content_path, content_path);
        assert_eq!(
            memory_c_string(&runtime.memory, LOCALE_ADDRESS),
            r".\original.c2s"
        );
    }

    #[test]
    fn a330_save_state_round_trip_is_complete_and_transactional() {
        let save_directory =
            std::env::temp_dir().join(format!("dingooemu-a330-state-{}", std::process::id()));
        let mut runtime =
            Runtime::from_package(svc_package("LCDGetWidth"), PathBuf::new()).unwrap();
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
        let mut task = Cpu::new(0x1010_1000, EXIT_ADDRESS - 0x100, EXIT_ADDRESS);
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

        let system_ram_pointer = runtime.memory.system_ram().as_ptr();
        let video_ram_pointer = runtime.memory.framebuffer().as_ptr();
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
        assert_eq!(runtime.memory.system_ram().as_ptr(), system_ram_pointer);
        assert_eq!(runtime.memory.framebuffer().as_ptr(), video_ram_pointer);
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
            Runtime::from_package(svc_package("LCDGetWidth"), PathBuf::new()).unwrap();
        runtime.start();
        runtime.tick().unwrap();
        assert_eq!(runtime.cpu.r[0], SCREEN_WIDTH);
        assert_eq!(runtime.cpu.instruction_count, 2);
        assert!(!runtime.is_running());
    }

    #[test]
    fn semihosting_console_output_returns_to_the_next_instruction() {
        let mut runtime = Runtime::from_package(semihosting_package(), PathBuf::new()).unwrap();
        runtime.memory.write8(STACK_BASE, b'X').unwrap();
        runtime.cpu.r[0] = 0x03;
        runtime.cpu.r[1] = STACK_BASE;
        runtime.start();
        runtime.tick().unwrap();

        assert_eq!(runtime.cpu.r[0], 0);
        assert_eq!(runtime.cpu.instruction_count, 2);
        assert_eq!(runtime.console_output, b"X");
        assert_eq!(runtime.video.frame_count(), 1);
        assert!(!runtime.is_running());
    }

    #[test]
    fn semihosting_exit_stops_the_runtime() {
        let mut runtime = Runtime::from_package(semihosting_package(), PathBuf::new()).unwrap();
        runtime.cpu.r[0] = 0x18;
        runtime.cpu.r[1] = 0x0002_0026;
        runtime.start();
        runtime.tick().unwrap();

        assert_eq!(runtime.cpu.r[0], 0);
        assert_eq!(runtime.cpu.instruction_count, 1);
        assert!(!runtime.is_running());
        assert_eq!(runtime.video.frame_count(), 1);
        assert!(runtime
            .video
            .framebuffer()
            .as_chunks::<2>()
            .0
            .iter()
            .any(|pixel| *pixel != [0, 0]));
    }

    #[test]
    fn early_exit_does_not_replace_a_presented_guest_frame() {
        let mut runtime = Runtime::from_package(semihosting_package(), PathBuf::new()).unwrap();
        runtime.video.framebuffer_mut()[..2].copy_from_slice(&0xf800_u16.to_le_bytes());
        runtime.video.advance_frame();
        let crc = runtime.video.framebuffer_crc32();

        runtime.present_early_exit();

        assert_eq!(runtime.video.frame_count(), 1);
        assert_eq!(runtime.video.framebuffer_crc32(), crc);
    }

    #[test]
    fn early_exit_replaces_a_presented_solid_frame() {
        let mut runtime = Runtime::from_package(semihosting_package(), PathBuf::new()).unwrap();
        runtime.video.advance_frame();

        runtime.present_early_exit();

        assert_eq!(runtime.video.frame_count(), 2);
        assert!(!framebuffer_is_solid(runtime.video.framebuffer()));
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
            Runtime::from_package(svc_package("kbd_get_status"), PathBuf::new()).unwrap();
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
            Runtime::from_package(svc_package("sys_judge_event"), PathBuf::new()).unwrap();
        runtime.input.set_buttons(BUTTON_START);
        runtime.start();
        runtime.tick().unwrap();
        assert_eq!(runtime.cpu.r[0], 1);

        runtime.cpu = Cpu::new(ArmProfile::RETAIL_ORIGIN, EXIT_ADDRESS - 16, EXIT_ADDRESS);
        runtime.cpu.start();
        runtime.running = true;
        runtime.boot_complete = true;
        runtime.tick().unwrap();
        assert_eq!(runtime.cpu.r[0], 0);
    }

    #[test]
    fn unknown_imports_are_aggregated_and_strict_mode_stops() {
        let mut runtime =
            Runtime::from_package(svc_package("unknown_call"), PathBuf::new()).unwrap();
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
            Runtime::from_package(svc_package("dl_get_proc"), PathBuf::new()).unwrap();
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

        runtime.cpu = Cpu::new(DYNAMIC_THUNK_BASE, EXIT_ADDRESS - 16, EXIT_ADDRESS);
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
            Runtime::from_package(svc_package("OSTaskCreate"), PathBuf::new()).unwrap();
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
            Runtime::from_package(svc_package("fsys_fopen"), directory.join("game.c2s")).unwrap();
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
        runtime.cpu = Cpu::new(ArmProfile::RETAIL_ORIGIN, EXIT_ADDRESS - 16, EXIT_ADDRESS);
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

        runtime.package.imports[0].name = "fsys_fseek".into();
        runtime.cpu = Cpu::new(ArmProfile::RETAIL_ORIGIN, EXIT_ADDRESS - 16, EXIT_ADDRESS);
        runtime.cpu.r[0] = handle;
        runtime.cpu.r[1] = 1;
        runtime.cpu.r[2] = 0;
        runtime.cpu.start();
        runtime.running = true;
        runtime.boot_complete = true;
        runtime.tick().unwrap();
        assert_eq!(runtime.cpu.r[0], 0);

        runtime.package.imports[0].name = "fsys_fread".into();
        runtime.cpu = Cpu::new(ArmProfile::RETAIL_ORIGIN, EXIT_ADDRESS - 16, EXIT_ADDRESS);
        runtime.cpu.r[0] = STACK_BASE + 64;
        runtime.cpu.r[1] = 1;
        runtime.cpu.r[2] = 2;
        runtime.cpu.r[3] = handle;
        runtime.cpu.start();
        runtime.running = true;
        runtime.boot_complete = true;
        runtime.tick().unwrap();
        assert_eq!(runtime.cpu.r[0], 2);
        assert_eq!(
            runtime.memory.read_bytes(STACK_BASE + 64, 2).unwrap(),
            [2, 3]
        );

        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn arm_file_search_enumerates_device_game_directory() {
        let directory =
            std::env::temp_dir().join(format!("dingooemu-arm-file-search-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("Alpha.gb"), b"first").unwrap();
        std::fs::write(directory.join("beta.GB"), b"second").unwrap();
        std::fs::write(directory.join("ignore.txt"), b"ignored").unwrap();

        let mut package = svc_package("fsys_findfirst");
        package.format = ContentFormat::C2s;
        package.target = TargetDevice::GemeiA330(ArmProfile::Homebrew);
        package.rawd.entry = ArmProfile::HOMEBREW_ORIGIN;
        package.rawd.origin = ArmProfile::HOMEBREW_ORIGIN;
        package.imports[0].address = ArmProfile::HOMEBREW_ORIGIN;
        let mut runtime = Runtime::from_package(package, directory.join("game.c2s")).unwrap();
        runtime
            .memory
            .write_bytes(STACK_BASE, b"A:\\GAME\\*.gb\0")
            .unwrap();
        let search_data = STACK_BASE + 0x100;
        runtime.cpu.r[0] = STACK_BASE;
        runtime.cpu.r[1] = 0x8000_0037;
        runtime.cpu.r[2] = search_data;
        runtime.start();
        runtime.tick().unwrap();

        assert_eq!(runtime.cpu.r[0], 0);
        assert_eq!(
            memory_c_string(&runtime.memory, search_data + FILE_SEARCH_NAME_OFFSET),
            "Alpha.gb"
        );

        runtime.package.imports[0].name = "fsys_findnext".into();
        for expected in [Some("beta.GB"), None] {
            runtime.cpu = Cpu::new(ArmProfile::HOMEBREW_ORIGIN, EXIT_ADDRESS - 16, EXIT_ADDRESS);
            runtime.cpu.r[0] = search_data;
            runtime.cpu.start();
            runtime.running = true;
            runtime.boot_complete = true;
            runtime.tick().unwrap();
            if let Some(name) = expected {
                assert_eq!(runtime.cpu.r[0], 0);
                assert_eq!(
                    memory_c_string(&runtime.memory, search_data + FILE_SEARCH_NAME_OFFSET),
                    name
                );
            } else {
                assert_eq!(runtime.cpu.r[0], u32::MAX);
            }
        }

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
        package.target = TargetDevice::GemeiA330(ArmProfile::Homebrew);
        package.rawd.entry = ArmProfile::HOMEBREW_ORIGIN;
        package.rawd.origin = ArmProfile::HOMEBREW_ORIGIN;
        package.imports[0].address = ArmProfile::HOMEBREW_ORIGIN;
        let mut runtime = Runtime::from_package(package, directory.join("game.c2s")).unwrap();
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
            Runtime::from_package(svc_package("fsys_fopen"), directory.join("game.c2s")).unwrap();
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
        runtime.cpu = Cpu::new(ArmProfile::RETAIL_ORIGIN, EXIT_ADDRESS - 16, EXIT_ADDRESS);
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
        runtime.cpu = Cpu::new(ArmProfile::RETAIL_ORIGIN, EXIT_ADDRESS - 16, EXIT_ADDRESS);
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
            Runtime::from_package(svc_package("fsys_fopenW"), directory.join("game.c2s")).unwrap();
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
            Runtime::from_package(svc_package("OSSemCreate"), PathBuf::new()).unwrap();
        runtime.cpu.r[0] = 1;
        runtime.start();
        runtime.tick().unwrap();
        let handle = runtime.cpu.r[0];
        assert_eq!(runtime.semaphores[&handle], 1);

        runtime.package.imports[0].name = "OSSemPend".into();
        runtime.cpu = Cpu::new(origin, EXIT_ADDRESS - 16, EXIT_ADDRESS);
        runtime.cpu.r[0] = handle;
        runtime.cpu.start();
        runtime.running = true;
        runtime.boot_complete = true;
        runtime.tick().unwrap();
        assert_eq!(runtime.semaphores[&handle], 0);

        runtime.cpu = Cpu::new(origin, EXIT_ADDRESS - 16, EXIT_ADDRESS);
        runtime.cpu.r[0] = handle;
        runtime.cpu.start();
        runtime.running = true;
        runtime.tick().unwrap();
        assert_eq!(runtime.cpu.r[15], origin);
        assert!(runtime.is_running());

        runtime.package.imports[0].name = "OSSemPost".into();
        runtime.cpu = Cpu::new(origin, EXIT_ADDRESS - 16, EXIT_ADDRESS);
        runtime.cpu.r[0] = handle;
        runtime.cpu.start();
        runtime.running = true;
        runtime.tick().unwrap();
        assert_eq!(runtime.semaphores[&handle], 1);
    }

    #[test]
    fn cache_flush_submits_legacy_rgb565_frames() {
        let mut runtime =
            Runtime::from_package(svc_package("FlushDCache"), PathBuf::new()).unwrap();
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
    fn legacy_stride_selects_framebuffer_depth() {
        for (stride, expected_bits) in [(SCREEN_WIDTH * 2, 16), (SCREEN_WIDTH * 4, 32)] {
            let mut runtime =
                Runtime::from_package(legacy_stride_package(stride), PathBuf::new()).unwrap();
            runtime.start();
            runtime.tick().unwrap();

            assert_eq!(
                runtime.framebuffer_bits,
                expected_bits | FRAMEBUFFER_BITS_EXPLICIT
            );
        }
    }

    #[test]
    fn homebrew_legacy_framebuffer_defaults_to_rgb565() {
        let mut package = svc_package("FlushDCache");
        package.target = TargetDevice::GemeiA330(ArmProfile::Homebrew);
        package.rawd.entry = ArmProfile::HOMEBREW_ORIGIN;
        package.rawd.origin = ArmProfile::HOMEBREW_ORIGIN;
        package.imports[0].address = ArmProfile::HOMEBREW_ORIGIN;
        let mut runtime = Runtime::from_package(package, PathBuf::new()).unwrap();
        runtime
            .memory
            .write16(LEGACY_FRAMEBUFFER_ADDRESS, 0x07e0)
            .unwrap();
        runtime.cpu.r[0] = LEGACY_FRAMEBUFFER_ADDRESS;
        runtime.start();
        runtime.tick().unwrap();

        assert_eq!(runtime.framebuffer_bits, 32);
        assert_eq!(&runtime.video.framebuffer()[..2], &[0xe0, 0x07]);
    }

    #[test]
    fn thirty_two_bit_guest_frames_are_converted_to_rgb565() {
        let mut runtime =
            Runtime::from_package(svc_package("FlushDCache"), PathBuf::new()).unwrap();
        runtime.framebuffer_bits = 32 | FRAMEBUFFER_BITS_EXPLICIT;
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
    fn homebrew_frames_default_to_xrgb8888() {
        let mut package = svc_package("FlushDCache");
        package.target = TargetDevice::GemeiA330(ArmProfile::Homebrew);
        package.rawd.entry = ArmProfile::HOMEBREW_ORIGIN;
        package.rawd.origin = ArmProfile::HOMEBREW_ORIGIN;
        package.imports[0].address = ArmProfile::HOMEBREW_ORIGIN;
        let mut runtime = Runtime::from_package(package, PathBuf::new()).unwrap();
        runtime
            .memory
            .write32(APP_PATH_ADDRESS, 0x00ff_0000)
            .unwrap();
        runtime.cpu.r[0] = APP_PATH_ADDRESS;
        runtime.start();
        runtime.tick().unwrap();

        assert_eq!(runtime.framebuffer_bits, 32);
        assert_eq!(&runtime.video.framebuffer()[..2], &[0x00, 0xf8]);
    }

    #[test]
    fn frame_submission_uses_the_configured_framebuffer() {
        let mut runtime =
            Runtime::from_package(svc_package("lcd_set_frame"), PathBuf::new()).unwrap();
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
            Runtime::from_package(svc_package("waveout_open"), PathBuf::new()).unwrap();
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
        let mut runtime = Runtime::from_package(package, PathBuf::new()).unwrap();
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
