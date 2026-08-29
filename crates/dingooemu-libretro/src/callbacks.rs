use std::ffi::c_void;
use std::sync::RwLock;

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

static CALLBACKS: RwLock<Callbacks> = RwLock::new(Callbacks::new());

pub fn set_environment(callback: RetroEnvironmentCallback) {
    CALLBACKS.write().unwrap().environment = callback;
}

pub fn set_video_refresh(callback: RetroVideoRefreshCallback) {
    CALLBACKS.write().unwrap().video_refresh = callback;
}

pub fn set_audio_sample(callback: RetroAudioSampleCallback) {
    CALLBACKS.write().unwrap().audio_sample = callback;
}

pub fn set_audio_sample_batch(callback: RetroAudioSampleBatchCallback) {
    CALLBACKS.write().unwrap().audio_sample_batch = callback;
}

pub fn set_input_poll(callback: RetroInputPollCallback) {
    CALLBACKS.write().unwrap().input_poll = callback;
}

pub fn set_input_state(callback: RetroInputStateCallback) {
    CALLBACKS.write().unwrap().input_state = callback;
}

pub fn initialize_log_interface() {
    CALLBACKS.write().unwrap().log = None;
    let mut interface = RetroLogCallback { log: None };
    if environment(
        RETRO_ENVIRONMENT_GET_LOG_INTERFACE,
        &mut interface as *mut RetroLogCallback as *mut c_void,
    ) {
        CALLBACKS.write().unwrap().log = interface.log;
    }
}

pub fn environment(command: u32, data: *mut c_void) -> bool {
    let callback = CALLBACKS.read().unwrap().environment;
    unsafe { callback.is_some_and(|callback| callback(command, data)) }
}

pub fn video_refresh(data: *const c_void, width: u32, height: u32, pitch: usize) {
    let callback = CALLBACKS.read().unwrap().video_refresh;
    if let Some(callback) = callback {
        unsafe { callback(data, width, height, pitch) };
    }
}

pub fn audio_sample(left: i16, right: i16) {
    let callback = CALLBACKS.read().unwrap().audio_sample;
    if let Some(callback) = callback {
        unsafe { callback(left, right) };
    }
}

pub fn audio_sample_batch(data: *const i16, frames: usize) -> Option<usize> {
    let callback = CALLBACKS.read().unwrap().audio_sample_batch;
    unsafe { callback.map(|callback| callback(data, frames)) }
}

pub fn input_poll() {
    let callback = CALLBACKS.read().unwrap().input_poll;
    if let Some(callback) = callback {
        unsafe { callback() };
    }
}

pub fn input_state(port: u32, device: u32, index: u32, id: u32) -> i16 {
    let callback = CALLBACKS.read().unwrap().input_state;
    unsafe { callback.map_or(0, |callback| callback(port, device, index, id)) }
}

pub fn log(level: u32, message: *const std::os::raw::c_char) {
    let callback = CALLBACKS.read().unwrap().log;
    if let Some(callback) = callback {
        unsafe { callback(level, message) };
    }
}
