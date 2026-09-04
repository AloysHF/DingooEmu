//! dingooemu-core: Platform-independent Dingoo A320 and Gemei A330 emulator engine
//!
//! This crate contains all emulation logic with no dependency on any windowing
//! or audio output device. Both front-ends (standalone and libretro) link
//! against this crate.

pub mod a330_memory;
mod a330_runtime;
pub mod app_loader;
pub mod arm_cpu;
pub mod audio;
pub mod cheats;
pub mod content;
pub mod cpu;
pub mod emulator;
pub mod error;
mod firmware_archive;
pub mod input;
#[cfg(feature = "jit")]
mod jit;
pub mod memory;
mod runtime;
mod save_state;
pub mod video;

// Re-export main types for convenience
pub use content::{ArmProfile, ContentFormat, GuestArchitecture};
pub use emulator::{JitDiagnostics, UnknownHleCall, UnknownHlePolicy};
pub use error::{Result, SimulatorError};
pub use runtime::Emulator;
