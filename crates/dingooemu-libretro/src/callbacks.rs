use crate::constants::*;
use crate::CALLBACKS;

/// Log a message through the libretro log interface
pub fn retro_log(level: u32, msg: &str) {
    unsafe {
        if let Some(callbacks) = CALLBACKS.as_ref() {
            if let Some(log_fn) = callbacks.log {
                let c_msg = std::ffi::CString::new(msg).unwrap();
                log_fn(level, c_msg.as_ptr());
            }
        }
    }
}

/// Get a libretro environment variable
pub fn environment_get(id: u32, data: *mut std::ffi::c_void) -> bool {
    unsafe {
        if let Some(callbacks) = CALLBACKS.as_ref() {
            if let Some(env_fn) = callbacks.environment {
                return env_fn(id, data);
            }
        }
        false
    }
}

/// Set pixel format
pub fn environment_set_pixel_format(format: u32) -> bool {
    environment_get(
        RETRO_ENVIRONMENT_SET_PIXEL_FORMAT,
        &format as *const _ as *mut _,
    )
}

/// Set input descriptors
pub fn environment_set_input_descriptors(
    descriptors: *const crate::types::RetroInputDescriptor,
) -> bool {
    environment_get(
        RETRO_ENVIRONMENT_SET_INPUT_DESCRIPTORS,
        descriptors as *mut _,
    )
}
