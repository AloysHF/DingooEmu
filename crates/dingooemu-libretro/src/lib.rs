//! RetroArch libretro core for Dingoo A320 software.

#![allow(dead_code)]
#![allow(static_mut_refs)]
#![allow(clippy::not_unsafe_ptr_arg_deref)]

mod api;
mod callbacks;
mod constants;
mod logger;
mod types;

use dingooemu_core::Emulator;

static mut EMULATOR: Option<Emulator> = None;
