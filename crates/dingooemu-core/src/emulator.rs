use crate::app_loader::AppImage;
use crate::cpu::Cpu;
use crate::error::Result;
use crate::input::Input;
use crate::memory::Memory;
use crate::sdk_hle::SdkHle;
use crate::video::Video;
use std::collections::HashMap;
use std::path::Path;

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
    /// SDK HLE bridge
    pub sdk: SdkHle,
    /// Frame count
    frame_count: u64,
    /// Parsed app image (for resource access)
    app: Option<AppImage>,
    /// Import address to function name mapping
    import_addrs: HashMap<u32, String>,
}

impl Emulator {
    /// Create a new emulator from an .app file path
    pub fn from_path<P: AsRef<Path>>(path: P) -> Result<Self> {
        let app = AppImage::from_path(path)?;
        Self::from_app(app)
    }

    /// Create a new emulator from a parsed AppImage
    pub fn from_app(app: AppImage) -> Result<Self> {
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

        let mut cpu = Cpu::new(app.entry_point());

        // Initialize stack pointer to a reasonable value in RAM
        // Stack grows downward from top of RAM (32MB)
        cpu.regs.write(29, 0x01FF_FFF0); // $sp = top of RAM - 16

        // Try to find framebuffer address from imports
        let framebuffer_addr = find_framebuffer_addr(&app).unwrap_or(0x0300_0000);
        let video = Video::new(framebuffer_addr);

        let input = Input::new();
        let sdk = SdkHle::new();

        // Build import address map for SDK hooking
        // Include both KSEG0 and physical addresses
        let mut import_addrs = HashMap::new();
        for import in &app.imports {
            // KSEG0 address
            import_addrs.insert(import.address, import.name.clone());
            // Physical address
            let phys = import.address & 0x1FFF_FFFF;
            if phys != import.address {
                import_addrs.insert(phys, import.name.clone());
            }
        }

        log::info!(
            "Emulator initialized: entry={:#010x}, base={:#010x}, physical={:#010x}, framebuffer={:#010x}, imports={}",
            app.entry_point(),
            load_base,
            physical_addr,
            framebuffer_addr,
            import_addrs.len()
        );

        Ok(Self {
            cpu,
            memory,
            video,
            input,
            sdk,
            frame_count: 0,
            app: Some(app),
            import_addrs,
        })
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

    /// Run one frame of emulation
    pub fn tick(&mut self) -> Result<()> {
        // Execute instructions for one frame
        // Dingoo A320 runs at 336 MHz, 60 fps = 5,600,000 cycles per frame
        let cycles_per_frame = 5_600_000;

        for _ in 0..cycles_per_frame {
            if !self.cpu.is_running() {
                break;
            }

            // Check if PC is at an import table address (SDK call)
            let pc = self.cpu.regs.pc;
            if let Some(func_name) = self.import_addrs.get(&pc).cloned() {
                // Handle SDK call directly
                self.handle_sdk_call(pc, &func_name)?;
            } else {
                // Check if this is a JAL/J instruction to an import address
                let instr = self.memory.read_u32(pc).unwrap_or(0);
                let opcode = (instr >> 26) & 0x3F;

                if opcode == 0x03 || opcode == 0x02 {
                    // JAL or J instruction - check if target is an import
                    let target = if opcode == 0x03 {
                        // JAL: target = (PC & 0xF0000000) | (instr_index << 2)
                        let instr_index = instr & 0x03FF_FFFF;
                        (pc & 0xF000_0000) | (instr_index << 2)
                    } else {
                        // J: same calculation
                        let instr_index = instr & 0x03FF_FFFF;
                        (pc & 0xF000_0000) | (instr_index << 2)
                    };

                    if let Some(func_name) = self.import_addrs.get(&target).cloned() {
                        // Execute the delay slot first (the instruction after JAL/J)
                        self.cpu.step(&mut self.memory)?;

                        // Handle SDK call (this will set $v0 and return to $ra)
                        self.handle_sdk_call(target, &func_name)?;
                    } else {
                        // Normal instruction execution
                        self.cpu.step(&mut self.memory)?;
                    }
                } else {
                    // Normal instruction execution
                    self.cpu.step(&mut self.memory)?;
                }
            }
        }

        // Sync framebuffer from guest memory to video subsystem
        self.sync_framebuffer();

        self.video.advance_frame();
        self.frame_count += 1;

        Ok(())
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

            // Graphics/LCD
            "_lcd_get_frame" | "lcd_get_frame" | "lcd_get_cframe" => {
                let kseg0_addr = self.video.framebuffer_addr() | 0x8000_0000;
                self.cpu.regs.write(2, kseg0_addr); // $v0
                log::trace!("  lcd_get_frame() = {:#010x}", kseg0_addr);
            }
            "_lcd_set_frame" | "lcd_set_frame" | "ap_lcd_set_frame" => {
                let addr = self.cpu.regs.read(4); // $a0
                let physical_addr = if (0x8000_0000..0xA000_0000).contains(&addr) {
                    addr & 0x1FFF_FFFF
                } else {
                    addr
                };
                self.video.set_framebuffer_addr(physical_addr);
                log::info!(
                    "  lcd_set_frame({:#010x}) -> physical: {:#010x}",
                    addr,
                    physical_addr
                );
            }
            "lcd_flip" => {
                log::trace!("  lcd_flip()");
            }
            "LcdGetDisMode" => {
                self.cpu.regs.write(2, 0); // $v0 = 0
                log::trace!("  LcdGetDisMode() = 0");
            }

            // Input
            "_kbd_get_status" | "kbd_get_status" => {
                let status_ptr = self.cpu.regs.read(4); // $a0
                let buttons = self.input.buttons();
                let _ = self.memory.write_u32(status_ptr, buttons);
                log::trace!("  kbd_get_status({:#010x}) = {:#010x}", status_ptr, buttons);
            }
            "_kbd_get_key" | "kbd_get_key" => {
                // Convert bitmask to key code
                let buttons = self.input.buttons();
                let key = if buttons & 0x0001 != 0 {
                    1
                } else if buttons & 0x0002 != 0 {
                    2
                } else if buttons & 0x0004 != 0 {
                    3
                } else if buttons & 0x0008 != 0 {
                    4
                } else if buttons & 0x0010 != 0 {
                    5
                } else if buttons & 0x0020 != 0 {
                    6
                } else if buttons & 0x0040 != 0 {
                    7
                } else if buttons & 0x0080 != 0 {
                    8
                } else if buttons & 0x0100 != 0 {
                    9
                } else if buttons & 0x0200 != 0 {
                    10
                } else if buttons & 0x0400 != 0 {
                    11
                } else if buttons & 0x0800 != 0 {
                    12
                } else {
                    0
                };
                self.cpu.regs.write(2, key); // $v0
                log::trace!("  kbd_get_key() = {}", key);
            }

            // Timer
            "GetTickCount" | "OSTimeGet" => {
                let ticks = (self.frame_count * 16) as u32; // ~60fps -> ~16ms per frame
                self.cpu.regs.write(2, ticks); // $v0
                log::trace!("  GetTickCount() = {}", ticks);
            }
            "delay_ms" | "mdelay" => {
                let ms = self.cpu.regs.read(4); // $a0
                log::trace!("  delay_ms({})", ms);
            }

            // File I/O (stubs)
            "fopen" | "fsys_fopen" => {
                self.cpu.regs.write(2, 0); // $v0 = NULL
                log::trace!("  fopen() = NULL (stub)");
            }
            "fclose" | "fsys_fclose" => {
                self.cpu.regs.write(2, 0); // $v0 = 0
                log::trace!("  fclose() = 0 (stub)");
            }
            "fread" | "fsys_fread" => {
                self.cpu.regs.write(2, 0); // $v0 = 0
                log::trace!("  fread() = 0 (stub)");
            }
            "fwrite" | "fsys_fwrite" => {
                self.cpu.regs.write(2, 0); // $v0 = 0
                log::trace!("  fwrite() = 0 (stub)");
            }

            // System (stubs)
            "vxGoHome" | "abort" | "TaskMediaFunStop" => {
                self.cpu.stop();
                log::trace!("  {} -> stopping", func_name);
            }
            "printf" | "sprintf" | "fprintf" => {
                // Stubs - just return 0
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
    fn sync_framebuffer(&mut self) {
        let addr = self.video.framebuffer_addr();
        let size = crate::video::FRAMEBUFFER_SIZE;

        // Try to read framebuffer from guest memory
        let mut fb_data = vec![0u8; size];
        let mut all_ok = true;

        for (i, byte) in fb_data.iter_mut().enumerate() {
            match self.memory.read_u8(addr.wrapping_add(i as u32)) {
                Ok(b) => *byte = b,
                Err(_) => {
                    all_ok = false;
                    break;
                }
            }
        }

        if all_ok {
            // Copy to video subsystem
            let dst = self.video.framebuffer_mut();
            dst.copy_from_slice(&fb_data);
        }
    }

    /// Set the button state
    pub fn set_buttons(&mut self, buttons: u32) {
        self.input.set_buttons(buttons);
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

/// Try to find framebuffer address from app imports
fn find_framebuffer_addr(app: &AppImage) -> Option<u32> {
    // Look for common framebuffer function names
    for import in &app.imports {
        let name = import.name.to_lowercase();
        if name.contains("lcd_get_frame") || name.contains("get_framebuffer") {
            return Some(import.address);
        }
    }
    None
}

impl Default for Emulator {
    fn default() -> Self {
        Self {
            cpu: Cpu::new(0x8000_0000),
            memory: Memory::new(),
            video: Video::new(0x0300_0000),
            input: Input::new(),
            sdk: SdkHle::new(),
            frame_count: 0,
            app: None,
            import_addrs: HashMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_emulator_creation() {
        let emu = Emulator::default();
        assert_eq!(emu.frame_count(), 0);
        assert!(!emu.is_running());
    }
}
