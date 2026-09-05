//! dingooemu-core: Platform-independent Dingoo A320 and Gemei A330 emulator engine
//!
//! This crate contains all emulation logic with no dependency on any windowing
//! or audio output device. Both front-ends (standalone and libretro) link
//! against this crate.

pub mod a320;
pub mod a330;
pub mod audio;
pub mod cheats;
mod common;
pub mod content;
pub mod error;
pub mod input;
pub mod package;
mod runtime;
mod save_state;
pub mod video;

// Re-export main types for convenience
pub use a320::JitDiagnostics;
pub use common::execution::UnknownInstructionPolicy;
pub use common::hle::{UnknownHleCall, UnknownHlePolicy};
pub use content::{ArmProfile, ContentFormat, GuestArchitecture, TargetDevice};
pub use error::{Result, SimulatorError};
pub use runtime::Emulator;
