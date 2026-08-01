use std::ffi::c_void;
use std::os::raw::{c_char, c_uint};

pub type RetroEnvironmentCallback = Option<unsafe extern "C" fn(c_uint, *mut c_void) -> bool>;
pub type RetroVideoRefreshCallback =
    Option<unsafe extern "C" fn(*const c_void, c_uint, c_uint, usize)>;
pub type RetroAudioSampleCallback = Option<unsafe extern "C" fn(i16, i16)>;
pub type RetroAudioSampleBatchCallback = Option<unsafe extern "C" fn(*const i16, usize) -> usize>;
pub type RetroInputPollCallback = Option<unsafe extern "C" fn()>;
pub type RetroInputStateCallback =
    Option<unsafe extern "C" fn(c_uint, c_uint, c_uint, c_uint) -> i16>;
pub type RetroLogPrintfCallback = Option<unsafe extern "C" fn(c_uint, *const c_char)>;

#[repr(C)]
pub struct RetroSystemInfo {
    pub library_name: *const c_char,
    pub library_version: *const c_char,
    pub valid_extensions: *const c_char,
    pub need_fullpath: bool,
    pub block_extract: bool,
}

#[repr(C)]
pub struct RetroSystemAvInfo {
    pub geometry: RetroGameGeometry,
    pub timing: RetroSystemTiming,
}

#[repr(C)]
pub struct RetroGameGeometry {
    pub base_width: c_uint,
    pub base_height: c_uint,
    pub max_width: c_uint,
    pub max_height: c_uint,
    pub aspect_ratio: f32,
}

#[repr(C)]
pub struct RetroSystemTiming {
    pub fps: f64,
    pub sample_rate: f64,
}

#[repr(C)]
pub struct RetroGameInfo {
    pub path: *const c_char,
    pub data: *const c_void,
    pub size: usize,
    pub meta: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct RetroInputDescriptor {
    pub port: c_uint,
    pub device: c_uint,
    pub index: c_uint,
    pub id: c_uint,
    pub description: *const c_char,
}

#[repr(C)]
pub struct RetroLogCallback {
    pub log: RetroLogPrintfCallback,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct RetroVariable {
    pub key: *const c_char,
    pub value: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct RetroMemoryDescriptor {
    pub flags: u64,
    pub ptr: *mut c_void,
    pub offset: usize,
    pub start: usize,
    pub select: usize,
    pub disconnect: usize,
    pub len: usize,
    pub addrspace: *const c_char,
}

#[repr(C)]
pub struct RetroMemoryMap {
    pub descriptors: *const RetroMemoryDescriptor,
    pub num_descriptors: c_uint,
}
