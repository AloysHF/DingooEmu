use std::os::raw::{c_char, c_int, c_uint};

/// System info structure
#[repr(C)]
pub struct RetroSystemInfo {
    pub library_name: *const c_char,
    pub library_version: *const c_char,
    pub valid_extensions: *const c_char,
    pub block_extract: bool,
    pub need_fullpath: bool,
}

/// System A/V info structure
#[repr(C)]
pub struct RetroSystemAvInfo {
    pub geometry: RetroGameGeometry,
    pub timing: RetroSystemTiming,
}

/// Game geometry
#[repr(C)]
pub struct RetroGameGeometry {
    pub base_width: c_uint,
    pub base_height: c_uint,
    pub max_width: c_uint,
    pub max_height: c_uint,
    pub aspect_ratio: f32,
}

/// System timing
#[repr(C)]
pub struct RetroSystemTiming {
    pub fps: f64,
    pub sample_rate: f64,
}

/// Game info structure
#[repr(C)]
pub struct RetroGameInfo {
    pub path: *const c_char,
    pub data: *const u8,
    pub size: usize,
    pub meta: *const c_char,
}

/// Input descriptor
#[repr(C)]
pub struct RetroInputDescriptor {
    pub port: c_uint,
    pub device: c_uint,
    pub index: c_uint,
    pub description: *const c_char,
}

/// Sensor interface
#[repr(C)]
pub struct RetroSensorInterface {
    pub api_version: c_int,
    pub set_sensor_state: Option<unsafe extern "C" fn(c_uint, c_int, c_uint) -> bool>,
    pub poll_sensor_state: Option<unsafe extern "C" fn(c_uint) -> bool>,
}

/// Rumble interface
#[repr(C)]
pub struct RetroRumbleInterface {
    pub set_rumble_state: Option<unsafe extern "C" fn(c_uint, c_uint, u16) -> bool>,
}
