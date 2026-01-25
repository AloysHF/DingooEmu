use crate::app_loader::AppFile;
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
}

impl Emulator {
    /// Create a new emulator from an .app file path
    pub fn from_path<P: AsRef<Path>>(path: P) -> Result<Self> {
        let app = AppFile::from_path(path)?;
        Self::from_app(app)
    }

    /// Create a new emulator from a parsed AppFile
    pub fn from_app(app: AppFile) -> Result<Self> {
        let mut memory = Memory::new();

        // Load executable into memory
        memory.load_data(app.load_base, &app.executable)?;

        let cpu = Cpu::new(app.entry_point);
        let video = Video::new(0x0300_0000); // Default framebuffer address
        let input = Input::new();
        let sdk = SdkHle::new();

        Ok(Self {
            cpu,
            memory,
            video,
            input,
            sdk,
            frame_count: 0,
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
        // Execute instructions for one frame (~336000 cycles at 336MHz / 60fps)
        // TODO: Make this configurable
        let cycles_per_frame = 336_000 / 60;

        for _ in 0..cycles_per_frame {
            if !self.cpu.is_running() {
                break;
            }
            self.cpu.step(&mut self.memory)?;
        }

        self.video.advance_frame();
        self.frame_count += 1;

        Ok(())
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_emulator_creation() {
        // This test would need an actual .app file
        // For now, just test that the struct can be created
        let memory = Memory::new();
        let cpu = Cpu::new(0x8000_0000);
        let video = Video::new(0x0300_0000);
        let input = Input::new();
        let sdk = SdkHle::new();

        let emu = Emulator {
            cpu,
            memory,
            video,
            input,
            sdk,
            frame_count: 0,
        };

        assert_eq!(emu.frame_count(), 0);
        assert!(!emu.is_running());
    }
}
