use crate::app_loader::{AppImage, ResourceKind};
use crate::audio::{Audio, AudioConfig};
use crate::cheats::{CheatManager, CheatParseError, CheatRule};
use crate::cpu::Cpu;
use crate::error::{Result, SimulatorError};
use crate::input::Input;
#[cfg(feature = "jit")]
use crate::jit::JitEngine;
use crate::memory::Memory;
use crate::video::Video;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Component, Path, PathBuf};

mod sdk_hle;

const CPU_CLOCK_HZ: u64 = 336_000_000;
const FRAMES_PER_SECOND: u64 = 60;
const OS_TICKS_PER_SECOND: u64 = 100;
const CYCLES_PER_FRAME: u64 = CPU_CLOCK_HZ / FRAMES_PER_SECOND;
// Model the pipeline and memory stalls with a conservative average CPI.
const CPU_CYCLES_PER_INSTRUCTION: u64 = 2;
const STANDARD_APP_LOAD_BASE: u32 = 0x80A0_0000;
const MAX_AUDIO_WRITE_BYTES: u32 = 4 * 1024 * 1024;
const TASK_QUANTUM_CYCLES: u64 = 4_096;
const TASK_RETURN_ADDRESS: u32 = u32::MAX;
const MAX_GUEST_TASKS: usize = 16;
const HOOK_FILTER_WORDS: usize = 1_024;
const MAX_INSTRUCTION_BLOCK_LEN: usize = 64;
const INSTRUCTION_BLOCK_CACHE_SLOTS: usize = 4_096;
const FILE_SEARCH_NAME_OFFSET: u32 = 0x12;
const FILE_SEARCH_NAME_CAPACITY: usize = 256;

/// Behavior when the guest calls an SDK function without an HLE implementation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UnknownHlePolicy {
    /// Record the call and return zero to preserve compatibility.
    #[default]
    Report,
    /// Record the call and stop unless the function name is allowlisted.
    Stop,
}

/// Aggregated diagnostics for one unknown SDK HLE function.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct UnknownHleCall {
    pub name: String,
    pub count: u64,
    pub import_address: u32,
    pub first_pc: u32,
    pub first_arguments: [u32; 4],
}

fn hook_filter_location(address: u32) -> (usize, u64) {
    let bit_index = (address as usize >> 2) & (HOOK_FILTER_WORDS * u64::BITS as usize - 1);
    (
        bit_index / u64::BITS as usize,
        1 << (bit_index % u64::BITS as usize),
    )
}

struct CachedInstructionBlock {
    start: u32,
    len: u8,
    instructions: [u32; MAX_INSTRUCTION_BLOCK_LEN],
}

fn instruction_block_cache_index(address: u32) -> usize {
    (address as usize >> 2) & (INSTRUCTION_BLOCK_CACHE_SLOTS - 1)
}

fn empty_instruction_block_cache() -> Box<[CachedInstructionBlock]> {
    std::iter::repeat_with(|| CachedInstructionBlock {
        start: 0,
        len: 0,
        instructions: [0; MAX_INSTRUCTION_BLOCK_LEN],
    })
    .take(INSTRUCTION_BLOCK_CACHE_SLOTS)
    .collect::<Vec<_>>()
    .into_boxed_slice()
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct OpenFile {
    data: Vec<u8>,
    position: usize,
    data_ptr: u32,
    save_path: Option<PathBuf>,
    writable: bool,
    dirty: bool,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct FileSearch {
    entries: Vec<String>,
    next_index: usize,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct GuiState {
    key_messages_enabled: bool,
    windows: BTreeMap<u32, u32>,
    focused_window: Option<u32>,
    next_window_handle: u32,
    reported_key: u32,
    message_buffer: Option<u32>,
    key_info_buffer: Option<u32>,
}

impl Default for GuiState {
    fn default() -> Self {
        Self {
            key_messages_enabled: false,
            windows: BTreeMap::new(),
            focused_window: None,
            next_window_handle: 1,
            reported_key: 0,
            message_buffer: None,
            key_info_buffer: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum TaskWait {
    AudioWrite,
    Semaphore(u32),
    UntilCycle(u64),
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct GuestTask {
    cpu: Cpu,
    priority: u32,
    wait: Option<TaskWait>,
}

#[derive(serde::Serialize)]
struct EmulatorStateRef<'a> {
    cpu: &'a Cpu,
    memory: &'a Memory,
    video: &'a Video,
    input: &'a Input,
    audio: &'a Audio,
    frame_count: u64,
    cycle_count: u64,
    tasks: &'a [GuestTask],
    scheduler_cursor: usize,
    main_wait: Option<TaskWait>,
    active_task: Option<usize>,
    semaphores: &'a HashMap<u32, u32>,
    next_semaphore_handle: u32,
    open_files: HashMap<u32, OpenFile>,
    next_file_handle: u32,
    file_searches: &'a HashMap<u32, FileSearch>,
    gui: &'a GuiState,
    app_main_args_initialized: bool,
    locale_ansi_buffer: Option<u32>,
    framebuffer_submitted: bool,
}

#[derive(serde::Deserialize)]
struct EmulatorState {
    cpu: Cpu,
    memory: Memory,
    video: Video,
    input: Input,
    audio: Audio,
    frame_count: u64,
    cycle_count: u64,
    tasks: Vec<GuestTask>,
    scheduler_cursor: usize,
    main_wait: Option<TaskWait>,
    active_task: Option<usize>,
    semaphores: HashMap<u32, u32>,
    next_semaphore_handle: u32,
    open_files: HashMap<u32, OpenFile>,
    next_file_handle: u32,
    file_searches: HashMap<u32, FileSearch>,
    gui: GuiState,
    app_main_args_initialized: bool,
    locale_ansi_buffer: Option<u32>,
    framebuffer_submitted: bool,
}

fn prepare_resource_file_data(name: &str, kind: ResourceKind, data: Vec<u8>) -> Vec<u8> {
    let is_bin = name
        .rsplit_once('.')
        .is_some_and(|(_, extension)| extension.eq_ignore_ascii_case("bin"));
    if kind != ResourceKind::Packed || !is_bin || data.len() < 12 {
        return data;
    }

    let record_count = u16::from_le_bytes([data[0], data[1]]);
    let declared_size = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);
    let Ok(data_size) = u32::try_from(data.len()) else {
        return data;
    };
    if record_count == 0 || declared_size <= data_size {
        return data;
    }

    let payload = &data[4..];
    let Ok(payload_size) = u32::try_from(payload.len()) else {
        return data;
    };
    let Some(view_size) = data.len().checked_add(16) else {
        return data;
    };
    let mut view = vec![0; view_size];
    view[0..4].copy_from_slice(&1_u32.to_le_bytes());
    view[8..12].copy_from_slice(&payload_size.to_le_bytes());
    view[12..12 + payload.len()].copy_from_slice(payload);
    view[16..20].fill(0);
    view
}

/// Main emulator struct that ties all components together
pub struct Emulator {
    /// CPU core
    pub cpu: Cpu,
    /// Memory system
    pub memory: Memory,
    /// Video subsystem
    pub video: Video,
    /// Input subsystem
    pub input: Input,
    /// PCM audio subsystem
    pub audio: Audio,
    /// Frontend-managed memory and register freeze rules
    cheats: CheatManager,
    /// Frame count
    frame_count: u64,
    /// Emulated CPU cycles elapsed
    cycle_count: u64,
    /// Cooperatively scheduled guest tasks
    tasks: Vec<GuestTask>,
    /// Scheduler position preserved across frontend frames
    scheduler_cursor: usize,
    /// Wait state for the main guest task
    main_wait: Option<TaskWait>,
    /// Task whose CPU is currently swapped into `cpu`
    active_task: Option<usize>,
    /// uC/OS-II semaphore counts by guest handle
    semaphores: HashMap<u32, u32>,
    /// Next guest semaphore handle
    next_semaphore_handle: u32,
    /// Parsed app image (for resource access)
    app: Option<AppImage>,
    /// Import address to function name mapping (for diagnostics)
    #[allow(dead_code)]
    import_addrs: HashMap<u32, String>,
    /// Hooked addresses (for SDK function interception)
    hooked_addrs: HashMap<u32, String>,
    /// Behavior for SDK imports without an HLE implementation
    unknown_hle_policy: UnknownHlePolicy,
    /// Function names allowed to retain compatibility-stub behavior in strict mode
    unknown_hle_allowlist: BTreeSet<String>,
    /// Unknown SDK calls aggregated by function name for diagnostics
    unknown_hle_calls: BTreeMap<String, UnknownHleCall>,
    /// Fast rejection filter for non-hook instruction addresses
    hook_filter: Box<[u64]>,
    /// Direct-mapped cache of sequential guest instruction blocks
    instruction_blocks: Box<[CachedInstructionBlock]>,
    /// Native code cache for hot MIPS instruction blocks
    #[cfg(feature = "jit")]
    jit: JitEngine,
    /// Open guest resource files
    open_files: HashMap<u32, OpenFile>,
    /// Host directory used for persistent guest-created files
    save_directory: Option<PathBuf>,
    /// Next guest file handle
    next_file_handle: u32,
    /// Active file searches keyed by the guest find-data address
    file_searches: HashMap<u32, FileSearch>,
    /// Minimal window-manager state used to deliver guest key messages
    gui: GuiState,
    /// AppMain export address
    app_main_entry: Option<u32>,
    /// AppMain startup check hook address
    app_main_init_check_address: Option<u32>,
    /// Whether AppMain startup arguments were installed
    app_main_args_initialized: bool,
    /// Original app path for AppMain
    app_path: String,
    /// Reusable guest buffer for ANSI string conversions
    locale_ansi_buffer: Option<u32>,
    /// Whether the guest submitted a framebuffer this tick
    framebuffer_submitted: bool,
}

impl Emulator {
    /// Create a new emulator from an .app file path
    pub fn from_path<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let app = AppImage::from_path(path)?;
        Self::from_app_with_path(app, path.to_string_lossy().into_owned())
    }

    /// Create a new emulator from a parsed AppImage
    pub fn from_app(app: AppImage) -> Result<Self> {
        Self::from_app_with_path(app, String::new())
    }

    fn from_app_with_path(app: AppImage, app_path: String) -> Result<Self> {
        let mut memory = Memory::new();

        // Load executable into memory at the load base address (KSEG0)
        let load_base = app.load_base();
        let executable = app.executable().to_vec();
        memory.load_data(load_base, &executable)?;

        // Also map at physical address (for games that use physical addressing)
        let physical_addr = load_base & 0x1FFF_FFFF;
        if physical_addr != load_base {
            memory.load_data(physical_addr, &executable)?;
        }

        // Map framebuffer at a fixed guest-visible address
        // The game writes directly to this address
        let fb_addr = crate::video::VM_LCD_FB_ADDRESS;
        let fb_size = crate::video::FRAMEBUFFER_SIZE;
        // Reserve space in memory for framebuffer (zero it out)
        for i in 0..fb_size {
            let _ = memory.write_u8(fb_addr + i as u32, 0);
        }

        let mut cpu = Cpu::new(app.entry_point());
        let app_main_entry = app
            .exports
            .iter()
            .find(|export| export.name == "AppMain")
            .map(|export| export.address);

        if let Some(app_main_entry) = app_main_entry {
            cpu.regs.write(31, app_main_entry);
            cpu.regs.write(25, app.entry_point());
        }
        // Older app layouts require the loader to request LCD initialization.
        cpu.regs
            .write(5, u32::from(load_base != STANDARD_APP_LOAD_BASE));

        // Initialize stack pointer to a reasonable value in RAM
        // Stack grows downward from top of RAM (32MB)
        cpu.regs.write(29, 0x01FF_FFF0); // $sp = top of RAM - 16

        // Use a fixed guest-visible framebuffer address
        // The game writes directly to this address
        let video = Video::new();

        let input = Input::new();
        let audio = Audio::new();
        // Build import address map for SDK hooking
        // The game uses physical addressing, not KSEG0
        // So we need to hook physical addresses
        let mut import_addrs = HashMap::new();
        let mut hooked_addrs = HashMap::new();
        let mut hook_filter = vec![0; HOOK_FILTER_WORDS].into_boxed_slice();
        for import in &app.imports {
            // Physical address (what the game actually uses)
            let phys = import.address & 0x1FFF_FFFF;
            import_addrs.insert(phys, import.name.clone());
            hooked_addrs.insert(phys, import.name.clone());
            let (word, mask) = hook_filter_location(phys);
            hook_filter[word] |= mask;
            // Also hook KSEG0 address (for completeness)
            if phys != import.address {
                import_addrs.insert(import.address, import.name.clone());
                hooked_addrs.insert(import.address, import.name.clone());
                let (word, mask) = hook_filter_location(import.address);
                hook_filter[word] |= mask;
            }
        }

        log::debug!(
            "Emulator initialized: entry={:#010x}, base={:#010x}, physical={:#010x}, framebuffer={:#010x}, imports={}, hooked={}",
            app.entry_point(),
            load_base,
            physical_addr,
            crate::video::VM_LCD_FB_ADDRESS,
            import_addrs.len(),
            hooked_addrs.len()
        );

        for (addr, name) in hooked_addrs.iter().take(5) {
            log::trace!("Hooked SDK import: {:#010x} = {}", addr, name);
        }

        let save_directory = Path::new(&app_path).parent().map(Path::to_path_buf);
        Ok(Self {
            cpu,
            memory,
            video,
            input,
            audio,
            cheats: CheatManager::default(),
            frame_count: 0,
            cycle_count: 0,
            tasks: Vec::new(),
            scheduler_cursor: 0,
            main_wait: None,
            active_task: None,
            semaphores: HashMap::new(),
            next_semaphore_handle: 1,
            app: Some(app),
            import_addrs,
            hooked_addrs,
            unknown_hle_policy: UnknownHlePolicy::default(),
            unknown_hle_allowlist: BTreeSet::new(),
            unknown_hle_calls: BTreeMap::new(),
            hook_filter,
            instruction_blocks: empty_instruction_block_cache(),
            #[cfg(feature = "jit")]
            jit: JitEngine::new(),
            open_files: HashMap::new(),
            save_directory,
            next_file_handle: 1,
            file_searches: HashMap::new(),
            gui: GuiState::default(),
            app_main_entry,
            app_main_init_check_address: app_main_entry.map(|addr| addr.wrapping_add(0x34)),
            app_main_args_initialized: false,
            app_path,
            locale_ansi_buffer: None,
            framebuffer_submitted: false,
        })
    }

    fn install_app_main_args(&mut self) -> Result<()> {
        if self.app_main_args_initialized {
            return Ok(());
        }
        self.app_main_args_initialized = true;

        let path = if self.app_path.is_empty() {
            "game.app"
        } else {
            self.app_path
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(&self.app_path)
        };
        let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        let byte_len = (wide.len() * 2) as u32;
        let ptr = self.memory.malloc(byte_len);
        if ptr == 0 {
            return Ok(());
        }
        for (i, word) in wide.iter().enumerate() {
            self.memory
                .write_u16(ptr.wrapping_add((i * 2) as u32), *word)?;
        }
        self.cpu.regs.write(4, ptr);
        if let Some(app_main_entry) = self.app_main_entry {
            self.cpu.regs.write(25, app_main_entry);
        }
        Ok(())
    }
    /// Start the emulator
    pub fn start(&mut self) {
        self.cpu.start();
        log::info!("Emulator started");
    }

    /// Configure how unknown SDK HLE calls affect execution.
    pub fn set_unknown_hle_policy(&mut self, policy: UnknownHlePolicy) {
        self.unknown_hle_policy = policy;
    }

    /// Replace the exact function-name allowlist used by strict HLE mode.
    pub fn set_unknown_hle_allowlist<I, S>(&mut self, names: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.unknown_hle_allowlist = names.into_iter().map(Into::into).collect();
    }

    /// Return unknown HLE diagnostics in stable function-name order.
    pub fn unknown_hle_calls(&self) -> impl ExactSizeIterator<Item = &UnknownHleCall> {
        self.unknown_hle_calls.values()
    }

    /// Clear all unknown HLE observations collected during this run.
    pub fn clear_unknown_hle_calls(&mut self) {
        self.unknown_hle_calls.clear();
    }

    fn record_unknown_hle(
        &mut self,
        name: &str,
        import_address: u32,
        return_address: u32,
    ) -> Result<()> {
        let first_pc = return_address.wrapping_sub(8);
        let first_arguments = [
            self.cpu.regs.read(4),
            self.cpu.regs.read(5),
            self.cpu.regs.read(6),
            self.cpu.regs.read(7),
        ];
        let is_first = if let Some(call) = self.unknown_hle_calls.get_mut(name) {
            call.count = call.count.saturating_add(1);
            false
        } else {
            self.unknown_hle_calls.insert(
                name.to_string(),
                UnknownHleCall {
                    name: name.to_string(),
                    count: 1,
                    import_address,
                    first_pc,
                    first_arguments,
                },
            );
            true
        };

        if is_first {
            log::warn!(
                "Unknown SDK HLE {name} first called at {first_pc:#010x} (import {import_address:#010x}); calls are aggregated"
            );
        }

        if self.unknown_hle_policy == UnknownHlePolicy::Stop
            && !self.unknown_hle_allowlist.contains(name)
        {
            return Err(SimulatorError::UnknownHle {
                name: name.to_string(),
                pc: first_pc,
                import_address,
                arguments: first_arguments,
            });
        }
        Ok(())
    }

    /// Stop the emulator
    pub fn stop(&mut self) {
        self.flush_save_files();
        self.cpu.stop();
        log::info!("Emulator stopped");
    }

    /// Rebuild all mutable runtime state from the loaded app image.
    pub fn reset(&mut self) -> Result<()> {
        self.flush_save_files();
        let app = self
            .app
            .clone()
            .ok_or_else(|| "cannot reset an emulator without a loaded app".to_string())?;
        let was_running = self.is_running();
        let mut replacement = Self::from_app_with_path(app, self.app_path.clone())?;
        replacement.save_directory = self.save_directory.clone();
        replacement.cheats = self.cheats.clone();
        replacement.unknown_hle_policy = self.unknown_hle_policy;
        replacement.unknown_hle_allowlist = self.unknown_hle_allowlist.clone();
        self.memory.copy_state_from(&replacement.memory);
        std::mem::swap(&mut replacement.memory, &mut self.memory);
        if was_running {
            replacement.start();
        }
        *self = replacement;
        log::info!("Emulator reset");
        Ok(())
    }

    /// Return the fixed buffer capacity required for a serialized state.
    pub fn serialized_state_size(&self) -> usize {
        crate::save_state::SERIALIZED_SIZE
    }

    /// Serialize the complete mutable runtime state into a fixed-size buffer.
    pub fn serialize_state(&self, output: &mut [u8]) -> anyhow::Result<()> {
        let app = self
            .app
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("cannot save state without loaded content"))?;
        let mut open_files = self.open_files.clone();
        for file in open_files.values_mut() {
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
        let state = EmulatorStateRef {
            cpu: &self.cpu,
            memory: &self.memory,
            video: &self.video,
            input: &self.input,
            audio: &self.audio,
            frame_count: self.frame_count,
            cycle_count: self.cycle_count,
            tasks: &self.tasks,
            scheduler_cursor: self.scheduler_cursor,
            main_wait: self.main_wait,
            active_task: self.active_task,
            semaphores: &self.semaphores,
            next_semaphore_handle: self.next_semaphore_handle,
            open_files,
            next_file_handle: self.next_file_handle,
            file_searches: &self.file_searches,
            gui: &self.gui,
            app_main_args_initialized: self.app_main_args_initialized,
            locale_ansi_buffer: self.locale_ansi_buffer,
            framebuffer_submitted: self.framebuffer_submitted,
        };
        crate::save_state::encode(&state, crc32fast::hash(&app.data), output)
    }

    /// Restore a serialized state without changing the emulator on failure.
    pub fn unserialize_state(&mut self, input: &[u8]) -> anyhow::Result<()> {
        let app = self
            .app
            .clone()
            .ok_or_else(|| anyhow::anyhow!("cannot load state without loaded content"))?;
        let mut state: EmulatorState =
            crate::save_state::decode(input, crc32fast::hash(&app.data))?;
        if !state.memory.snapshot_layout_is_valid() || !state.video.snapshot_layout_is_valid() {
            anyhow::bail!("save state has an incompatible memory layout");
        }
        if state.tasks.len() > MAX_GUEST_TASKS
            || state
                .active_task
                .is_some_and(|index| index >= state.tasks.len())
        {
            anyhow::bail!("save state has an invalid task layout");
        }
        for file in state.open_files.values_mut() {
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

        let was_running = state.cpu.is_running();
        #[cfg(feature = "standalone")]
        let host_audio_output_enabled = self.audio.host_output_enabled();
        let mut replacement = Self::from_app_with_path(app, self.app_path.clone())?;
        replacement.save_directory = self.save_directory.clone();
        replacement.cheats = self.cheats.clone();
        replacement.unknown_hle_policy = self.unknown_hle_policy;
        replacement.unknown_hle_allowlist = self.unknown_hle_allowlist.clone();
        replacement.unknown_hle_calls = self.unknown_hle_calls.clone();
        replacement.cpu = state.cpu;
        self.memory.copy_state_from(&state.memory);
        std::mem::swap(&mut replacement.memory, &mut self.memory);
        replacement.video = state.video;
        replacement.input = state.input;
        replacement.audio = state.audio;
        #[cfg(feature = "standalone")]
        replacement
            .audio
            .set_host_output_enabled(host_audio_output_enabled);
        replacement.audio.resume_after_state_load();
        replacement.frame_count = state.frame_count;
        replacement.cycle_count = state.cycle_count;
        replacement.tasks = state.tasks;
        replacement.scheduler_cursor = state.scheduler_cursor;
        replacement.main_wait = state.main_wait;
        replacement.active_task = state.active_task;
        replacement.semaphores = state.semaphores;
        replacement.next_semaphore_handle = state.next_semaphore_handle;
        replacement.open_files = state.open_files;
        replacement.next_file_handle = state.next_file_handle;
        replacement.file_searches = state.file_searches;
        replacement.gui = state.gui;
        replacement.app_main_args_initialized = state.app_main_args_initialized;
        replacement.locale_ansi_buffer = state.locale_ansi_buffer;
        replacement.framebuffer_submitted = state.framebuffer_submitted;
        if was_running {
            replacement.cpu.start();
        }
        *self = replacement;
        Ok(())
    }

    /// Run one frame of emulation
    pub fn tick(&mut self) -> Result<()> {
        #[cfg(feature = "jit")]
        self.jit.begin_frame();
        self.framebuffer_submitted = false;
        self.cheats.apply(&mut self.memory, &mut self.cpu);

        let mut remaining_cycles = CYCLES_PER_FRAME;
        let mut idle_contexts = 0usize;
        while remaining_cycles > 0 {
            let context_count = self.tasks.len() + 1;
            if self.scheduler_cursor >= context_count {
                self.scheduler_cursor = 0;
            }
            let slice_cycles = remaining_cycles.min(TASK_QUANTUM_CYCLES);
            let executed = if self.scheduler_cursor == 0 {
                self.active_task = None;
                self.run_active_cpu_slice(slice_cycles)?
            } else {
                self.run_task_slice(self.scheduler_cursor - 1, slice_cycles)?
            };
            self.scheduler_cursor = (self.scheduler_cursor + 1) % context_count;

            if executed == 0 {
                idle_contexts += 1;
                if idle_contexts >= context_count {
                    self.cycle_count = self.cycle_count.wrapping_add(slice_cycles);
                    remaining_cycles -= slice_cycles;
                    idle_contexts = 0;
                }
            } else {
                remaining_cycles -= executed;
                idle_contexts = 0;
            }

            if self.framebuffer_submitted {
                self.cycle_count = self.cycle_count.wrapping_add(remaining_cycles);
                remaining_cycles = 0;
            }
        }
        self.tasks.retain(|task| task.cpu.is_running());

        // Use a fallback sync for tests or apps that draw without an explicit submit.
        if !self.framebuffer_submitted {
            self.sync_framebuffer();
        }

        self.video.advance_frame();
        self.audio.advance_frame();
        self.frame_count += 1;

        Ok(())
    }

    fn run_task_slice(&mut self, task_index: usize, cycles: u64) -> Result<u64> {
        let task_cpu = std::mem::replace(&mut self.tasks[task_index].cpu, Cpu::new(0));
        let main_cpu = std::mem::replace(&mut self.cpu, task_cpu);
        self.active_task = Some(task_index);
        let result = self.run_active_cpu_slice(cycles);
        let task_cpu = std::mem::replace(&mut self.cpu, main_cpu);
        self.tasks[task_index].cpu = task_cpu;
        self.active_task = None;
        result
    }

    fn run_active_cpu_slice(&mut self, cycles: u64) -> Result<u64> {
        if self.active_context_waiting() || !self.cpu.is_running() {
            return Ok(0);
        }

        let mut executed = 0;
        while executed < cycles {
            if self.cpu.regs.pc == TASK_RETURN_ADDRESS {
                self.cpu.stop();
                break;
            }

            let pc = self.cpu.regs.pc;
            if self.active_task.is_none()
                && Some(pc) == self.app_main_entry
                && !self.app_main_args_initialized
            {
                self.install_app_main_args()?;
            }
            if self.active_task.is_none() && Some(pc) == self.app_main_init_check_address {
                self.cpu.regs.write(2, 1);
            }

            let (hook_word, hook_mask) = hook_filter_location(pc);
            let func_name = (self.hook_filter[hook_word] & hook_mask != 0)
                .then(|| self.hooked_addrs.get(&pc))
                .flatten()
                .cloned();
            if let Some(func_name) = func_name {
                log::trace!("SDK hook: PC={:#010x} = {}", pc, func_name);
                sdk_hle::dispatch(self, pc, &func_name)?;
                self.cycle_count = self.cycle_count.wrapping_add(CPU_CYCLES_PER_INSTRUCTION);
                executed += CPU_CYCLES_PER_INSTRUCTION;
                if self.framebuffer_submitted
                    || self.active_context_waiting()
                    || !self.cpu.is_running()
                {
                    break;
                }
            } else {
                let completed = self.run_cached_instruction_block(pc, cycles - executed)?;
                let completed_cycles = completed * CPU_CYCLES_PER_INSTRUCTION;
                self.cycle_count = self.cycle_count.wrapping_add(completed_cycles);
                executed += completed_cycles;
            }
        }
        Ok(executed)
    }

    fn run_cached_instruction_block(&mut self, start: u32, remaining_cycles: u64) -> Result<u64> {
        self.ensure_instruction_block(start)?;
        let instruction_limit = (remaining_cycles / CPU_CYCLES_PER_INSTRUCTION) as usize;
        let cache_index = instruction_block_cache_index(start);

        #[cfg(feature = "jit")]
        if !self.cpu.branch_delay {
            let block = &self.instruction_blocks[cache_index];
            let ram = self.memory.jit_ram_ptr();
            let framebuffer = self.memory.jit_framebuffer_ptr();
            if let Some(completed) = self.jit.execute(
                start,
                &block.instructions[..block.len as usize],
                instruction_limit,
                &mut self.cpu.regs,
                ram,
                framebuffer,
            ) {
                self.cpu.account_instructions(completed);
                return Ok(completed);
            }
        }

        let block = &self.instruction_blocks[cache_index];
        let mut completed = 0u64;

        for &instruction in block.instructions[..block.len as usize]
            .iter()
            .take(instruction_limit)
        {
            let current_pc = self.cpu.regs.pc;
            let step_result = self
                .cpu
                .step_fetched_unaccounted(instruction, &mut self.memory);
            if step_result.is_err() {
                self.cpu.account_instructions(completed);
                self.cycle_count = self
                    .cycle_count
                    .wrapping_add(completed * CPU_CYCLES_PER_INSTRUCTION);
            }
            if !step_result? {
                break;
            }
            completed += 1;
            if !self.cpu.is_running() || self.cpu.regs.pc != current_pc.wrapping_add(4) {
                break;
            }
        }

        self.cpu.account_instructions(completed);
        Ok(completed)
    }

    fn ensure_instruction_block(&mut self, start: u32) -> Result<()> {
        let cache_index = instruction_block_cache_index(start);
        if self.instruction_blocks[cache_index].len != 0
            && self.instruction_blocks[cache_index].start == start
        {
            return Ok(());
        }

        let mut instructions = [0; MAX_INSTRUCTION_BLOCK_LEN];
        let mut instruction_count = 0usize;
        let mut address = start;
        while instruction_count < MAX_INSTRUCTION_BLOCK_LEN {
            if address != start && self.is_instruction_block_boundary(address) {
                break;
            }
            instructions[instruction_count] = self.memory.fetch_instruction(address)?;
            instruction_count += 1;
            address = address.wrapping_add(4);
        }
        self.instruction_blocks[cache_index] = CachedInstructionBlock {
            start,
            len: instruction_count as u8,
            instructions,
        };
        Ok(())
    }

    fn is_instruction_block_boundary(&self, address: u32) -> bool {
        if address == TASK_RETURN_ADDRESS
            || Some(address) == self.app_main_entry
            || Some(address) == self.app_main_init_check_address
        {
            return true;
        }
        let (hook_word, hook_mask) = hook_filter_location(address);
        self.hook_filter[hook_word] & hook_mask != 0 && self.hooked_addrs.contains_key(&address)
    }

    pub(crate) fn clear_instruction_cache(&mut self) {
        for block in &mut self.instruction_blocks {
            block.len = 0;
        }
        #[cfg(feature = "jit")]
        self.jit.clear();
    }

    /// Enable or disable native translation of hot CPU blocks.
    pub fn set_jit_enabled(&mut self, enabled: bool) {
        #[cfg(feature = "jit")]
        self.jit.set_enabled(enabled);
        #[cfg(not(feature = "jit"))]
        let _ = enabled;
    }

    fn active_context_waiting(&mut self) -> bool {
        let cycle_count = self.cycle_count;
        let wait = if let Some(task_index) = self.active_task {
            self.tasks[task_index].wait
        } else {
            self.main_wait
        };
        match wait {
            Some(TaskWait::AudioWrite) if self.audio.can_write() => {
                self.clear_active_wait();
                false
            }
            Some(TaskWait::UntilCycle(deadline)) if cycle_count >= deadline => {
                self.clear_active_wait();
                false
            }
            Some(_) => true,
            None => false,
        }
    }

    fn create_guest_task(
        &mut self,
        entry: u32,
        data_ptr: u32,
        stack_ptr: u32,
        priority: u32,
    ) -> bool {
        if entry == 0 || self.tasks.len() >= MAX_GUEST_TASKS {
            return false;
        }

        let mut cpu = Cpu::new(entry);
        cpu.regs.write(4, data_ptr);
        cpu.regs.write(25, entry);
        cpu.regs.write(29, stack_ptr);
        cpu.regs.write(31, TASK_RETURN_ADDRESS);
        cpu.start();
        self.tasks.push(GuestTask {
            cpu,
            priority,
            wait: None,
        });
        log::debug!(
            "Created guest task: entry={entry:#010x}, data={data_ptr:#010x}, stack={stack_ptr:#010x}, priority={priority}"
        );
        true
    }

    fn set_active_wait(&mut self, wait: TaskWait) {
        if let Some(task_index) = self.active_task {
            self.tasks[task_index].wait = Some(wait);
        } else {
            self.main_wait = Some(wait);
        }
    }

    fn delay_active_until(&mut self, deadline: u64) {
        if deadline > self.cycle_count {
            self.set_active_wait(TaskWait::UntilCycle(deadline));
        }
    }

    fn create_semaphore(&mut self, count: u32) -> u32 {
        let handle = self.next_semaphore_handle;
        self.next_semaphore_handle = self.next_semaphore_handle.wrapping_add(1).max(1);
        self.semaphores.insert(handle, count);
        handle
    }

    fn pend_semaphore(&mut self, handle: u32) -> bool {
        let Some(count) = self.semaphores.get_mut(&handle) else {
            return false;
        };
        if *count > 0 {
            *count -= 1;
        } else {
            self.set_active_wait(TaskWait::Semaphore(handle));
        }
        true
    }

    fn post_semaphore(&mut self, handle: u32) -> bool {
        if !self.semaphores.contains_key(&handle) {
            return false;
        }

        if self.main_wait == Some(TaskWait::Semaphore(handle)) {
            self.main_wait = None;
            return true;
        }
        if let Some(task_index) = self
            .tasks
            .iter()
            .enumerate()
            .filter(|(_, task)| task.wait == Some(TaskWait::Semaphore(handle)))
            .min_by_key(|(_, task)| task.priority)
            .map(|(index, _)| index)
        {
            self.tasks[task_index].wait = None;
            return true;
        }

        if let Some(count) = self.semaphores.get_mut(&handle) {
            *count = count.saturating_add(1);
        }
        true
    }

    fn read_guest_c_string(&self, ptr: u32) -> String {
        let mut bytes = Vec::new();
        let mut offset = 0u32;
        while let Ok(b) = self.memory.read_u8(ptr.wrapping_add(offset)) {
            if b == 0 {
                break;
            }
            bytes.push(b);
            offset = offset.wrapping_add(1);
            if bytes.len() >= 1024 {
                break;
            }
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }

    fn read_guest_w_string(&self, ptr: u32) -> String {
        let mut words = Vec::new();
        let mut offset = 0u32;
        while let Ok(w) = self.memory.read_u16(ptr.wrapping_add(offset)) {
            if w == 0 {
                break;
            }
            words.push(w);
            offset = offset.wrapping_add(2);
            if words.len() >= 1024 {
                break;
            }
        }
        String::from_utf16_lossy(&words)
    }

    fn guest_printf_arg(&self, index: usize) -> Result<u32> {
        match index {
            0 => Ok(self.cpu.regs.read(6)),
            1 => Ok(self.cpu.regs.read(7)),
            _ => {
                let stack_offset = 8u32.wrapping_add((index as u32).wrapping_mul(4));
                self.memory
                    .read_u32(self.cpu.regs.read(29).wrapping_add(stack_offset))
            }
        }
    }

    fn format_guest_printf(&self, format: &str) -> Result<String> {
        let bytes = format.as_bytes();
        let mut output = String::new();
        let mut cursor = 0;
        let mut arg_index = 0;

        while cursor < bytes.len() {
            if bytes[cursor] != b'%' {
                output.push(bytes[cursor] as char);
                cursor += 1;
                continue;
            }

            cursor += 1;
            if cursor < bytes.len() && bytes[cursor] == b'%' {
                output.push('%');
                cursor += 1;
                continue;
            }

            let mut left_aligned = false;
            let mut show_sign = false;
            let mut space_sign = false;
            let mut alternate = false;
            let mut zero_padded = false;
            while cursor < bytes.len() {
                match bytes[cursor] {
                    b'-' => left_aligned = true,
                    b'+' => show_sign = true,
                    b' ' => space_sign = true,
                    b'#' => alternate = true,
                    b'0' => zero_padded = true,
                    _ => break,
                }
                cursor += 1;
            }

            let width_start = cursor;
            while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
                cursor += 1;
            }
            let width = if cursor > width_start {
                format[width_start..cursor].parse::<usize>().ok()
            } else {
                None
            };

            let precision = if cursor < bytes.len() && bytes[cursor] == b'.' {
                cursor += 1;
                let precision_start = cursor;
                while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
                    cursor += 1;
                }
                Some(
                    format[precision_start..cursor]
                        .parse::<usize>()
                        .unwrap_or(0),
                )
            } else {
                None
            };

            while cursor < bytes.len()
                && matches!(bytes[cursor], b'h' | b'l' | b'j' | b'z' | b't' | b'L')
            {
                cursor += 1;
            }
            if cursor >= bytes.len() {
                output.push('%');
                break;
            }

            let conversion = bytes[cursor];
            cursor += 1;
            let argument = self.guest_printf_arg(arg_index)?;
            arg_index += 1;

            let (mut field, numeric_prefix_len) = match conversion {
                b's' => {
                    let value = self.read_guest_c_string(argument);
                    let value = precision
                        .map(|limit| value.chars().take(limit).collect())
                        .unwrap_or(value);
                    (value, 0)
                }
                b'c' => ((argument as u8 as char).to_string(), 0),
                b'd' | b'i' => {
                    let signed = argument as i32 as i64;
                    let sign = if signed < 0 {
                        "-"
                    } else if show_sign {
                        "+"
                    } else if space_sign {
                        " "
                    } else {
                        ""
                    };
                    let digits = signed.unsigned_abs().to_string();
                    let digits = Self::apply_integer_precision(digits, precision, argument == 0);
                    (format!("{sign}{digits}"), sign.len())
                }
                b'u' => {
                    let digits = argument.to_string();
                    (
                        Self::apply_integer_precision(digits, precision, argument == 0),
                        0,
                    )
                }
                b'o' => {
                    let mut prefix = "";
                    let digits = format!("{argument:o}");
                    if alternate && !digits.starts_with('0') {
                        prefix = "0";
                    }
                    let digits = Self::apply_integer_precision(digits, precision, argument == 0);
                    (format!("{prefix}{digits}"), prefix.len())
                }
                b'x' | b'X' | b'p' => {
                    let uppercase = conversion == b'X';
                    let prefix = if conversion == b'p' || (alternate && argument != 0) {
                        if uppercase {
                            "0X"
                        } else {
                            "0x"
                        }
                    } else {
                        ""
                    };
                    let digits = if uppercase {
                        format!("{argument:X}")
                    } else {
                        format!("{argument:x}")
                    };
                    let digits = Self::apply_integer_precision(digits, precision, argument == 0);
                    (format!("{prefix}{digits}"), prefix.len())
                }
                _ => {
                    output.push('%');
                    output.push(conversion as char);
                    continue;
                }
            };

            if let Some(width) = width {
                if field.len() < width {
                    let padding = width - field.len();
                    if left_aligned {
                        field.push_str(&" ".repeat(padding));
                    } else if zero_padded && precision.is_none() && numeric_prefix_len > 0 {
                        field.insert_str(numeric_prefix_len, &"0".repeat(padding));
                    } else {
                        let padding_char =
                            if zero_padded && precision.is_none() && numeric_prefix_len == 0 {
                                '0'
                            } else {
                                ' '
                            };
                        field.insert_str(0, &padding_char.to_string().repeat(padding));
                    }
                }
            }
            output.push_str(&field);
        }

        Ok(output)
    }

    fn apply_integer_precision(
        mut digits: String,
        precision: Option<usize>,
        is_zero: bool,
    ) -> String {
        let Some(precision) = precision else {
            return digits;
        };
        if precision == 0 && is_zero {
            return String::new();
        }
        if digits.len() < precision {
            digits.insert_str(0, &"0".repeat(precision - digits.len()));
        }
        digits
    }

    fn convert_guest_w_string_to_ansi(&mut self, ptr: u32) -> u32 {
        const MAX_CHARS: u32 = 511;
        const BUFFER_SIZE: u32 = MAX_CHARS + 1;

        if self.locale_ansi_buffer == Some(ptr) {
            return ptr;
        }

        let mut bytes = Vec::new();
        for index in 0..MAX_CHARS {
            let Ok(word) = self.memory.read_u16(ptr.wrapping_add(index * 2)) else {
                return 0;
            };
            if word == 0 {
                break;
            }
            bytes.push(if (0x20..=0x7E).contains(&word) {
                word as u8
            } else {
                b'?'
            });
        }
        bytes.push(0);

        let output = match self.locale_ansi_buffer {
            Some(output) => output,
            None => {
                let output = self.memory.malloc(BUFFER_SIZE);
                if output == 0 {
                    return 0;
                }
                self.locale_ansi_buffer = Some(output);
                output
            }
        };
        if self.memory.load_data(output, &bytes).is_err() {
            return 0;
        }
        output
    }

    fn resource_name_from_args(&self, args: &[u32]) -> Option<String> {
        args.iter().find_map(|&ptr| {
            if ptr < 0x10000 {
                return None;
            }
            let name = self.read_guest_c_string(ptr);
            (!name.is_empty()).then_some(name)
        })
    }
    fn open_resource_file(&mut self, name: &str) -> u32 {
        let Some(app) = self.app.as_ref() else {
            return 0;
        };
        let Some(resource) = app.find_resource(name) else {
            log::trace!("Resource open failed: {name}");
            return 0;
        };

        let kind = resource.kind;
        let resource_data = app.get_resource_data(resource);
        let data = prepare_resource_file_data(name, kind, resource_data);
        let handle = self.next_file_handle;
        self.next_file_handle = self.next_file_handle.wrapping_add(1).max(1);
        let size = data.len();
        self.open_files.insert(
            handle,
            OpenFile {
                data,
                position: 0,
                data_ptr: 0,
                save_path: None,
                writable: false,
                dirty: false,
            },
        );
        log::trace!("Resource opened: {name} -> {handle} ({size} bytes)");
        handle
    }

    fn open_host_file(&mut self, name: &str) -> u32 {
        let path = self.resolve_host_file_path(name);
        let Ok(data) = std::fs::read(&path) else {
            log::trace!("  host file open failed: {}", name);
            return 0;
        };

        let handle = self.next_file_handle;
        self.next_file_handle = self.next_file_handle.wrapping_add(1).max(1);
        let size = data.len();
        self.open_files.insert(
            handle,
            OpenFile {
                data,
                position: 0,
                data_ptr: 0,
                save_path: None,
                writable: false,
                dirty: false,
            },
        );
        log::trace!("  host file open: {} -> {} ({} bytes)", name, handle, size);
        handle
    }

    fn resolve_host_file_path(&self, name: &str) -> PathBuf {
        let normalized_name = name.replace(['/', '\\'], std::path::MAIN_SEPARATOR_STR);
        let path = PathBuf::from(normalized_name);
        if path.is_absolute() {
            return path;
        }

        let Some(separator) = self.app_path.rfind(['/', '\\']) else {
            return path;
        };
        let app_directory =
            self.app_path[..separator].replace(['/', '\\'], std::path::MAIN_SEPARATOR_STR);
        Path::new(&app_directory).join(path)
    }

    fn begin_file_search(&mut self, pattern: &str, attributes: u32, data_ptr: u32) -> Result<u32> {
        self.file_searches.remove(&data_ptr);
        if data_ptr == 0 {
            return Ok(u32::MAX);
        }

        let Some(entries) = self.collect_file_search_entries(pattern, attributes) else {
            return Ok(u32::MAX);
        };
        let Some(first) = entries.first().cloned() else {
            return Ok(u32::MAX);
        };

        self.write_file_search_name(data_ptr, &first)?;
        self.file_searches.insert(
            data_ptr,
            FileSearch {
                entries,
                next_index: 1,
            },
        );
        Ok(0)
    }

    fn next_file_search(&mut self, data_ptr: u32) -> Result<u32> {
        let Some(name) = self.file_searches.get_mut(&data_ptr).and_then(|search| {
            let name = search.entries.get(search.next_index)?.clone();
            search.next_index += 1;
            Some(name)
        }) else {
            return Ok(u32::MAX);
        };

        self.write_file_search_name(data_ptr, &name)?;
        Ok(0)
    }

    fn close_file_search(&mut self, data_ptr: u32) -> u32 {
        self.file_searches.remove(&data_ptr);
        0
    }

    fn collect_file_search_entries(&self, pattern: &str, attributes: u32) -> Option<Vec<String>> {
        let normalized = normalize_guest_search_pattern(pattern)?;
        let (directory, file_pattern) = normalized
            .rsplit_once('/')
            .map_or(("", normalized.as_str()), |(directory, pattern)| {
                (directory, pattern)
            });
        let file_pattern = if file_pattern.is_empty() {
            "*"
        } else {
            file_pattern
        };

        let app_directory = Path::new(&self.app_path).parent().unwrap_or(Path::new("."));
        let search_directory = if directory.is_empty() {
            app_directory.to_path_buf()
        } else {
            app_directory.join(directory.replace('/', std::path::MAIN_SEPARATOR_STR))
        };
        let root = app_directory.canonicalize().ok()?;
        let search_directory = search_directory.canonicalize().ok()?;
        if !search_directory.starts_with(&root) {
            return None;
        }

        let find_directories = attributes & 0x10 != 0;
        let mut entries = std::fs::read_dir(search_directory)
            .ok()?
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| {
                let file_type = entry.file_type().ok()?;
                if (find_directories && !file_type.is_dir())
                    || (!find_directories && !file_type.is_file())
                {
                    return None;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                wildcard_matches(file_pattern, &name).then_some(name)
            })
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| {
            left.to_ascii_lowercase()
                .cmp(&right.to_ascii_lowercase())
                .then_with(|| left.cmp(right))
        });
        Some(entries)
    }

    fn write_file_search_name(&mut self, data_ptr: u32, name: &str) -> Result<()> {
        let name = name.as_bytes();
        let length = name.len().min(FILE_SEARCH_NAME_CAPACITY - 1);
        let destination = data_ptr.wrapping_add(FILE_SEARCH_NAME_OFFSET);
        for (offset, byte) in name.iter().copied().take(length).enumerate() {
            self.memory
                .write_u8(destination.wrapping_add(offset as u32), byte)?;
        }
        self.memory
            .write_u8(destination.wrapping_add(length as u32), 0)?;
        Ok(())
    }

    fn open_memory_file(
        &mut self,
        data: Vec<u8>,
        save_path: PathBuf,
        append: bool,
        writable: bool,
        dirty: bool,
    ) -> u32 {
        let handle = self.next_file_handle;
        self.next_file_handle = self.next_file_handle.wrapping_add(1).max(1);
        let position = if append { data.len() } else { 0 };
        self.open_files.insert(
            handle,
            OpenFile {
                data,
                position,
                data_ptr: 0,
                save_path: Some(save_path),
                writable,
                dirty,
            },
        );
        handle
    }

    fn save_file_path(&self, name: &str) -> Option<PathBuf> {
        let root = self.save_directory.as_ref()?;
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

    fn open_save_file(&mut self, name: &str, operation: u8, writable: bool) -> u32 {
        let Some(path) = self.save_file_path(name) else {
            log::warn!("Rejected guest save path: {name}");
            return 0;
        };
        let data = match operation {
            b'w' => Vec::new(),
            b'a' | b'r' => match std::fs::read(&path) {
                Ok(data) => data,
                Err(error) if operation == b'a' && error.kind() == std::io::ErrorKind::NotFound => {
                    Vec::new()
                }
                Err(_) => return 0,
            },
            _ => return 0,
        };
        self.open_memory_file(
            data,
            path,
            operation == b'a',
            writable,
            writable && operation != b'r',
        )
    }

    fn open_guest_file(&mut self, name: &str, mode: &str) -> u32 {
        let operation = mode.as_bytes().first().copied().unwrap_or(b'r');
        let writable = operation == b'w' || operation == b'a' || mode.contains('+');
        if writable {
            let handle = self.open_save_file(name, operation, true);
            if handle != 0 {
                return handle;
            }
            if operation != b'r' {
                return 0;
            }
        } else {
            let handle = self.open_save_file(name, b'r', false);
            if handle != 0 {
                return handle;
            }
        }

        match self.open_resource_file(name) {
            0 => self.open_host_file(name),
            handle => handle,
        }
    }

    fn flush_save_file(&mut self, handle: u32) -> std::io::Result<()> {
        let Some(file) = self.open_files.get_mut(&handle) else {
            return Ok(());
        };
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

    /// Flush all modified guest save files to the configured save directory.
    pub fn flush_save_files(&mut self) {
        let handles: Vec<u32> = self.open_files.keys().copied().collect();
        for handle in handles {
            if let Err(error) = self.flush_save_file(handle) {
                log::error!("Failed to flush guest save file {handle}: {error}");
            }
        }
    }

    /// Set the directory used for persistent guest-created files.
    pub fn set_save_directory<P: Into<PathBuf>>(&mut self, directory: P) {
        self.flush_save_files();
        self.save_directory = Some(directory.into());
    }

    fn read_file(&mut self, dest: u32, size: u32, count: u32, handle: u32) -> Result<u32> {
        let Some(file) = self.open_files.get_mut(&handle) else {
            return Ok(0);
        };
        let requested = (size as usize).saturating_mul(count as usize);
        if requested == 0 {
            return Ok(0);
        }

        let remaining = file.data.len().saturating_sub(file.position);
        let bytes_to_copy = requested.min(remaining);
        for i in 0..bytes_to_copy {
            self.memory
                .write_u8(dest.wrapping_add(i as u32), file.data[file.position + i])?;
        }
        file.position += bytes_to_copy;

        if size == 0 {
            Ok(0)
        } else {
            Ok((bytes_to_copy / size as usize) as u32)
        }
    }

    fn write_file(&mut self, src: u32, size: u32, count: u32, handle: u32) -> Result<u32> {
        let requested = (size as usize).saturating_mul(count as usize);
        if requested == 0 {
            return Ok(0);
        }

        let mut data = Vec::with_capacity(requested);
        for offset in 0..requested {
            data.push(self.memory.read_u8(src.wrapping_add(offset as u32))?);
        }

        let Some(file) = self.open_files.get_mut(&handle) else {
            return Ok(0);
        };
        if !file.writable {
            return Ok(0);
        }
        let end = file.position.saturating_add(requested);
        if file.data.len() < end {
            file.data.resize(end, 0);
        }
        file.data[file.position..end].copy_from_slice(&data);
        file.position = end;
        file.dirty = true;

        Ok(count)
    }

    fn read_resource_data(
        &mut self,
        handle: u32,
        buffer: u32,
        buffer_len: u32,
        read_len: u32,
    ) -> Result<u32> {
        let Some(file) = self.open_files.get_mut(&handle) else {
            return Ok(0);
        };

        if buffer == 0 {
            if file.data_ptr == 0 {
                let ptr = self.memory.malloc(file.data.len() as u32);
                if ptr == 0 {
                    return Ok(0);
                }
                for (i, &byte) in file.data.iter().enumerate() {
                    self.memory.write_u8(ptr.wrapping_add(i as u32), byte)?;
                }
                file.data_ptr = ptr;
            }
            return Ok(file.data_ptr);
        }

        let remaining = file.data.len().saturating_sub(file.position);
        let mut copy_size = if read_len != 0 && buffer_len > 1 {
            (read_len as usize).saturating_mul(buffer_len as usize)
        } else if read_len != 0 {
            read_len as usize
        } else {
            buffer_len as usize
        };
        if copy_size == 0 || copy_size > remaining {
            copy_size = remaining;
        }

        for i in 0..copy_size {
            self.memory
                .write_u8(buffer.wrapping_add(i as u32), file.data[file.position + i])?;
        }
        file.position += copy_size;

        if read_len != 0 {
            Ok((copy_size / read_len as usize) as u32)
        } else {
            Ok(copy_size as u32)
        }
    }
    fn seek_file(&mut self, handle: u32, offset: i32, origin: u32) -> u32 {
        let Some(file) = self.open_files.get_mut(&handle) else {
            return u32::MAX;
        };

        let base = match origin {
            0 => 0i64,
            1 => file.position as i64,
            2 => file.data.len() as i64,
            _ => return u32::MAX,
        };
        let next = base + offset as i64;
        if next < 0 {
            return u32::MAX;
        }
        file.position = (next as usize).min(file.data.len());
        0
    }

    /// Sync framebuffer from guest memory to video subsystem
    /// The game writes directly to the fixed framebuffer address
    fn sync_framebuffer(&mut self) {
        let fb_data = &self.memory.framebuffer()[..crate::video::FRAMEBUFFER_SIZE];

        self.framebuffer_submitted = true;
        let dst = self.video.framebuffer_mut();
        dst.copy_from_slice(fb_data);
        if log::log_enabled!(log::Level::Trace) {
            let non_zero_count = fb_data.iter().filter(|&&byte| byte != 0).count();
            log::trace!(
                "  sync_framebuffer: {}/{} non-zero bytes",
                non_zero_count,
                fb_data.len()
            );
        }
    }

    fn clear_active_wait(&mut self) {
        if let Some(task_index) = self.active_task {
            self.tasks[task_index].wait = None;
        } else {
            self.main_wait = None;
        }
    }

    /// Set the button state
    pub fn set_buttons(&mut self, buttons: u32) {
        self.input.set_buttons(buttons);
    }

    /// Install or update a frontend cheat slot.
    pub fn set_cheat(
        &mut self,
        index: u32,
        enabled: bool,
        code: &str,
    ) -> std::result::Result<(), CheatParseError> {
        self.cheats.set_slot(index, enabled, code, &self.memory)?;
        self.clear_instruction_cache();
        Ok(())
    }

    /// Install an already parsed cheat rule.
    pub fn set_parsed_cheat(
        &mut self,
        index: u32,
        enabled: bool,
        rule: CheatRule,
    ) -> std::result::Result<(), CheatParseError> {
        self.cheats
            .set_parsed_rule(index, enabled, rule, &self.memory)?;
        self.clear_instruction_cache();
        Ok(())
    }

    /// Remove every configured cheat slot.
    pub fn clear_cheats(&mut self) {
        self.cheats.clear();
        self.clear_instruction_cache();
    }

    /// Get one video frame of interleaved stereo audio.
    pub fn take_audio_samples(&mut self) -> Vec<i16> {
        self.audio.take_frame_samples()
    }

    /// Get the fixed frontend audio sample rate.
    pub fn audio_sample_rate(&self) -> u32 {
        crate::audio::OUTPUT_SAMPLE_RATE
    }

    /// Get the current frame count
    pub fn frame_count(&self) -> u64 {
        self.frame_count
    }

    /// Check if the emulator is running
    pub fn is_running(&self) -> bool {
        self.cpu.is_running()
    }

    /// Get the app image (for resource access)
    pub fn app(&self) -> Option<&AppImage> {
        self.app.as_ref()
    }
}

fn safe_relative_path(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}

fn normalize_guest_search_pattern(pattern: &str) -> Option<String> {
    let mut normalized = pattern.replace('\\', "/");
    if normalized.as_bytes().get(1) == Some(&b':')
        && normalized
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphabetic)
    {
        normalized.drain(..2);
    }
    let normalized = normalized.trim_start_matches('/');
    let path = Path::new(normalized);
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
    {
        return None;
    }
    Some(normalized.to_string())
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

impl Default for Emulator {
    fn default() -> Self {
        Self {
            cpu: Cpu::new(0x8000_0000),
            memory: Memory::new(),
            video: Video::new(),
            input: Input::new(),
            audio: Audio::new(),
            cheats: CheatManager::default(),
            frame_count: 0,
            cycle_count: 0,
            tasks: Vec::new(),
            scheduler_cursor: 0,
            main_wait: None,
            active_task: None,
            semaphores: HashMap::new(),
            next_semaphore_handle: 1,
            app: None,
            import_addrs: HashMap::new(),
            hooked_addrs: HashMap::new(),
            unknown_hle_policy: UnknownHlePolicy::default(),
            unknown_hle_allowlist: BTreeSet::new(),
            unknown_hle_calls: BTreeMap::new(),
            hook_filter: vec![0; HOOK_FILTER_WORDS].into_boxed_slice(),
            instruction_blocks: empty_instruction_block_cache(),
            #[cfg(feature = "jit")]
            jit: JitEngine::new(),
            open_files: HashMap::new(),
            save_directory: None,
            next_file_handle: 1,
            file_searches: HashMap::new(),
            gui: GuiState::default(),
            app_main_entry: None,
            app_main_init_check_address: None,
            app_main_args_initialized: false,
            app_path: String::new(),
            locale_ansi_buffer: None,
            framebuffer_submitted: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_app() -> AppImage {
        let mut data = vec![0u8; 132];
        data[0..4].copy_from_slice(b"CCDL");
        data[0x20..0x24].copy_from_slice(b"IMPT");
        data[0x40..0x44].copy_from_slice(b"EXPT");
        data[0x60..0x64].copy_from_slice(b"RAWD");
        data[0x68..0x6c].copy_from_slice(&128u32.to_le_bytes());
        data[0x6c..0x70].copy_from_slice(&4u32.to_le_bytes());
        data[0x74..0x78].copy_from_slice(&0x8000_0000u32.to_le_bytes());
        data[0x78..0x7c].copy_from_slice(&0x8000_0000u32.to_le_bytes());
        data[0x7c..0x80].copy_from_slice(&4u32.to_le_bytes());
        AppImage::parse(&data).unwrap()
    }

    fn try_invoke_sdk_import(emu: &mut Emulator, address: u32, function_name: &str) -> Result<u64> {
        emu.hooked_addrs.insert(address, function_name.to_string());
        let (word, mask) = hook_filter_location(address);
        emu.hook_filter[word] |= mask;
        emu.cpu.regs.pc = address;
        emu.cpu.start();

        emu.run_active_cpu_slice(CPU_CYCLES_PER_INSTRUCTION)
    }

    fn invoke_sdk_import(emu: &mut Emulator, address: u32, function_name: &str) {
        assert_eq!(
            try_invoke_sdk_import(emu, address, function_name).unwrap(),
            CPU_CYCLES_PER_INSTRUCTION
        );
    }

    #[test]
    fn test_emulator_creation() {
        let emu = Emulator::default();
        assert_eq!(emu.frame_count(), 0);
        assert!(!emu.is_running());
    }

    #[test]
    fn unknown_hle_calls_are_aggregated_by_name() {
        let mut emu = Emulator::default();
        emu.cpu.regs.write(31, 0x8000_1008);
        for (register, value) in (4..=7).zip([1, 2, 3, 4]) {
            emu.cpu.regs.write(register, value);
        }

        invoke_sdk_import(&mut emu, 0x1000, "missing_sdk_call");
        emu.cpu.regs.write(31, 0x8000_2008);
        emu.cpu.regs.write(4, 99);
        invoke_sdk_import(&mut emu, 0x1004, "missing_sdk_call");

        let calls: Vec<_> = emu.unknown_hle_calls().cloned().collect();
        assert_eq!(
            calls,
            vec![UnknownHleCall {
                name: "missing_sdk_call".to_string(),
                count: 2,
                import_address: 0x1000,
                first_pc: 0x8000_1000,
                first_arguments: [1, 2, 3, 4],
            }]
        );
        assert_eq!(emu.cpu.regs.read(2), 0);
        assert_eq!(emu.cpu.regs.pc, 0x8000_2008);
    }

    #[test]
    fn strict_unknown_hle_policy_stops_and_keeps_diagnostics() {
        let mut emu = Emulator::default();
        emu.set_unknown_hle_policy(UnknownHlePolicy::Stop);
        emu.cpu.regs.write(31, 0x8000_3008);
        emu.cpu.regs.write(4, 0x1234);

        let error = try_invoke_sdk_import(&mut emu, 0x2000, "strict_missing").unwrap_err();
        assert!(matches!(
            error,
            SimulatorError::UnknownHle {
                ref name,
                pc: 0x8000_3000,
                import_address: 0x2000,
                arguments: [0x1234, 0, 0, 0],
            } if name == "strict_missing"
        ));
        assert_eq!(emu.unknown_hle_calls().len(), 1);
    }

    #[test]
    fn strict_unknown_hle_allowlist_is_exact_and_survives_reset() {
        let mut emu = Emulator::from_app(minimal_app()).unwrap();
        emu.set_unknown_hle_policy(UnknownHlePolicy::Stop);
        emu.set_unknown_hle_allowlist(["allowed_missing"]);
        emu.cpu.regs.write(31, 0x8000_4008);

        invoke_sdk_import(&mut emu, 0x3000, "allowed_missing");
        assert_eq!(emu.unknown_hle_calls().len(), 1);

        emu.reset().unwrap();
        assert_eq!(emu.unknown_hle_policy, UnknownHlePolicy::Stop);
        assert!(emu.unknown_hle_allowlist.contains("allowed_missing"));
        assert_eq!(emu.unknown_hle_calls().len(), 0);

        emu.cpu.regs.write(31, 0x8000_5008);
        invoke_sdk_import(&mut emu, 0x3004, "allowed_missing");
        assert!(try_invoke_sdk_import(&mut emu, 0x3008, "Allowed_Missing").is_err());
    }

    #[test]
    fn test_legacy_load_base_requests_lcd_initialization() {
        let legacy = Emulator::from_app(minimal_app()).unwrap();
        assert_eq!(legacy.cpu.regs.read(5), 1);

        let mut standard_app = minimal_app();
        standard_app.rawd.origin = STANDARD_APP_LOAD_BASE;
        let standard = Emulator::from_app(standard_app).unwrap();
        assert_eq!(standard.cpu.regs.read(5), 0);
    }

    #[test]
    fn test_reset_rebuilds_loaded_runtime_state() {
        let mut emu = Emulator::from_app(minimal_app()).unwrap();
        emu.start();
        emu.memory.write_u32(0x1000, 0x1234_5678).unwrap();
        emu.set_buttons(crate::input::BUTTON_A);
        emu.frame_count = 42;
        emu.cycle_count = 123;

        emu.reset().unwrap();

        assert!(emu.is_running());
        assert_eq!(emu.cpu.regs.pc, 0x8000_0000);
        assert_eq!(emu.memory.read_u32(0x1000).unwrap(), 0);
        assert_eq!(emu.input.buttons(), 0);
        assert_eq!(emu.frame_count, 0);
        assert_eq!(emu.cycle_count, 0);
    }

    #[test]
    fn test_guest_task_executes_with_shared_memory() {
        let mut emu = Emulator::default();
        let entry = 0x1000;
        let addiu_t0 = (0x09 << 26) | (8 << 16) | 0x1234;
        let sw_t0 = (0x2B << 26) | (8 << 16) | 0x2000;
        let jr_ra = (31 << 21) | 0x08;
        emu.memory.write_u32(entry, addiu_t0).unwrap();
        emu.memory.write_u32(entry + 4, sw_t0).unwrap();
        emu.memory.write_u32(entry + 8, jr_ra).unwrap();
        emu.memory.write_u32(entry + 12, 0).unwrap();

        assert!(emu.create_guest_task(entry, 0, 0x3000, 16));
        emu.tick().unwrap();

        assert_eq!(emu.memory.read_u32(0x2000).unwrap(), 0x1234);
        assert!(emu.tasks.is_empty());
    }

    #[test]
    fn test_tick_uses_interpreter_cycle_cost() {
        let mut emu = Emulator::default();
        emu.start();

        emu.tick().unwrap();

        assert_eq!(
            emu.cpu.instruction_count,
            CYCLES_PER_FRAME / CPU_CYCLES_PER_INSTRUCTION
        );
        assert_eq!(emu.cycle_count, CYCLES_PER_FRAME);
    }

    #[cfg(feature = "jit")]
    #[test]
    #[ignore = "manual JIT throughput benchmark"]
    fn benchmark_jit_hot_integer_loop() {
        fn make_emulator(jit_enabled: bool, block_count: u32) -> Emulator {
            let mut emu = Emulator::default();
            for block_index in 0..block_count {
                let block_address = 0x8000_0000 + block_index * 64;
                let next_address = 0x8000_0000 + ((block_index + 1) % block_count) * 64;
                let instructions = [
                    (0x09 << 26) | (8 << 21) | (8 << 16) | 1,
                    (9 << 21) | (8 << 16) | (9 << 11) | 0x21,
                    (9 << 21) | (8 << 16) | (10 << 11) | 0x26,
                    (10 << 21) | (8 << 16) | (11 << 11) | 0x25,
                    (11 << 16) | (12 << 11) | (3 << 6),
                    (12 << 16) | (13 << 11) | (2 << 6) | 0x02,
                    (13 << 21) | (8 << 16) | (14 << 11) | 0x24,
                    (0x2b << 26) | (14 << 16) | 0x0200,
                    (0x23 << 26) | (15 << 16) | 0x0200,
                    (15 << 21) | (8 << 16) | (16 << 11) | 0x23,
                    (0x0d << 26) | (16 << 21) | (17 << 16) | 0x55aa,
                    (0x0e << 26) | (17 << 21) | (18 << 16) | 0xa55a,
                    (18 << 21) | (8 << 16) | (19 << 11) | 0x2b,
                    (19 << 21) | (9 << 16) | (20 << 11) | 0x21,
                    (0x02 << 26) | ((next_address >> 2) & 0x03ff_ffff),
                    0,
                ];
                for (index, instruction) in instructions.into_iter().enumerate() {
                    emu.memory
                        .write_u32(block_address + (index as u32) * 4, instruction)
                        .unwrap();
                }
            }
            emu.set_jit_enabled(jit_enabled);
            emu.start();
            emu
        }

        fn measure(mut emu: Emulator) -> (std::time::Duration, std::time::Duration) {
            let cold_start = std::time::Instant::now();
            emu.tick().unwrap();
            let cold = cold_start.elapsed();
            let warm_start = std::time::Instant::now();
            for _ in 0..5 {
                emu.tick().unwrap();
            }
            (cold, warm_start.elapsed())
        }

        let (interpreter_cold, interpreter_warm) = measure(make_emulator(false, 1));
        let (jit_cold, jit_warm) = measure(make_emulator(true, 1));
        eprintln!(
            "cold: interpreter={interpreter_cold:?} jit={jit_cold:?} ratio={:.2}x; warm: interpreter={interpreter_warm:?} jit={jit_warm:?} speedup={:.2}x",
            jit_cold.as_secs_f64() / interpreter_cold.as_secs_f64(),
            interpreter_warm.as_secs_f64() / jit_warm.as_secs_f64()
        );
        let (many_interpreter_cold, many_interpreter_warm) = measure(make_emulator(false, 64));
        let (many_jit_cold, many_jit_warm) = measure(make_emulator(true, 64));
        eprintln!(
            "64 blocks cold: interpreter={many_interpreter_cold:?} jit={many_jit_cold:?} ratio={:.2}x; warm: interpreter={many_interpreter_warm:?} jit={many_jit_warm:?} ratio={:.2}x",
            many_jit_cold.as_secs_f64() / many_interpreter_cold.as_secs_f64(),
            many_jit_warm.as_secs_f64() / many_interpreter_warm.as_secs_f64()
        );
    }

    #[test]
    fn test_tick_stops_after_framebuffer_submission() {
        let mut emu = Emulator::default();
        let hook_address = 0x1000;
        emu.hooked_addrs
            .insert(hook_address, "lcd_set_frame".to_string());
        let (word, mask) = hook_filter_location(hook_address);
        emu.hook_filter[word] |= mask;
        emu.video.framebuffer_mut().fill(0xff);
        emu.cpu.regs.pc = hook_address;
        emu.cpu.regs.write(31, hook_address + 4);
        emu.start();

        emu.tick().unwrap();

        assert_eq!(emu.cpu.regs.pc, hook_address + 4);
        assert_eq!(emu.cpu.instruction_count, 0);
        assert_eq!(emu.cycle_count, CYCLES_PER_FRAME);
        assert!(emu.video.framebuffer().iter().all(|&byte| byte == 0));
    }

    #[test]
    fn test_instruction_blocks_stop_before_sdk_hooks() {
        let mut emu = Emulator::default();
        let hook_address = 0x1020;
        emu.hooked_addrs
            .insert(hook_address, "lcd_set_frame".to_string());
        let (word, mask) = hook_filter_location(hook_address);
        emu.hook_filter[word] |= mask;

        emu.ensure_instruction_block(0x1000).unwrap();

        let cache_index = instruction_block_cache_index(0x1000);
        assert_eq!(emu.instruction_blocks[cache_index].len, 8);
    }

    #[test]
    fn test_instruction_cache_is_cleared_by_guest_invalidation() {
        let mut emu = Emulator::default();
        let cache_index = instruction_block_cache_index(0x1000);
        emu.instruction_blocks[cache_index] = CachedInstructionBlock {
            start: 0x1000,
            len: 1,
            instructions: [0; MAX_INSTRUCTION_BLOCK_LEN],
        };

        invoke_sdk_import(&mut emu, 0x2000, "__icache_invalidate_all");

        assert!(emu.instruction_blocks.iter().all(|block| block.len == 0));
    }

    #[test]
    fn test_hle_modules_execute_through_runtime_hooks() {
        let mut emu = Emulator::default();
        let return_address = 0x9000;
        emu.cpu.regs.write(31, return_address);

        emu.memory.load_data(0x100, b"hello\0").unwrap();
        emu.cpu.regs.write(4, 0x100);
        invoke_sdk_import(&mut emu, 0x1000, "strlen");
        assert_eq!(emu.cpu.regs.read(2), 5);
        assert_eq!(emu.cpu.regs.pc, return_address);

        invoke_sdk_import(&mut emu, 0x1004, "LCD_GetXSize");
        assert_eq!(emu.cpu.regs.read(2), crate::video::SCREEN_WIDTH);

        emu.set_buttons(crate::input::BUTTON_A);
        invoke_sdk_import(&mut emu, 0x1008, "kbd_get_key");
        assert_eq!(emu.cpu.regs.read(2), crate::input::BUTTON_A);

        invoke_sdk_import(&mut emu, 0x100c, "pcm_ioctl");
        assert_eq!(emu.cpu.regs.read(2), 0);

        emu.cpu.regs.write(4, 2);
        invoke_sdk_import(&mut emu, 0x1010, "OSSemCreate");
        assert_eq!(emu.semaphores.get(&emu.cpu.regs.read(2)), Some(&2));

        invoke_sdk_import(&mut emu, 0x1014, "get_dl_handle");
        assert_eq!(emu.cpu.regs.read(2), 0);
    }

    #[test]
    fn test_gui_exec_dispatches_key_transitions_to_focused_guest_window() {
        let mut emu = Emulator::default();
        let return_address = 0x9000;
        let callback = 0x4000;
        let stack_pointer = 0x3000;
        emu.cpu.regs.write(29, stack_pointer);
        emu.cpu.regs.write(31, return_address);
        emu.memory.write_u32(stack_pointer + 20, callback).unwrap();

        invoke_sdk_import(&mut emu, 0x1100, "WM_CreateWindow");
        let window = emu.cpu.regs.read(2);
        assert_ne!(window, 0);

        emu.cpu.regs.write(4, window);
        invoke_sdk_import(&mut emu, 0x1104, "WM_SetFocus");
        invoke_sdk_import(&mut emu, 0x1108, "open_gui_key_msg");

        emu.set_buttons(crate::input::BUTTON_RIGHT);
        invoke_sdk_import(&mut emu, 0x110c, "GUI_Exec");
        assert_eq!(emu.cpu.regs.pc, callback);
        assert_eq!(emu.cpu.regs.read(31), return_address);
        let message = emu.cpu.regs.read(4);
        assert_eq!(emu.memory.read_u32(message).unwrap(), 14);
        assert_eq!(emu.memory.read_u32(message + 4).unwrap(), window);
        let key_info = emu.memory.read_u32(message + 8).unwrap();
        assert_eq!(emu.memory.read_u32(key_info).unwrap(), 18);
        assert_eq!(emu.memory.read_u32(key_info + 4).unwrap(), 1);

        emu.cpu.regs.write(31, return_address);
        invoke_sdk_import(&mut emu, 0x110c, "GUI_Exec");
        assert_eq!(emu.cpu.regs.pc, return_address);

        emu.set_buttons(0);
        emu.cpu.regs.write(31, return_address);
        invoke_sdk_import(&mut emu, 0x110c, "GUI_Exec");
        assert_eq!(emu.cpu.regs.pc, callback);
        assert_eq!(emu.memory.read_u32(key_info).unwrap(), 18);
        assert_eq!(emu.memory.read_u32(key_info + 4).unwrap(), 0);
    }

    #[test]
    fn test_semaphore_wakes_waiting_main_task() {
        let mut emu = Emulator::default();
        let semaphore = emu.create_semaphore(0);

        assert!(emu.pend_semaphore(semaphore));
        assert_eq!(emu.main_wait, Some(TaskWait::Semaphore(semaphore)));
        assert!(emu.post_semaphore(semaphore));
        assert_eq!(emu.main_wait, None);
    }

    #[test]
    fn test_packed_bin_resource_view_inserts_header() {
        let mut data = vec![0; 24];
        data[0..2].copy_from_slice(&1_u16.to_le_bytes());
        data[4..8].copy_from_slice(&0x1122_3344_u32.to_le_bytes());
        data[8..12].copy_from_slice(&0x260a_1300_u32.to_le_bytes());
        data[12..24].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);

        let view = prepare_resource_file_data("brick.bin", ResourceKind::Packed, data);

        assert_eq!(view.len(), 40);
        assert_eq!(u32::from_le_bytes(view[0..4].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(view[4..8].try_into().unwrap()), 0);
        assert_eq!(u32::from_le_bytes(view[8..12].try_into().unwrap()), 20);
        assert_eq!(&view[12..16], &0x1122_3344_u32.to_le_bytes());
        assert_eq!(&view[16..20], &[0; 4]);
        assert_eq!(&view[20..32], &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);
        assert_eq!(&view[32..40], &[0; 8]);
    }

    #[test]
    fn test_regular_resource_data_remains_unchanged() {
        let mut data = vec![0x5a; 12];
        data[8..12].copy_from_slice(&12_u32.to_le_bytes());

        assert_eq!(
            prepare_resource_file_data("image.bin", ResourceKind::Packed, data.clone()),
            data
        );
    }

    #[test]
    fn test_guest_timers_advance_with_emulated_cycles() {
        let mut emu = Emulator {
            cycle_count: CPU_CLOCK_HZ / OS_TICKS_PER_SECOND,
            ..Default::default()
        };

        invoke_sdk_import(&mut emu, 0, "OSTimeGet");
        assert_eq!(emu.cpu.regs.read(2), 1);

        invoke_sdk_import(&mut emu, 0, "GetTickCount");
        assert_eq!(emu.cpu.regs.read(2), 10_000);
    }

    #[test]
    fn test_sprintf_builds_guest_path() {
        let mut emu = Emulator::default();
        let destination = 0x8001_0000;
        let format = 0x8001_0100;
        let directory = 0x8001_0200;
        emu.memory.load_data(format, b"%ssplash.tga\0").unwrap();
        emu.memory.load_data(directory, b"games/astro/\0").unwrap();
        emu.cpu.regs.write(4, destination);
        emu.cpu.regs.write(5, format);
        emu.cpu.regs.write(6, directory);

        invoke_sdk_import(&mut emu, 0, "sprintf");

        assert_eq!(
            emu.read_guest_c_string(destination),
            "games/astro/splash.tga"
        );
        assert_eq!(emu.cpu.regs.read(2), 22);
    }

    #[test]
    fn test_sprintf_reads_stack_varargs() {
        let mut emu = Emulator::default();
        let destination = 0x8001_0000;
        let format = 0x8001_0100;
        let stack = 0x8001_1000;
        emu.memory
            .load_data(format, b"Ver: %lu.%lu.%04lu\0")
            .unwrap();
        emu.cpu.regs.write(4, destination);
        emu.cpu.regs.write(5, format);
        emu.cpu.regs.write(6, 1);
        emu.cpu.regs.write(7, 2);
        emu.cpu.regs.write(29, stack);
        emu.memory.write_u32(stack + 16, 3).unwrap();

        invoke_sdk_import(&mut emu, 0, "sprintf");

        assert_eq!(emu.read_guest_c_string(destination), "Ver: 1.2.0003");
        assert_eq!(emu.cpu.regs.read(2), 13);
    }

    #[test]
    fn test_app_main_receives_file_name() {
        let mut emu = Emulator {
            app_path: "games/astro/Astro-Lander.app".to_string(),
            ..Default::default()
        };

        emu.install_app_main_args().unwrap();

        assert_eq!(
            emu.read_guest_w_string(emu.cpu.regs.read(4)),
            "Astro-Lander.app"
        );
    }

    #[test]
    fn test_host_file_path_resolves_from_app_directory() {
        let emu = Emulator {
            app_path: "games/astro/Astro-Lander.app".to_string(),
            ..Default::default()
        };

        assert_eq!(
            emu.resolve_host_file_path(r"assets\splash.tga"),
            Path::new("games")
                .join("astro")
                .join("assets")
                .join("splash.tga")
        );
    }

    #[test]
    fn test_to_locale_ansi_preserves_input_and_reuses_output_buffer() {
        let mut emu = Emulator::default();
        let ptr = 0x100;
        for (index, word) in "Ali中.app\0".encode_utf16().enumerate() {
            emu.memory
                .write_u16(ptr + (index as u32 * 2), word)
                .unwrap();
        }
        emu.cpu.regs.write(4, ptr);
        emu.cpu.regs.write(31, 0x1234);

        invoke_sdk_import(&mut emu, 0, "__to_locale_ansi");

        let output = emu.cpu.regs.read(2);
        assert_ne!(output, ptr);
        assert_eq!(emu.cpu.regs.pc, 0x1234);
        assert_eq!(
            (0..9)
                .map(|offset| emu.memory.read_u8(output + offset).unwrap())
                .collect::<Vec<_>>(),
            b"Ali?.app\0"
        );
        assert_eq!(emu.read_guest_w_string(ptr), "Ali中.app");

        emu.cpu.regs.write(4, ptr);
        invoke_sdk_import(&mut emu, 0, "__to_locale_ansi");

        assert_eq!(emu.cpu.regs.read(2), output);
        assert_eq!(emu.read_guest_c_string(output), "Ali?.app");
    }

    #[test]
    fn test_get_system_model_writes_a320_as_utf16() {
        let mut emu = Emulator::default();
        let ptr = 0x100;
        emu.cpu.regs.write(4, ptr);
        emu.cpu.regs.write(31, 0x1234);

        invoke_sdk_import(&mut emu, 0, "cmGetSysModel");

        assert_eq!(emu.cpu.regs.read(2), 0);
        assert_eq!(emu.cpu.regs.pc, 0x1234);
        assert_eq!(emu.read_guest_w_string(ptr), "A320");
    }

    #[test]
    fn test_u8_conversion_aliases_read_little_endian_values() {
        let mut emu = Emulator::default();
        let ptr = 0x100;
        emu.memory
            .load_data(ptr, &[0x18, 0xC2, 0x01, 0x00])
            .unwrap();
        emu.cpu.regs.write(4, ptr);

        for name in ["U8TOU16", "U8TOX16"] {
            invoke_sdk_import(&mut emu, 0, name);
            assert_eq!(emu.cpu.regs.read(2), 0xC218);
        }

        for name in ["U8TOU32", "U8TOX32"] {
            invoke_sdk_import(&mut emu, 0, name);
            assert_eq!(emu.cpu.regs.read(2), 0x0001_C218);
        }
    }

    #[test]
    fn test_lcd_size_matches_a320_display() {
        let mut emu = Emulator::default();

        invoke_sdk_import(&mut emu, 0, "LCD_GetXSize");
        assert_eq!(emu.cpu.regs.read(2), crate::video::SCREEN_WIDTH);

        invoke_sdk_import(&mut emu, 0, "LCD_GetYSize");
        assert_eq!(emu.cpu.regs.read(2), crate::video::SCREEN_HEIGHT);
    }

    #[cfg(not(feature = "standalone"))]
    #[test]
    fn test_waveout_hle_opens_and_queues_guest_pcm() {
        let mut emu = Emulator::default();
        let args_ptr = 0x1000;
        emu.memory.write_u32(args_ptr, 16_000).unwrap();
        emu.memory.write_u16(args_ptr + 4, 16).unwrap();
        emu.memory.write_u8(args_ptr + 6, 1).unwrap();
        emu.memory.write_u8(args_ptr + 7, 100).unwrap();
        emu.cpu.regs.write(4, args_ptr);
        invoke_sdk_import(&mut emu, 0, "waveout_open");

        assert_eq!(emu.cpu.regs.read(2), 1);
        assert_eq!(emu.audio.config(), AudioConfig::new(16_000, 16, 1, 100));

        let buffer_ptr = 0x2000;
        for index in 0..1_600u32 {
            let sample = if index % 2 == 0 {
                10_000i16
            } else {
                -10_000i16
            };
            emu.memory
                .write_u16(buffer_ptr + index * 2, sample as u16)
                .unwrap();
        }
        emu.cpu.regs.write(5, buffer_ptr);
        emu.cpu.regs.write(6, 3_200);
        invoke_sdk_import(&mut emu, 0, "waveout_write");

        assert_eq!(emu.cpu.regs.read(2), 1);
        assert!(emu.take_audio_samples().iter().any(|&sample| sample != 0));
    }

    #[cfg(not(feature = "standalone"))]
    #[test]
    fn test_waveout_write_retries_after_queue_space_is_available() {
        let mut emu = Emulator::default();
        let config = AudioConfig::new(16_000, 16, 1, 100).unwrap();
        assert!(emu.audio.open(config));
        assert!(emu.audio.write(&vec![0; 32_000]));
        assert!(!emu.audio.can_write());

        let hook_address = 0x4000;
        let return_address = 0x1234;
        let buffer_ptr = 0x2000;
        emu.memory
            .load_data(buffer_ptr, &1_000i16.to_le_bytes())
            .unwrap();
        emu.cpu.regs.pc = hook_address;
        emu.cpu.regs.write(2, 0xdead_beef);
        emu.cpu.regs.write(5, buffer_ptr);
        emu.cpu.regs.write(6, 2);
        emu.cpu.regs.write(31, return_address);

        invoke_sdk_import(&mut emu, hook_address, "waveout_write");

        assert_eq!(emu.main_wait, Some(TaskWait::AudioWrite));
        assert_eq!(emu.cpu.regs.pc, hook_address);
        assert_eq!(emu.cpu.regs.read(2), 0xdead_beef);

        for _ in 0..60 {
            if emu.audio.can_write() {
                break;
            }
            emu.take_audio_samples();
        }
        assert!(emu.audio.can_write());
        assert!(!emu.active_context_waiting());

        invoke_sdk_import(&mut emu, hook_address, "waveout_write");

        assert_eq!(emu.main_wait, None);
        assert_eq!(emu.cpu.regs.pc, return_address);
        assert_eq!(emu.cpu.regs.read(2), 1);
    }

    #[test]
    fn test_writable_file_persists_and_reopens() {
        let mut emu = Emulator::default();
        let save_directory = std::env::temp_dir().join(format!(
            "dingooemu-save-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        emu.set_save_directory(&save_directory);
        emu.memory.load_data(0x100, b"test.log\0").unwrap();
        emu.memory.load_data(0x120, b"w\0").unwrap();
        emu.memory.load_data(0x140, b"abcdef").unwrap();
        emu.cpu.regs.write(4, 0x100);
        emu.cpu.regs.write(5, 0x120);

        invoke_sdk_import(&mut emu, 0, "fsys_fopen");
        let handle = emu.cpu.regs.read(2);
        assert_ne!(handle, 0);

        emu.cpu.regs.write(4, 0x140);
        emu.cpu.regs.write(5, 2);
        emu.cpu.regs.write(6, 3);
        emu.cpu.regs.write(7, handle);
        invoke_sdk_import(&mut emu, 0, "fsys_fwrite");

        assert_eq!(emu.cpu.regs.read(2), 3);
        assert_eq!(emu.open_files[&handle].data, b"abcdef");

        emu.cpu.regs.write(4, handle);
        invoke_sdk_import(&mut emu, 0, "fsys_fclose");
        assert_eq!(emu.cpu.regs.read(2), 0);
        assert_eq!(
            std::fs::read(save_directory.join("test.log")).unwrap(),
            b"abcdef"
        );

        emu.memory.load_data(0x120, b"r\0").unwrap();
        emu.cpu.regs.write(4, 0x100);
        emu.cpu.regs.write(5, 0x120);
        invoke_sdk_import(&mut emu, 0, "fsys_fopen");
        let reopened = emu.cpu.regs.read(2);
        assert_eq!(emu.open_files[&reopened].data, b"abcdef");

        std::fs::remove_dir_all(save_directory).unwrap();
    }

    #[test]
    fn test_read_only_file_can_be_read_but_not_written() {
        let mut emu = Emulator::default();
        let handle = 7;
        emu.open_files.insert(
            handle,
            OpenFile {
                data: b"resource".to_vec(),
                position: 0,
                data_ptr: 0,
                save_path: None,
                writable: false,
                dirty: false,
            },
        );

        assert_eq!(emu.read_file(0x100, 1, 8, handle).unwrap(), 8);
        let read_back = (0..8)
            .map(|offset| emu.memory.read_u8(0x100 + offset).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(read_back, b"resource");

        emu.open_files.get_mut(&handle).unwrap().position = 0;
        emu.memory.load_data(0x200, b"modified").unwrap();
        assert_eq!(emu.write_file(0x200, 1, 8, handle).unwrap(), 0);
        assert_eq!(emu.open_files[&handle].data, b"resource");
        assert!(!emu.open_files[&handle].dirty);
    }

    #[test]
    fn test_guest_save_path_cannot_escape_save_directory() {
        let mut emu = Emulator::default();
        emu.set_save_directory(std::env::temp_dir().join("dingooemu-save-root"));

        assert!(emu.save_file_path("save/profile.dat").is_some());
        assert!(emu.save_file_path("A:\\save\\profile.dat").is_some());
        assert!(emu.save_file_path("../outside.dat").is_none());
        assert!(emu.save_file_path("save/../../outside.dat").is_none());
    }

    #[test]
    fn test_file_search_enumerates_matching_entries_and_terminates() {
        let directory =
            std::env::temp_dir().join(format!("dingooemu-file-search-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(directory.join("Music")).unwrap();
        std::fs::write(directory.join("Dn-Beyond.xm"), b"xm").unwrap();
        std::fs::write(directory.join("Fountain.mod"), b"mod").unwrap();

        let mut emu = Emulator {
            app_path: directory.join("GooPlayer.app").display().to_string(),
            ..Default::default()
        };
        emu.memory.load_data(0x100, b"*\0").unwrap();
        emu.cpu.regs.write(4, 0x100);
        emu.cpu.regs.write(5, 0x10);
        emu.cpu.regs.write(6, 0x200);
        invoke_sdk_import(&mut emu, 0x1000, "fsys_findfirst");
        assert_eq!(emu.cpu.regs.read(2), 0);
        assert_eq!(
            emu.read_guest_c_string(0x200 + FILE_SEARCH_NAME_OFFSET),
            "Music"
        );

        emu.cpu.regs.write(4, 0x200);
        invoke_sdk_import(&mut emu, 0x1004, "fsys_findnext");
        assert_eq!(emu.cpu.regs.read(2), u32::MAX);

        emu.cpu.regs.write(4, 0x100);
        emu.cpu.regs.write(5, 0);
        emu.cpu.regs.write(6, 0x300);
        invoke_sdk_import(&mut emu, 0x1008, "fsys_findfirst");
        assert_eq!(emu.cpu.regs.read(2), 0);
        assert_eq!(
            emu.read_guest_c_string(0x300 + FILE_SEARCH_NAME_OFFSET),
            "Dn-Beyond.xm"
        );

        emu.cpu.regs.write(4, 0x300);
        invoke_sdk_import(&mut emu, 0x100c, "fsys_findnext");
        assert_eq!(emu.cpu.regs.read(2), 0);
        assert_eq!(
            emu.read_guest_c_string(0x300 + FILE_SEARCH_NAME_OFFSET),
            "Fountain.mod"
        );

        emu.cpu.regs.write(4, 0x200);
        invoke_sdk_import(&mut emu, 0x1010, "fsys_findclose");
        assert_eq!(emu.cpu.regs.read(2), 0);
        assert!(!emu.file_searches.contains_key(&0x200));

        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn test_file_search_filters_wildcards_and_rejects_parent_paths() {
        let directory =
            std::env::temp_dir().join(format!("dingooemu-file-filter-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("Track.XM"), b"xm").unwrap();
        std::fs::write(directory.join("Track.mod"), b"mod").unwrap();

        let mut emu = Emulator {
            app_path: directory.join("GooPlayer.app").display().to_string(),
            ..Default::default()
        };
        assert_eq!(emu.begin_file_search("*.xm", 0, 0x200).unwrap(), 0);
        assert_eq!(
            emu.read_guest_c_string(0x200 + FILE_SEARCH_NAME_OFFSET),
            "Track.XM"
        );
        assert_eq!(emu.next_file_search(0x200).unwrap(), u32::MAX);
        assert_eq!(
            emu.begin_file_search("../*", 0x10, 0x300).unwrap(),
            u32::MAX
        );

        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn test_save_state_round_trip_and_transactional_rejection() {
        let mut emu = Emulator::from_app(minimal_app()).unwrap();
        let save_directory = std::env::temp_dir().join("dingooemu-state-save-root");
        emu.set_save_directory(&save_directory);
        emu.start();
        emu.cpu.regs.write(8, 0x1234_5678);
        emu.memory.write_u32(0x2000, 0xaabb_ccdd).unwrap();
        emu.input.set_buttons(crate::input::BUTTON_A);
        emu.frame_count = 42;
        emu.cycle_count = 987_654;
        emu.open_files.insert(
            7,
            OpenFile {
                data: b"open save data".to_vec(),
                position: 4,
                data_ptr: 0,
                save_path: Some(save_directory.join("profile.dat")),
                writable: true,
                dirty: true,
            },
        );
        emu.file_searches.insert(
            0x3000,
            FileSearch {
                entries: vec!["first.mod".to_string(), "second.xm".to_string()],
                next_index: 1,
            },
        );
        emu.gui.key_messages_enabled = true;
        emu.gui.windows.insert(3, 0x8000_4000);
        emu.gui.focused_window = Some(3);
        emu.gui.next_window_handle = 4;
        emu.gui.reported_key = 18;
        emu.gui.message_buffer = Some(0x2100);
        emu.gui.key_info_buffer = Some(0x2200);

        let mut state = vec![0; emu.serialized_state_size()];
        emu.serialize_state(&mut state).unwrap();
        emu.cpu.regs.write(8, 0);
        emu.memory.write_u32(0x2000, 0).unwrap();
        emu.input.set_buttons(0);
        emu.frame_count = 0;
        emu.open_files.clear();
        emu.file_searches.clear();
        emu.gui = GuiState::default();

        emu.unserialize_state(&state).unwrap();
        assert_eq!(emu.cpu.regs.read(8), 0x1234_5678);
        assert_eq!(emu.memory.read_u32(0x2000).unwrap(), 0xaabb_ccdd);
        assert_eq!(emu.input.buttons(), crate::input::BUTTON_A);
        assert_eq!(emu.frame_count, 42);
        assert_eq!(emu.cycle_count, 987_654);
        assert!(emu.is_running());
        assert_eq!(emu.open_files[&7].data, b"open save data");
        assert_eq!(emu.file_searches[&0x3000].next_index, 1);
        assert_eq!(emu.file_searches[&0x3000].entries[1], "second.xm");
        assert!(emu.gui.key_messages_enabled);
        assert_eq!(emu.gui.windows[&3], 0x8000_4000);
        assert_eq!(emu.gui.focused_window, Some(3));
        assert_eq!(emu.gui.next_window_handle, 4);
        assert_eq!(emu.gui.reported_key, 18);
        assert_eq!(emu.gui.message_buffer, Some(0x2100));
        assert_eq!(emu.gui.key_info_buffer, Some(0x2200));
        assert_eq!(
            emu.open_files[&7].save_path.as_deref(),
            Some(save_directory.join("profile.dat").as_path())
        );

        let unchanged_pc = emu.cpu.regs.pc;
        let unchanged_memory = emu.memory.read_u32(0x2000).unwrap();
        state[32] ^= 1;
        assert!(emu.unserialize_state(&state).is_err());
        assert_eq!(emu.cpu.regs.pc, unchanged_pc);
        assert_eq!(emu.memory.read_u32(0x2000).unwrap(), unchanged_memory);

        state[32] ^= 1;
        let mut other_app = minimal_app();
        other_app.data.push(1);
        let mut other = Emulator::from_app(other_app).unwrap();
        other.cpu.regs.pc = 0x8765_4321;
        assert!(other.unserialize_state(&state).is_err());
        assert_eq!(other.cpu.regs.pc, 0x8765_4321);
    }
}
