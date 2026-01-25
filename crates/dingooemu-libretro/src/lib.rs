//! dingooemu-libretro: RetroArch libretro core for Dingoo A320 emulator

#![allow(dead_code)]
#![allow(static_mut_refs)]

mod api;
mod callbacks;
mod constants;
mod types;

use dingooemu_core::Emulator;

/// Global emulator instance
static mut EMULATOR: Option<Emulator> = None;

/// Global callbacks
static mut CALLBACKS: Option<Callbacks> = None;

/// Libretro callback function pointers
#[derive(Default)]
pub struct Callbacks {
    pub environment: Option<unsafe extern "C" fn(u32, *const std::ffi::c_void) -> bool>,
    pub video_refresh: Option<unsafe extern "C" fn(*const u32, u32, u32, usize)>,
    pub audio_sample: Option<unsafe extern "C" fn(i16, i16)>,
    pub audio_sample_batch: Option<unsafe extern "C" fn(*const i16, usize) -> usize>,
    pub input_poll: Option<unsafe extern "C" fn()>,
    pub input_state: Option<unsafe extern "C" fn(u32, u32, u32, u32) -> i16>,
    pub log: Option<unsafe extern "C" fn(u32, *const std::os::raw::c_char)>,
}

/// Initialize the emulator
unsafe fn init_emulator() -> bool {
    EMULATOR = Some(Emulator::default());
    EMULATOR.is_some()
}

/// Get a reference to the emulator
unsafe fn emulator() -> Option<&'static mut Emulator> {
    EMULATOR.as_mut()
}
