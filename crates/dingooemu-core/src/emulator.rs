use crate::app_loader::AppImage;
use crate::cpu::Cpu;
use crate::error::Result;
use crate::input::Input;
use crate::memory::Memory;
use crate::sdk_hle::SdkHle;
use crate::video::Video;
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

        // Load executable into memory at the load base address
        let load_base = app.load_base();
        let executable = app.executable().to_vec();
        memory.load_data(load_base, &executable)?;

        let mut cpu = Cpu::new(app.entry_point());

        // Initialize stack pointer to a reasonable value in RAM
        // Stack grows downward from top of RAM (32MB)
        cpu.regs.write(29, 0x01FF_FFF0); // $sp = top of RAM - 16

        // Initialize global pointer if needed
        // cpu.regs.write(28, 0x0100_0000); // $gp

        // Try to find framebuffer address from imports
        let framebuffer_addr = find_framebuffer_addr(&app).unwrap_or(0x0300_0000);
        let video = Video::new(framebuffer_addr);

        let input = Input::new();
        let sdk = SdkHle::new();

        log::info!(
            "Emulator initialized: entry={:#010x}, base={:#010x}, framebuffer={:#010x}",
            app.entry_point(),
            load_base,
            framebuffer_addr
        );

        Ok(Self {
            cpu,
            memory,
            video,
            input,
            sdk,
            frame_count: 0,
            app: Some(app),
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
            self.cpu.step(&mut self.memory)?;
        }

        // Sync framebuffer from guest memory to video subsystem
        self.sync_framebuffer();

        self.video.advance_frame();
        self.frame_count += 1;

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
