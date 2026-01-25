use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_uint, c_void};

use crate::constants::*;
use crate::types::*;
use crate::{CALLBACKS, EMULATOR};

use dingooemu_core::Emulator;

// ============================================================================
// Libretro API Implementation
// ============================================================================

/// Set the environment callback
#[no_mangle]
pub extern "C" fn retro_set_environment(
    cb: Option<unsafe extern "C" fn(u32, *const c_void) -> bool>,
) {
    unsafe {
        if let Some(callbacks) = CALLBACKS.as_mut() {
            callbacks.environment = cb;
        }
    }
}

/// Set the video refresh callback
#[no_mangle]
pub extern "C" fn retro_set_video_refresh(
    cb: Option<unsafe extern "C" fn(*const u32, u32, u32, usize)>,
) {
    unsafe {
        if let Some(callbacks) = CALLBACKS.as_mut() {
            callbacks.video_refresh = cb;
        }
    }
}

/// Set the audio sample callback
#[no_mangle]
pub extern "C" fn retro_set_audio_sample(cb: Option<unsafe extern "C" fn(i16, i16)>) {
    unsafe {
        if let Some(callbacks) = CALLBACKS.as_mut() {
            callbacks.audio_sample = cb;
        }
    }
}

/// Set the audio sample batch callback
#[no_mangle]
pub extern "C" fn retro_set_audio_sample_batch(
    cb: Option<unsafe extern "C" fn(*const i16, usize) -> usize>,
) {
    unsafe {
        if let Some(callbacks) = CALLBACKS.as_mut() {
            callbacks.audio_sample_batch = cb;
        }
    }
}

/// Set the input poll callback
#[no_mangle]
pub extern "C" fn retro_set_input_poll(cb: Option<unsafe extern "C" fn()>) {
    unsafe {
        if let Some(callbacks) = CALLBACKS.as_mut() {
            callbacks.input_poll = cb;
        }
    }
}

/// Set the input state callback
#[no_mangle]
pub extern "C" fn retro_set_input_state(
    cb: Option<unsafe extern "C" fn(u32, u32, u32, u32) -> i16>,
) {
    unsafe {
        if let Some(callbacks) = CALLBACKS.as_mut() {
            callbacks.input_state = cb;
        }
    }
}

/// Initialize the libretro core
#[no_mangle]
pub extern "C" fn retro_init() {
    log::info!("retro_init");
    // TODO: Initialize the emulator
}

/// Deinitialize the libretro core
#[no_mangle]
pub extern "C" fn retro_deinit() {
    log::info!("retro_deinit");
    unsafe {
        EMULATOR = None;
    }
}

/// Get the libretro API version
#[no_mangle]
pub extern "C" fn retro_api_version() -> c_uint {
    RETRO_API_VERSION
}

/// Get system information
#[no_mangle]
pub extern "C" fn retro_get_system_info(info: *mut RetroSystemInfo) {
    unsafe {
        if let Some(info) = info.as_mut() {
            let name = CString::new("dingooemu").unwrap();
            let valid_exts = CString::new("app").unwrap();
            let version = CString::new("0.1.0").unwrap();

            info.library_name = name.into_raw();
            info.library_version = version.into_raw();
            info.valid_extensions = valid_exts.into_raw();
            info.block_extract = false;
            info.need_fullpath = true;
        }
    }
}

/// Get system A/V info
#[no_mangle]
pub extern "C" fn retro_get_system_av_info(info: *mut RetroSystemAvInfo) {
    unsafe {
        if let Some(info) = info.as_mut() {
            info.geometry = RetroGameGeometry {
                base_width: 320,
                base_height: 240,
                max_width: 320,
                max_height: 240,
                aspect_ratio: 320.0 / 240.0,
            };
            info.timing = RetroSystemTiming {
                fps: 60.0,
                sample_rate: 22050.0,
            };
        }
    }
}

/// Get the region (NTSC)
#[no_mangle]
pub extern "C" fn retro_get_region() -> c_uint {
    RETRO_REGION_NTSC
}

/// Load a game
#[no_mangle]
pub extern "C" fn retro_load_game(info: *const RetroGameInfo) -> bool {
    unsafe {
        if info.is_null() {
            return false;
        }

        let info = &*info;

        // Get the game path
        let path = if !info.path.is_null() {
            match CStr::from_ptr(info.path).to_str() {
                Ok(p) => p,
                Err(_) => return false,
            }
        } else {
            return false;
        };

        // Create the emulator
        match Emulator::from_path(path) {
            Ok(emu) => {
                EMULATOR = Some(emu);
                log::info!("Loaded game: {}", path);
                true
            }
            Err(e) => {
                log::error!("Failed to load game: {}", e);
                false
            }
        }
    }
}

/// Unload the game
#[no_mangle]
pub extern "C" fn retro_unload_game() -> bool {
    unsafe {
        EMULATOR = None;
        true
    }
}

/// Run one frame
#[no_mangle]
pub extern "C" fn retro_run() {
    unsafe {
        if let Some(emu) = EMULATOR.as_mut() {
            // Poll input
            // TODO: Implement input polling

            // Run one frame
            if let Err(e) = emu.tick() {
                log::error!("Tick error: {}", e);
            }

            // Submit framebuffer
            if let Some(cb) = CALLBACKS.as_ref().and_then(|c| c.video_refresh) {
                let buffer = emu.video.to_xrgb8888();
                cb(buffer.as_ptr(), 320, 240, 320 * std::mem::size_of::<u32>());
            }
        }
    }
}

/// Reset the emulator
#[no_mangle]
pub extern "C" fn retro_reset() {
    unsafe {
        if let Some(emu) = EMULATOR.as_mut() {
            emu.stop();
            emu.start();
        }
    }
}

/// Serialize state (save state)
#[no_mangle]
pub extern "C" fn retro_serialize_size() -> usize {
    // TODO: Implement save state
    0
}

/// Serialize state
#[no_mangle]
pub extern "C" fn retro_serialize(_data: *mut u8, _size: usize) -> bool {
    // TODO: Implement save state
    false
}

/// Unserialize state (load state)
#[no_mangle]
pub extern "C" fn retro_unserialize(_data: *const u8, _size: usize) -> bool {
    // TODO: Implement load state
    false
}

/// Set cheat
#[no_mangle]
pub extern "C" fn retro_cheat_set(_index: c_uint, _enabled: bool, _code: *const c_char) {
    // TODO: Implement cheats
}

/// Get memory data
#[no_mangle]
pub extern "C" fn retro_get_memory_data(_id: c_uint) -> *mut u8 {
    // TODO: Implement memory access
    std::ptr::null_mut()
}

/// Get memory size
#[no_mangle]
pub extern "C" fn retro_get_memory_size(_id: c_uint) -> usize {
    // TODO: Implement memory access
    0
}

/// Get controller info
#[no_mangle]
pub extern "C" fn retro_get_controller_info() -> *const RetroInputDescriptor {
    // TODO: Implement controller info
    std::ptr::null()
}

/// Get sensor info
#[no_mangle]
pub extern "C" fn retro_get_sensor_interface() -> *const RetroSensorInterface {
    std::ptr::null()
}

/// Get rumble interface
#[no_mangle]
pub extern "C" fn retro_get_rumble_interface() -> *const RetroRumbleInterface {
    std::ptr::null()
}

/// Get input device capabilities
#[no_mangle]
pub extern "C" fn retro_get_input_device_capabilities() -> u64 {
    // Keyboard + gamepad
    (1 << RETRO_DEVICE_KEYBOARD) | (1 << RETRO_DEVICE_JOYPAD)
}
