//! dingooemu-core: Platform-independent Dingoo A320 emulator engine
//!
//! This crate contains all emulation logic with no dependency on any windowing
//! or audio output device. Both front-ends (standalone and libretro) link
//! against this crate.

pub mod app_loader;
pub mod audio;
pub mod cheats;
pub mod cpu;
pub mod emulator;
pub mod error;
pub mod input;
#[cfg(feature = "jit")]
mod jit;
pub mod memory;
mod save_state;
pub mod video;

// Re-export main types for convenience
pub use emulator::{Emulator, UnknownHleCall, UnknownHlePolicy};
pub use error::{Result, SimulatorError};
