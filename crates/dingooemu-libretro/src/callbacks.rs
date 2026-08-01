use std::ffi::c_void;

use crate::constants::RETRO_ENVIRONMENT_GET_LOG_INTERFACE;
use crate::types::*;

struct Callbacks {
    environment: RetroEnvironmentCallback,
    video_refresh: RetroVideoRefreshCallback,
    audio_sample: RetroAudioSampleCallback,
    audio_sample_batch: RetroAudioSampleBatchCallback,
    input_poll: RetroInputPollCallback,
    input_state: RetroInputStateCallback,
    log: RetroLogPrintfCallback,
}

impl Callbacks {
    const fn new() -> Self {
        Self {
            environment: None,
            video_refresh: None,
            audio_sample: None,
            audio_sample_batch: None,
            input_poll: None,
            input_state: None,
            log: None,
        }
    }
}

static mut CALLBACKS: Callbacks = Callbacks::new();

pub fn set_environment(callback: RetroEnvironmentCallback) {
    unsafe { CALLBACKS.environment = callback };
}

pub fn set_video_refresh(callback: RetroVideoRefreshCallback) {
    unsafe { CALLBACKS.video_refresh = callback };
}

pub fn set_audio_sample(callback: RetroAudioSampleCallback) {
    unsafe { CALLBACKS.audio_sample = callback };
}

pub fn set_audio_sample_batch(callback: RetroAudioSampleBatchCallback) {
    unsafe { CALLBACKS.audio_sample_batch = callback };
}

pub fn set_input_poll(callback: RetroInputPollCallback) {
    unsafe { CALLBACKS.input_poll = callback };
}

pub fn set_input_state(callback: RetroInputStateCallback) {
    unsafe { CALLBACKS.input_state = callback };
}

pub fn initialize_log_interface() {
    unsafe { CALLBACKS.log = None };
    let mut interface = RetroLogCallback { log: None };
    if environment(
        RETRO_ENVIRONMENT_GET_LOG_INTERFACE,
        &mut interface as *mut RetroLogCallback as *mut c_void,
    ) {
        unsafe { CALLBACKS.log = interface.log };
    }
}

pub fn environment(command: u32, data: *mut c_void) -> bool {
    unsafe {
        CALLBACKS
            .environment
            .is_some_and(|callback| callback(command, data))
    }
}

pub fn video_refresh(data: *const c_void, width: u32, height: u32, pitch: usize) {
    unsafe {
        if let Some(callback) = CALLBACKS.video_refresh {
            callback(data, width, height, pitch);
        }
    }
}

pub fn audio_sample(left: i16, right: i16) {
    unsafe {
        if let Some(callback) = CALLBACKS.audio_sample {
            callback(left, right);
        }
    }
}

pub fn audio_sample_batch(data: *const i16, frames: usize) -> Option<usize> {
    unsafe {
        CALLBACKS
            .audio_sample_batch
            .map(|callback| callback(data, frames))
    }
}

pub fn input_poll() {
    unsafe {
        if let Some(callback) = CALLBACKS.input_poll {
            callback();
        }
    }
}

pub fn input_state(port: u32, device: u32, index: u32, id: u32) -> i16 {
    unsafe {
        CALLBACKS
            .input_state
            .map_or(0, |callback| callback(port, device, index, id))
    }
}

pub fn log(level: u32, message: *const std::os::raw::c_char) {
    unsafe {
        if let Some(callback) = CALLBACKS.log {
            callback(level, message);
        }
    }
}
