use crate::app_loader::{AppImage, ResourceKind};
use crate::audio::{Audio, AudioConfig};
use crate::cpu::Cpu;
use crate::error::Result;
use crate::input::Input;
use crate::memory::Memory;
use crate::sdk_hle::SdkHle;
use crate::video::Video;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const CPU_CLOCK_HZ: u64 = 336_000_000;
const FRAMES_PER_SECOND: u64 = 60;
const OS_TICKS_PER_SECOND: u64 = 100;
const CYCLES_PER_FRAME: u64 = CPU_CLOCK_HZ / FRAMES_PER_SECOND;
// Model the pipeline and memory stalls with a conservative average CPI.
const CPU_CYCLES_PER_INSTRUCTION: u64 = 2;
const MAX_AUDIO_WRITE_BYTES: u32 = 4 * 1024 * 1024;
const TASK_QUANTUM_CYCLES: u64 = 4_096;
const TASK_RETURN_ADDRESS: u32 = u32::MAX;
const MAX_GUEST_TASKS: usize = 16;
const HOOK_FILTER_WORDS: usize = 1_024;

fn hook_filter_location(address: u32) -> (usize, u64) {
    let bit_index = (address as usize >> 2) & (HOOK_FILTER_WORDS * u64::BITS as usize - 1);
    (
        bit_index / u64::BITS as usize,
        1 << (bit_index % u64::BITS as usize),
    )
}

struct OpenFile {
    data: Vec<u8>,
    position: usize,
    data_ptr: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TaskWait {
    AudioWrite,
    Semaphore(u32),
    UntilCycle(u64),
}

struct GuestTask {
    cpu: Cpu,
    priority: u32,
    wait: Option<TaskWait>,
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
    /// SDK HLE bridge
    pub sdk: SdkHle,
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
    /// Fast rejection filter for non-hook instruction addresses
    hook_filter: Box<[u64]>,
    /// Open guest resource files
    open_files: HashMap<u32, OpenFile>,
    /// Next guest file handle
    next_file_handle: u32,
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
        cpu.regs.write(5, 0);

        // Initialize stack pointer to a reasonable value in RAM
        // Stack grows downward from top of RAM (32MB)
        cpu.regs.write(29, 0x01FF_FFF0); // $sp = top of RAM - 16

        // Use a fixed guest-visible framebuffer address
        // The game writes directly to this address
        let video = Video::new();

        let input = Input::new();
        let audio = Audio::new();
        let sdk = SdkHle::new();

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

        Ok(Self {
            cpu,
            memory,
            video,
            input,
            audio,
            sdk,
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
            hook_filter,
            open_files: HashMap::new(),
            next_file_handle: 1,
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

    /// Stop the emulator
    pub fn stop(&mut self) {
        self.cpu.stop();
        log::info!("Emulator stopped");
    }

    /// Rebuild all mutable runtime state from the loaded app image.
    pub fn reset(&mut self) -> Result<()> {
        let app = self
            .app
            .clone()
            .ok_or_else(|| "cannot reset an emulator without a loaded app".to_string())?;
        let was_running = self.is_running();
        let mut replacement = Self::from_app_with_path(app, self.app_path.clone())?;
        if was_running {
            replacement.start();
        }
        *self = replacement;
        log::info!("Emulator reset");
        Ok(())
    }

    /// Run one frame of emulation
    pub fn tick(&mut self) -> Result<()> {
        self.framebuffer_submitted = false;

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
                self.handle_sdk_call(pc, &func_name)?;
                self.cycle_count = self.cycle_count.wrapping_add(CPU_CYCLES_PER_INSTRUCTION);
                executed += CPU_CYCLES_PER_INSTRUCTION;
                if self.framebuffer_submitted
                    || self.active_context_waiting()
                    || !self.cpu.is_running()
                {
                    break;
                }
            } else {
                self.cpu.step(&mut self.memory)?;
                self.cycle_count = self.cycle_count.wrapping_add(CPU_CYCLES_PER_INSTRUCTION);
                executed += CPU_CYCLES_PER_INSTRUCTION;
            }
        }
        Ok(executed)
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

    fn open_memory_file(&mut self) -> u32 {
        let handle = self.next_file_handle;
        self.next_file_handle = self.next_file_handle.wrapping_add(1).max(1);
        self.open_files.insert(
            handle,
            OpenFile {
                data: Vec::new(),
                position: 0,
                data_ptr: 0,
            },
        );
        handle
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
        let end = file.position.saturating_add(requested);
        if file.data.len() < end {
            file.data.resize(end, 0);
        }
        file.data[file.position..end].copy_from_slice(&data);
        file.position = end;

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
    /// Handle SDK function call at import address
    fn handle_sdk_call(&mut self, addr: u32, func_name: &str) -> Result<()> {
        log::trace!("SDK call: {:#010x} = {}", addr, func_name);

        // Save return address from $ra
        let ra = self.cpu.regs.read(31);

        match func_name {
            // Memory management
            "malloc" => {
                let size = self.cpu.regs.read(4); // $a0
                let ptr = self.memory.malloc(size);
                self.cpu.regs.write(2, ptr); // $v0
                log::info!(
                    "  malloc({}) = {:#010x} (heap_ptr={:#010x})",
                    size,
                    ptr,
                    self.memory.heap_ptr()
                );
            }
            "free" => {
                let ptr = self.cpu.regs.read(4); // $a0
                self.memory.free(ptr);
                log::trace!("  free({:#010x})", ptr);
            }
            "realloc" => {
                let ptr = self.cpu.regs.read(4); // $a0
                let size = self.cpu.regs.read(5); // $a1
                let new_ptr = self.memory.realloc(ptr, size);
                self.cpu.regs.write(2, new_ptr); // $v0
                log::trace!("  realloc({:#010x}, {}) = {:#010x}", ptr, size, new_ptr);
            }
            "memset" => {
                let ptr = self.cpu.regs.read(4); // $a0
                let value = self.cpu.regs.read(5) as u8; // $a1
                let size = self.cpu.regs.read(6); // $a2
                self.memory.memset(ptr, value, size);
                self.cpu.regs.write(2, ptr); // $v0
                log::trace!("  memset({:#010x}, {:#04x}, {})", ptr, value, size);
            }
            "memcpy" => {
                let dest = self.cpu.regs.read(4); // $a0
                let src = self.cpu.regs.read(5); // $a1
                let size = self.cpu.regs.read(6); // $a2
                self.memory.memcpy(dest, src, size)?;
                self.cpu.regs.write(2, dest); // $v0
                log::trace!("  memcpy({:#010x}, {:#010x}, {})", dest, src, size);
            }
            "strlen" => {
                let ptr = self.cpu.regs.read(4); // $a0
                let len = self.memory.read_string_len(ptr);
                self.cpu.regs.write(2, len); // $v0
                log::trace!("  strlen({:#010x}) = {}", ptr, len);
            }
            "__to_locale_ansi" => {
                let ptr = self.cpu.regs.read(4);
                let result = self.convert_guest_w_string_to_ansi(ptr);
                self.cpu.regs.write(2, result);
                log::trace!("  __to_locale_ansi({:#010x}) = {:#010x}", ptr, result);
            }
            "cmGetSysModel" => {
                let ptr = self.cpu.regs.read(4);
                for (index, word) in "A320\0".encode_utf16().enumerate() {
                    self.memory
                        .write_u16(ptr.wrapping_add((index * 2) as u32), word)?;
                }
                self.cpu.regs.write(2, 0);
                log::trace!("  cmGetSysModel({:#010x}) = 0", ptr);
            }
            "U8TOU32" => {
                let ptr = self.cpu.regs.read(4);
                let value = self.memory.read_u32(ptr)?;
                self.cpu.regs.write(2, value);
                log::trace!("  U8TOU32({:#010x}) = {:#010x}", ptr, value);
            }

            // Graphics/LCD
            "_lcd_get_frame" | "lcd_get_frame" | "lcd_get_cframe" => {
                // Return fixed guest-visible framebuffer address
                // The game writes directly to this address
                let phys_addr = crate::video::VM_LCD_FB_ADDRESS;
                self.cpu.regs.write(2, phys_addr); // $v0
                log::trace!("  lcd_get_frame() = {:#010x}", phys_addr);
            }
            "_lcd_set_frame" | "lcd_set_frame" | "ap_lcd_set_frame" => {
                self.sync_framebuffer();
                log::trace!("  lcd_set_frame() - framebuffer updated");
            }
            "lcd_flip" => {
                self.sync_framebuffer();
                log::trace!("  lcd_flip() - framebuffer updated");
            }
            "LcdGetDisMode" => {
                self.cpu.regs.write(2, 0); // $v0 = 0
                log::trace!("  LcdGetDisMode() = 0");
            }
            "LCD_GetXSize" => {
                self.cpu.regs.write(2, crate::video::SCREEN_WIDTH);
                log::trace!("  LCD_GetXSize() = {}", crate::video::SCREEN_WIDTH);
            }
            "LCD_GetYSize" => {
                self.cpu.regs.write(2, crate::video::SCREEN_HEIGHT);
                log::trace!("  LCD_GetYSize() = {}", crate::video::SCREEN_HEIGHT);
            }

            // Input
            "_kbd_get_status" | "kbd_get_status" => {
                let status_ptr = self.cpu.regs.read(4); // $a0
                let (pressed, released, status) = self.input.take_status();
                self.memory.write_u32(status_ptr, pressed)?;
                self.memory
                    .write_u32(status_ptr.wrapping_add(4), released)?;
                self.memory.write_u32(status_ptr.wrapping_add(8), status)?;
                log::trace!(
                    "  kbd_get_status({:#010x}) pressed={:#010x} released={:#010x} status={:#010x}",
                    status_ptr,
                    pressed,
                    released,
                    status
                );
            }
            "_kbd_get_key" | "kbd_get_key" => {
                // Convert bitmask to key code
                let buttons = self.input.buttons();
                let key = if buttons & crate::input::BUTTON_UP != 0 {
                    20
                } else if buttons & crate::input::BUTTON_DOWN != 0 {
                    27
                } else if buttons & crate::input::BUTTON_LEFT != 0 {
                    28
                } else if buttons & crate::input::BUTTON_RIGHT != 0 {
                    18
                } else if buttons & crate::input::BUTTON_A != 0 {
                    31
                } else if buttons & crate::input::BUTTON_B != 0 {
                    21
                } else if buttons & crate::input::BUTTON_X != 0 {
                    16
                } else if buttons & crate::input::BUTTON_Y != 0 {
                    6
                } else if buttons & crate::input::BUTTON_START != 0 {
                    11
                } else if buttons & crate::input::BUTTON_SELECT != 0 {
                    10
                } else if buttons & crate::input::BUTTON_L != 0 {
                    8
                } else if buttons & crate::input::BUTTON_R != 0 {
                    29
                } else {
                    0
                };
                self.cpu.regs.write(2, key); // $v0
                log::trace!("  kbd_get_key() = {}", key);
            }

            // Tasks and synchronization
            "OSTaskCreate" => {
                let entry = self.cpu.regs.read(4);
                let data_ptr = self.cpu.regs.read(5);
                let stack_ptr = self.cpu.regs.read(6);
                let priority = self.cpu.regs.read(7);
                let created = self.create_guest_task(entry, data_ptr, stack_ptr, priority);
                self.cpu.regs.write(2, if created { 0 } else { u32::MAX });
                log::trace!(
                    "  OSTaskCreate({entry:#010x}, {data_ptr:#010x}, {stack_ptr:#010x}, {priority}) = {}",
                    self.cpu.regs.read(2)
                );
            }
            "OSSemCreate" => {
                let count = self.cpu.regs.read(4);
                let handle = self.create_semaphore(count);
                self.cpu.regs.write(2, handle);
                log::trace!("  OSSemCreate({count}) = {handle}");
            }
            "OSSemPend" => {
                let handle = self.cpu.regs.read(4);
                let error_ptr = self.cpu.regs.read(6);
                let pending = self.pend_semaphore(handle);
                if error_ptr != 0 {
                    self.memory
                        .write_u8(error_ptr, if pending { 0 } else { 1 })?;
                }
                log::trace!("  OSSemPend({handle}) = {pending}");
            }
            "OSSemPost" => {
                let handle = self.cpu.regs.read(4);
                let posted = self.post_semaphore(handle);
                self.cpu.regs.write(2, if posted { 0 } else { 1 });
                log::trace!("  OSSemPost({handle}) = {}", self.cpu.regs.read(2));
            }
            "OSSemDel" => {
                let handle = self.cpu.regs.read(4);
                let error_ptr = self.cpu.regs.read(6);
                let removed = self.semaphores.remove(&handle).is_some();
                if error_ptr != 0 {
                    self.memory
                        .write_u8(error_ptr, if removed { 0 } else { 1 })?;
                }
                self.cpu.regs.write(2, 0);
                log::trace!("  OSSemDel({handle}) = {removed}");
            }

            // Audio
            "_waveout_open" | "waveout_open" => {
                let args_ptr = self.cpu.regs.read(4);
                let config = AudioConfig::new(
                    self.memory.read_u32(args_ptr)?,
                    self.memory.read_u16(args_ptr.wrapping_add(4))?,
                    self.memory.read_u8(args_ptr.wrapping_add(6))?,
                    self.memory.read_u8(args_ptr.wrapping_add(7))?,
                );
                let opened = config.is_some_and(|config| self.audio.open(config));
                self.cpu.regs.write(2, u32::from(opened));
                log::trace!("  waveout_open({args_ptr:#010x}) = {opened}");
            }
            "waveout_write" => {
                let buffer_ptr = self.cpu.regs.read(5);
                let count = self.cpu.regs.read(6);
                let written = if count == 0 || count > MAX_AUDIO_WRITE_BYTES {
                    false
                } else if !self.audio.can_write() {
                    self.set_active_wait(TaskWait::AudioWrite);
                    log::trace!(
                        "  waveout_write({buffer_ptr:#010x}, {count}) deferred until queue space is available"
                    );
                    return Ok(());
                } else {
                    let mut data = Vec::with_capacity(count as usize);
                    for offset in 0..count {
                        data.push(self.memory.read_u8(buffer_ptr.wrapping_add(offset))?);
                    }
                    self.audio.write(&data)
                };
                self.cpu.regs.write(2, u32::from(written));
                log::trace!(
                    "  waveout_write({buffer_ptr:#010x}, {count}) = {}",
                    u32::from(written)
                );
            }
            "waveout_can_write" | "pcm_can_write" => {
                let can_write = self.audio.can_write();
                self.cpu.regs.write(2, u32::from(can_write));
                log::trace!("  {func_name}() = {}", u32::from(can_write));
            }
            "waveout_close" | "waveout_close_at_once" => {
                let closed = self.audio.close();
                self.cpu.regs.write(2, u32::from(closed));
                log::trace!("  {func_name}() = {}", u32::from(closed));
            }
            "_waveout_set_volume" | "waveout_set_volume" => {
                let volume = self.cpu.regs.read(4);
                let updated = self.audio.set_volume(volume);
                self.cpu.regs.write(2, u32::from(updated));
                log::trace!("  {func_name}({volume}) = {}", u32::from(updated));
            }
            "HP_Mute_sw" => {
                let muted = self.cpu.regs.read(4) != 0;
                let updated = self.audio.set_muted(muted);
                self.cpu.regs.write(2, u32::from(updated));
                log::trace!("  HP_Mute_sw({muted}) = {}", u32::from(updated));
            }
            "pcm_ioctl" => {
                self.cpu.regs.write(2, 0);
                log::trace!("  pcm_ioctl() = 0");
            }

            // Timer
            "OSTimeGet" => {
                let ticks = self
                    .cycle_count
                    .saturating_mul(OS_TICKS_PER_SECOND)
                    .checked_div(CPU_CLOCK_HZ)
                    .unwrap_or(0) as u32;
                self.cpu.regs.write(2, ticks); // $v0
                log::trace!("  OSTimeGet() = {}", ticks);
            }
            "GetTickCount" => {
                let micros = self
                    .cycle_count
                    .saturating_mul(1_000_000)
                    .checked_div(CPU_CLOCK_HZ)
                    .unwrap_or(0);
                self.cpu.regs.write(2, micros as u32); // $v0
                log::trace!("  GetTickCount() = {}", micros as u32);
            }
            "delay_ms" | "mdelay" => {
                let ms = self.cpu.regs.read(4); // $a0
                let delay_cycles = (ms as u64).saturating_mul(CPU_CLOCK_HZ) / 1_000;
                self.delay_active_until(self.cycle_count.saturating_add(delay_cycles));
                log::trace!("  delay_ms({})", ms);
            }
            "StartSwTimer" => {
                self.cpu.regs.write(2, 0); // $v0 = 0
                log::trace!("  StartSwTimer() = 0");
            }
            "OSTimeDly" => {
                let ticks = self.cpu.regs.read(4); // $a0
                let delay_cycles =
                    (ticks as u64).saturating_mul(CPU_CLOCK_HZ) / OS_TICKS_PER_SECOND;
                self.delay_active_until(self.cycle_count.saturating_add(delay_cycles));
                log::trace!("  OSTimeDly({})", ticks);
            }
            "udelay" => {
                let us = self.cpu.regs.read(4); // $a0
                let delay_cycles = (us as u64).saturating_mul(CPU_CLOCK_HZ) / 1_000_000;
                self.delay_active_until(self.cycle_count.saturating_add(delay_cycles));
                log::trace!("  udelay({})", us);
            }
            "_sys_judge_event" | "sys_judge_event" => {
                let pending = u32::from(self.input.take_pending_event());
                self.cpu.regs.write(2, pending);
                log::trace!("  sys_judge_event() = {}", pending);
            }

            // Resource manager
            "get_dl_handle" => {
                self.cpu.regs.write(2, u32::from(self.app.is_some()));
            }
            "dl_res_open" => {
                let name = self.resource_name_from_args(&[
                    self.cpu.regs.read(6),
                    self.cpu.regs.read(5),
                    self.cpu.regs.read(4),
                ]);
                let handle = name
                    .as_deref()
                    .map(|name| self.open_resource_file(name))
                    .unwrap_or(0);
                self.cpu.regs.write(2, handle);
            }
            "dl_res_get_size" => {
                let handle = self.cpu.regs.read(4);
                let size = self
                    .open_files
                    .get(&handle)
                    .map(|file| file.data.len() as u32)
                    .unwrap_or(0);
                self.cpu.regs.write(2, size);
            }
            "dl_res_get_data" => {
                let handle = self.cpu.regs.read(4);
                let buffer = self.cpu.regs.read(5);
                let buffer_len = self.cpu.regs.read(6);
                let read_len = self.cpu.regs.read(7);
                let ret = self.read_resource_data(handle, buffer, buffer_len, read_len)?;
                self.cpu.regs.write(2, ret);
            }
            "dl_res_close" => {
                let handle = self.cpu.regs.read(4);
                self.open_files.remove(&handle);
                self.cpu.regs.write(2, 0);
            }
            // Resource-backed File I/O
            "fopen" | "fsys_fopen" => {
                let name = self.read_guest_c_string(self.cpu.regs.read(4));
                let mode = self.read_guest_c_string(self.cpu.regs.read(5));
                let operation = mode.as_bytes().first().copied().unwrap_or(b'r');
                let handle = match operation {
                    b'w' => self.open_memory_file(),
                    b'a' => {
                        let handle = match self.open_resource_file(&name) {
                            0 => self.open_host_file(&name),
                            handle => handle,
                        };
                        let handle = if handle == 0 {
                            self.open_memory_file()
                        } else {
                            handle
                        };
                        if let Some(file) = self.open_files.get_mut(&handle) {
                            file.position = file.data.len();
                        }
                        handle
                    }
                    _ => match self.open_resource_file(&name) {
                        0 => self.open_host_file(&name),
                        handle => handle,
                    },
                };
                self.cpu.regs.write(2, handle);
                log::trace!("  {}({}, {}) = {}", func_name, name, mode, handle);
            }
            "fsys_fopenW" => {
                let name = self.read_guest_w_string(self.cpu.regs.read(4));
                let mode = self.read_guest_w_string(self.cpu.regs.read(5));
                log::trace!("  fsys_fopenW({}, {})", name, mode);
                let handle = match self.open_resource_file(&name) {
                    0 => self.open_host_file(&name),
                    handle => handle,
                };
                self.cpu.regs.write(2, handle);
            }
            "fclose" | "fsys_fclose" | "fsys_fcloseW" => {
                let handle = self.cpu.regs.read(4);
                self.open_files.remove(&handle);
                self.cpu.regs.write(2, 0);
            }
            "fread" | "fsys_fread" => {
                let dest = self.cpu.regs.read(4);
                let size = self.cpu.regs.read(5);
                let count = self.cpu.regs.read(6);
                let handle = self.cpu.regs.read(7);
                let read = self.read_file(dest, size, count, handle)?;
                self.cpu.regs.write(2, read);
            }
            "fseek" | "fsys_fseek" => {
                let handle = self.cpu.regs.read(4);
                let offset = self.cpu.regs.read(5) as i32;
                let origin = self.cpu.regs.read(6);
                let ret = self.seek_file(handle, offset, origin);
                self.cpu.regs.write(2, ret);
            }
            "ftell" | "fsys_ftell" => {
                let handle = self.cpu.regs.read(4);
                let pos = self
                    .open_files
                    .get(&handle)
                    .map(|file| file.position as u32)
                    .unwrap_or(u32::MAX);
                self.cpu.regs.write(2, pos);
            }
            "feof" | "fsys_feof" => {
                let handle = self.cpu.regs.read(4);
                let eof = self
                    .open_files
                    .get(&handle)
                    .map(|file| u32::from(file.position >= file.data.len()))
                    .unwrap_or(1);
                self.cpu.regs.write(2, eof);
            }
            "fwrite" | "fsys_fwrite" => {
                let src = self.cpu.regs.read(4);
                let size = self.cpu.regs.read(5);
                let count = self.cpu.regs.read(6);
                let handle = self.cpu.regs.read(7);
                let written = self.write_file(src, size, count, handle)?;
                self.cpu.regs.write(2, written);
                log::trace!("  {}() = {}", func_name, written);
            }
            // System (stubs)
            "vxGoHome" | "abort" | "TaskMediaFunStop" => {
                self.cpu.stop();
                log::trace!("  {} -> stopping", func_name);
            }
            "sprintf" => {
                let destination = self.cpu.regs.read(4);
                let format = self.read_guest_c_string(self.cpu.regs.read(5));
                let rendered = self.format_guest_printf(&format)?;
                let mut bytes = rendered.as_bytes().to_vec();
                bytes.push(0);
                self.memory.load_data(destination, &bytes)?;
                self.cpu.regs.write(2, rendered.len() as u32);
                log::trace!("  sprintf({}) = {}", format, rendered.len());
            }
            "printf" | "fprintf" => {
                self.cpu.regs.write(2, 0);
                log::trace!("  {}() = 0 (stub)", func_name);
            }

            // Cache ops (no-op)
            "__icache_invalidate_all" | "__dcache_writeback_all" => {
                log::trace!("  {} (no-op)", func_name);
            }

            // Other stubs
            _ => {
                self.cpu.regs.write(2, 0); // Return 0 for unknown functions
                log::trace!("  {}() = 0 (unimplemented stub)", func_name);
            }
        }

        // Return to caller: jump to $ra
        self.cpu.regs.pc = ra;
        self.cpu.regs.gpr[0] = 0; // R0 is always zero

        Ok(())
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

impl Default for Emulator {
    fn default() -> Self {
        Self {
            cpu: Cpu::new(0x8000_0000),
            memory: Memory::new(),
            video: Video::new(),
            input: Input::new(),
            audio: Audio::new(),
            sdk: SdkHle::new(),
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
            hook_filter: vec![0; HOOK_FILTER_WORDS].into_boxed_slice(),
            open_files: HashMap::new(),
            next_file_handle: 1,
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

    #[test]
    fn test_emulator_creation() {
        let emu = Emulator::default();
        assert_eq!(emu.frame_count(), 0);
        assert!(!emu.is_running());
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

        emu.handle_sdk_call(0, "OSTimeGet").unwrap();
        assert_eq!(emu.cpu.regs.read(2), 1);

        emu.handle_sdk_call(0, "GetTickCount").unwrap();
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

        emu.handle_sdk_call(0, "sprintf").unwrap();

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

        emu.handle_sdk_call(0, "sprintf").unwrap();

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

        emu.handle_sdk_call(0, "__to_locale_ansi").unwrap();

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
        emu.handle_sdk_call(0, "__to_locale_ansi").unwrap();

        assert_eq!(emu.cpu.regs.read(2), output);
        assert_eq!(emu.read_guest_c_string(output), "Ali?.app");
    }

    #[test]
    fn test_get_system_model_writes_a320_as_utf16() {
        let mut emu = Emulator::default();
        let ptr = 0x100;
        emu.cpu.regs.write(4, ptr);
        emu.cpu.regs.write(31, 0x1234);

        emu.handle_sdk_call(0, "cmGetSysModel").unwrap();

        assert_eq!(emu.cpu.regs.read(2), 0);
        assert_eq!(emu.cpu.regs.pc, 0x1234);
        assert_eq!(emu.read_guest_w_string(ptr), "A320");
    }

    #[test]
    fn test_u8_to_u32_reads_little_endian_value() {
        let mut emu = Emulator::default();
        let ptr = 0x100;
        emu.memory
            .load_data(ptr, &[0x18, 0xC2, 0x01, 0x00])
            .unwrap();
        emu.cpu.regs.write(4, ptr);

        emu.handle_sdk_call(0, "U8TOU32").unwrap();

        assert_eq!(emu.cpu.regs.read(2), 0x0001_C218);
    }

    #[test]
    fn test_lcd_size_matches_a320_display() {
        let mut emu = Emulator::default();

        emu.handle_sdk_call(0, "LCD_GetXSize").unwrap();
        assert_eq!(emu.cpu.regs.read(2), crate::video::SCREEN_WIDTH);

        emu.handle_sdk_call(0, "LCD_GetYSize").unwrap();
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
        emu.handle_sdk_call(0, "waveout_open").unwrap();

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
        emu.handle_sdk_call(0, "waveout_write").unwrap();

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

        emu.handle_sdk_call(hook_address, "waveout_write").unwrap();

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

        emu.handle_sdk_call(hook_address, "waveout_write").unwrap();

        assert_eq!(emu.main_wait, None);
        assert_eq!(emu.cpu.regs.pc, return_address);
        assert_eq!(emu.cpu.regs.read(2), 1);
    }

    #[test]
    fn test_writable_file_is_buffered_in_memory() {
        let mut emu = Emulator::default();
        emu.memory.load_data(0x100, b"test.log\0").unwrap();
        emu.memory.load_data(0x120, b"w\0").unwrap();
        emu.memory.load_data(0x140, b"abcdef").unwrap();
        emu.cpu.regs.write(4, 0x100);
        emu.cpu.regs.write(5, 0x120);

        emu.handle_sdk_call(0, "fsys_fopen").unwrap();
        let handle = emu.cpu.regs.read(2);
        assert_ne!(handle, 0);

        emu.cpu.regs.write(4, 0x140);
        emu.cpu.regs.write(5, 2);
        emu.cpu.regs.write(6, 3);
        emu.cpu.regs.write(7, handle);
        emu.handle_sdk_call(0, "fsys_fwrite").unwrap();

        assert_eq!(emu.cpu.regs.read(2), 3);
        assert_eq!(emu.open_files[&handle].data, b"abcdef");
    }
}
