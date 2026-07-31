use std::ffi::{c_void, CStr};
use std::os::raw::{c_char, c_uint};
use std::ptr;

use dingooemu_core::input::{
    BUTTON_A, BUTTON_B, BUTTON_DOWN, BUTTON_L, BUTTON_LEFT, BUTTON_R, BUTTON_RIGHT, BUTTON_SELECT,
    BUTTON_START, BUTTON_UP, BUTTON_X, BUTTON_Y,
};
use dingooemu_core::video::{SCREEN_HEIGHT, SCREEN_WIDTH};
use dingooemu_core::Emulator;

use crate::callbacks;
use crate::constants::*;
use crate::types::*;
use crate::EMULATOR;

const PERFORMANCE_LEVEL: u32 = 4;
const AUDIO_SAMPLE_RATE: f64 = 22_050.0;
const FRAMES_PER_SECOND: f64 = 60.0;

#[no_mangle]
pub extern "C" fn retro_set_environment(callback: RetroEnvironmentCallback) {
    callbacks::set_environment(callback);
    set_core_options();
}

#[no_mangle]
pub extern "C" fn retro_set_video_refresh(callback: RetroVideoRefreshCallback) {
    callbacks::set_video_refresh(callback);
}

#[no_mangle]
pub extern "C" fn retro_set_audio_sample(callback: RetroAudioSampleCallback) {
    callbacks::set_audio_sample(callback);
}

#[no_mangle]
pub extern "C" fn retro_set_audio_sample_batch(callback: RetroAudioSampleBatchCallback) {
    callbacks::set_audio_sample_batch(callback);
}

#[no_mangle]
pub extern "C" fn retro_set_input_poll(callback: RetroInputPollCallback) {
    callbacks::set_input_poll(callback);
}

#[no_mangle]
pub extern "C" fn retro_set_input_state(callback: RetroInputStateCallback) {
    callbacks::set_input_state(callback);
}

#[no_mangle]
pub extern "C" fn retro_init() {
    callbacks::initialize_log_interface();
    crate::logger::initialize();
    log::info!("Libretro core initialized");
}

#[no_mangle]
pub extern "C" fn retro_deinit() {
    unsafe { EMULATOR = None };
    log::info!("Libretro core deinitialized");
}

#[no_mangle]
pub extern "C" fn retro_api_version() -> c_uint {
    RETRO_API_VERSION
}

#[no_mangle]
pub extern "C" fn retro_get_system_info(info: *mut RetroSystemInfo) {
    let Some(info) = (unsafe { info.as_mut() }) else {
        return;
    };

    info.library_name = c"DingooEmu".as_ptr();
    info.library_version = c"0.1.0".as_ptr();
    info.valid_extensions = c"app".as_ptr();
    info.need_fullpath = true;
    info.block_extract = false;
}

#[no_mangle]
pub extern "C" fn retro_get_system_av_info(info: *mut RetroSystemAvInfo) {
    let Some(info) = (unsafe { info.as_mut() }) else {
        return;
    };

    info.geometry = RetroGameGeometry {
        base_width: SCREEN_WIDTH,
        base_height: SCREEN_HEIGHT,
        max_width: SCREEN_WIDTH,
        max_height: SCREEN_HEIGHT,
        aspect_ratio: SCREEN_WIDTH as f32 / SCREEN_HEIGHT as f32,
    };
    info.timing = RetroSystemTiming {
        fps: FRAMES_PER_SECOND,
        sample_rate: AUDIO_SAMPLE_RATE,
    };
}

#[no_mangle]
pub extern "C" fn retro_set_controller_port_device(port: c_uint, device: c_uint) {
    if port == 0 && device != RETRO_DEVICE_NONE && device != RETRO_DEVICE_JOYPAD {
        log::warn!("Unsupported controller device {device} on port {port}");
    }
}

#[no_mangle]
pub extern "C" fn retro_get_region() -> c_uint {
    RETRO_REGION_NTSC
}

#[no_mangle]
pub extern "C" fn retro_load_game(info: *const RetroGameInfo) -> bool {
    let Some(info) = (unsafe { info.as_ref() }) else {
        return false;
    };
    if info.path.is_null() {
        return false;
    }

    let path = match unsafe { CStr::from_ptr(info.path) }.to_str() {
        Ok(path) => path,
        Err(error) => {
            log::error!("Content path is not valid UTF-8: {error}");
            return false;
        }
    };
    unsafe { EMULATOR = None };

    if !set_pixel_format() {
        log::error!("Frontend rejected the required XRGB8888 pixel format");
        return false;
    }
    register_input_descriptors();
    set_performance_level();

    match Emulator::from_path(path) {
        Ok(mut emulator) => {
            apply_core_options(&mut emulator);
            emulator.start();
            unsafe { EMULATOR = Some(emulator) };
            log::info!("Loaded content: {path}");
            true
        }
        Err(error) => {
            log::error!("Failed to load content: {error}");
            false
        }
    }
}

#[no_mangle]
pub extern "C" fn retro_load_game_special(
    _game_type: c_uint,
    _info: *const RetroGameInfo,
    _num_info: usize,
) -> bool {
    false
}

#[no_mangle]
pub extern "C" fn retro_unload_game() {
    unsafe { EMULATOR = None };
}

#[no_mangle]
pub extern "C" fn retro_run() {
    if core_options_changed() {
        if let Some(emulator) = unsafe { EMULATOR.as_mut() } {
            apply_core_options(emulator);
        }
    }
    let Some(emulator) = (unsafe { EMULATOR.as_mut() }) else {
        return;
    };

    callbacks::input_poll();
    let buttons =
        query_joypad_buttons(|id| callbacks::input_state(0, RETRO_DEVICE_JOYPAD, 0, id) != 0);
    emulator.set_buttons(buttons);

    if let Err(error) = emulator.tick() {
        log::error!("Frame execution failed: {error}");
    }

    let frame = emulator.video.to_xrgb8888();
    callbacks::video_refresh(
        frame.as_ptr().cast(),
        SCREEN_WIDTH,
        SCREEN_HEIGHT,
        SCREEN_WIDTH as usize * std::mem::size_of::<u32>(),
    );

    let samples = emulator.take_audio_samples();
    if callbacks::audio_sample_batch(samples.as_ptr(), samples.len() / 2).is_none() {
        for sample in samples.chunks_exact(2) {
            callbacks::audio_sample(sample[0], sample[1]);
        }
    }
}

#[no_mangle]
pub extern "C" fn retro_reset() {
    let Some(emulator) = (unsafe { EMULATOR.as_mut() }) else {
        return;
    };
    if let Err(error) = emulator.reset() {
        log::error!("Reset failed: {error}");
    } else {
        apply_core_options(emulator);
    }
}

#[no_mangle]
pub extern "C" fn retro_serialize_size() -> usize {
    0
}

#[no_mangle]
pub extern "C" fn retro_serialize(_data: *mut c_void, _size: usize) -> bool {
    false
}

#[no_mangle]
pub extern "C" fn retro_unserialize(_data: *const c_void, _size: usize) -> bool {
    false
}

#[no_mangle]
pub extern "C" fn retro_cheat_reset() {}

#[no_mangle]
pub extern "C" fn retro_cheat_set(_index: c_uint, _enabled: bool, _code: *const c_char) {}

#[no_mangle]
pub extern "C" fn retro_get_memory_data(_id: c_uint) -> *mut c_void {
    ptr::null_mut()
}

#[no_mangle]
pub extern "C" fn retro_get_memory_size(_id: c_uint) -> usize {
    0
}

fn set_pixel_format() -> bool {
    let mut format = RETRO_PIXEL_FORMAT_XRGB8888;
    callbacks::environment(
        RETRO_ENVIRONMENT_SET_PIXEL_FORMAT,
        (&mut format as *mut u32).cast(),
    )
}

fn set_performance_level() {
    let mut level = PERFORMANCE_LEVEL;
    callbacks::environment(
        RETRO_ENVIRONMENT_SET_PERFORMANCE_LEVEL,
        (&mut level as *mut u32).cast(),
    );
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CoreOptions {
    volume: u8,
}

impl Default for CoreOptions {
    fn default() -> Self {
        Self { volume: 100 }
    }
}

fn core_option_variables() -> Vec<RetroVariable> {
    vec![
        RetroVariable {
            key: c"dingooemu_volume".as_ptr(),
            value: c"Audio Volume (%); 100|90|80|70|60|50|40|30|20|10|0".as_ptr(),
        },
        RetroVariable {
            key: ptr::null(),
            value: ptr::null(),
        },
    ]
}

fn set_core_options() {
    let variables = core_option_variables();
    callbacks::environment(
        RETRO_ENVIRONMENT_SET_VARIABLES,
        variables.as_ptr().cast_mut().cast(),
    );
}

fn get_core_option(key: &CStr) -> Option<String> {
    let mut variable = RetroVariable {
        key: key.as_ptr(),
        value: ptr::null(),
    };
    let success = callbacks::environment(
        RETRO_ENVIRONMENT_GET_VARIABLE,
        (&mut variable as *mut RetroVariable).cast(),
    );
    if success && !variable.value.is_null() {
        unsafe {
            CStr::from_ptr(variable.value)
                .to_str()
                .ok()
                .map(str::to_owned)
        }
    } else {
        None
    }
}

fn core_options_changed() -> bool {
    let mut updated = false;
    callbacks::environment(
        RETRO_ENVIRONMENT_GET_VARIABLE_UPDATE,
        (&mut updated as *mut bool).cast(),
    ) && updated
}

fn read_core_options(mut get: impl FnMut(&CStr) -> Option<String>) -> CoreOptions {
    let mut options = CoreOptions::default();
    if let Some(volume) = get(c"dingooemu_volume").and_then(|value| value.parse::<u8>().ok()) {
        options.volume = volume.min(100);
    }
    options
}

fn apply_core_options(emulator: &mut Emulator) {
    let options = read_core_options(get_core_option);
    emulator.audio.set_master_volume(options.volume);
}

fn input_descriptors() -> [RetroInputDescriptor; 13] {
    let descriptor = |id, description: &'static CStr| RetroInputDescriptor {
        port: 0,
        device: RETRO_DEVICE_JOYPAD,
        index: 0,
        id,
        description: description.as_ptr(),
    };

    [
        descriptor(RETRO_DEVICE_ID_JOYPAD_UP, c"D-Pad Up"),
        descriptor(RETRO_DEVICE_ID_JOYPAD_DOWN, c"D-Pad Down"),
        descriptor(RETRO_DEVICE_ID_JOYPAD_LEFT, c"D-Pad Left"),
        descriptor(RETRO_DEVICE_ID_JOYPAD_RIGHT, c"D-Pad Right"),
        descriptor(RETRO_DEVICE_ID_JOYPAD_A, c"A"),
        descriptor(RETRO_DEVICE_ID_JOYPAD_B, c"B"),
        descriptor(RETRO_DEVICE_ID_JOYPAD_X, c"X"),
        descriptor(RETRO_DEVICE_ID_JOYPAD_Y, c"Y"),
        descriptor(RETRO_DEVICE_ID_JOYPAD_START, c"Start"),
        descriptor(RETRO_DEVICE_ID_JOYPAD_SELECT, c"Select"),
        descriptor(RETRO_DEVICE_ID_JOYPAD_L, c"L"),
        descriptor(RETRO_DEVICE_ID_JOYPAD_R, c"R"),
        RetroInputDescriptor {
            port: 0,
            device: 0,
            index: 0,
            id: 0,
            description: ptr::null(),
        },
    ]
}

fn register_input_descriptors() {
    let descriptors = input_descriptors();
    callbacks::environment(
        RETRO_ENVIRONMENT_SET_INPUT_DESCRIPTORS,
        descriptors.as_ptr().cast_mut().cast(),
    );
}

fn query_joypad_buttons(mut pressed: impl FnMut(u32) -> bool) -> u32 {
    const BUTTON_MAP: [(u32, u32); 12] = [
        (RETRO_DEVICE_ID_JOYPAD_UP, BUTTON_UP),
        (RETRO_DEVICE_ID_JOYPAD_DOWN, BUTTON_DOWN),
        (RETRO_DEVICE_ID_JOYPAD_LEFT, BUTTON_LEFT),
        (RETRO_DEVICE_ID_JOYPAD_RIGHT, BUTTON_RIGHT),
        (RETRO_DEVICE_ID_JOYPAD_A, BUTTON_A),
        (RETRO_DEVICE_ID_JOYPAD_B, BUTTON_B),
        (RETRO_DEVICE_ID_JOYPAD_X, BUTTON_X),
        (RETRO_DEVICE_ID_JOYPAD_Y, BUTTON_Y),
        (RETRO_DEVICE_ID_JOYPAD_START, BUTTON_START),
        (RETRO_DEVICE_ID_JOYPAD_SELECT, BUTTON_SELECT),
        (RETRO_DEVICE_ID_JOYPAD_L, BUTTON_L),
        (RETRO_DEVICE_ID_JOYPAD_R, BUTTON_R),
    ];

    BUTTON_MAP.iter().fold(0, |buttons, (id, mask)| {
        if pressed(*id) {
            buttons | mask
        } else {
            buttons
        }
    })
}

#[cfg(test)]
mod tests {
    use std::ffi::CString;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::Mutex;

    use super::*;

    static TEST_LOCK: Mutex<()> = Mutex::new(());
    static PIXEL_FORMAT: AtomicU32 = AtomicU32::new(u32::MAX);
    static INPUT_DESCRIPTORS_SET: AtomicBool = AtomicBool::new(false);
    static INPUT_POLLED: AtomicBool = AtomicBool::new(false);
    static VIDEO_WIDTH: AtomicU32 = AtomicU32::new(0);
    static AUDIO_BATCH_CALLED: AtomicBool = AtomicBool::new(false);

    unsafe extern "C" fn test_environment(command: u32, data: *mut c_void) -> bool {
        match command {
            RETRO_ENVIRONMENT_SET_PIXEL_FORMAT => {
                PIXEL_FORMAT.store(*(data.cast::<u32>()), Ordering::SeqCst);
                true
            }
            RETRO_ENVIRONMENT_SET_INPUT_DESCRIPTORS => {
                let descriptors = data.cast::<RetroInputDescriptor>();
                INPUT_DESCRIPTORS_SET
                    .store(!(*descriptors).description.is_null(), Ordering::SeqCst);
                true
            }
            RETRO_ENVIRONMENT_SET_PERFORMANCE_LEVEL => true,
            RETRO_ENVIRONMENT_SET_VARIABLES => true,
            RETRO_ENVIRONMENT_GET_VARIABLE | RETRO_ENVIRONMENT_GET_VARIABLE_UPDATE => false,
            RETRO_ENVIRONMENT_GET_LOG_INTERFACE => false,
            _ => false,
        }
    }

    unsafe extern "C" fn test_video_refresh(
        _data: *const c_void,
        width: u32,
        height: u32,
        pitch: usize,
    ) {
        assert_eq!(height, SCREEN_HEIGHT);
        assert_eq!(pitch, SCREEN_WIDTH as usize * std::mem::size_of::<u32>());
        VIDEO_WIDTH.store(width, Ordering::SeqCst);
    }

    unsafe extern "C" fn test_audio_batch(_data: *const i16, _frames: usize) -> usize {
        AUDIO_BATCH_CALLED.store(true, Ordering::SeqCst);
        0
    }

    unsafe extern "C" fn test_input_poll() {
        INPUT_POLLED.store(true, Ordering::SeqCst);
    }

    unsafe extern "C" fn test_input_state(port: u32, device: u32, index: u32, id: u32) -> i16 {
        assert_eq!((port, device, index), (0, RETRO_DEVICE_JOYPAD, 0));
        i16::from(matches!(
            id,
            RETRO_DEVICE_ID_JOYPAD_A | RETRO_DEVICE_ID_JOYPAD_START
        ))
    }

    fn minimal_app_bytes() -> Vec<u8> {
        let mut data = vec![0u8; 132];
        data[0..4].copy_from_slice(b"CCDL");
        data[0x20..0x24].copy_from_slice(b"IMPT");
        data[0x40..0x44].copy_from_slice(b"EXPT");
        data[0x60..0x64].copy_from_slice(b"RAWD");
        data[0x68..0x6c].copy_from_slice(&128u32.to_le_bytes());
        data[0x6c..0x70].copy_from_slice(&4u32.to_le_bytes());
        data[0x74..0x78].copy_from_slice(&0x8000_0000u32.to_le_bytes());
        data[0x78..0x7c].copy_from_slice(&0x8000_0000u32.to_le_bytes());
        data[0x7c..0x80].copy_from_slice(&4u32.to_le_bytes());
        data
    }

    #[test]
    fn maps_retropad_buttons_to_dingoo_masks() {
        let buttons = query_joypad_buttons(|id| {
            matches!(
                id,
                RETRO_DEVICE_ID_JOYPAD_UP
                    | RETRO_DEVICE_ID_JOYPAD_A
                    | RETRO_DEVICE_ID_JOYPAD_START
                    | RETRO_DEVICE_ID_JOYPAD_R
            )
        });

        assert_eq!(buttons, BUTTON_UP | BUTTON_A | BUTTON_START | BUTTON_R);
    }

    #[test]
    fn terminates_input_descriptor_array() {
        let descriptors = input_descriptors();
        assert_eq!(descriptors.len(), 13);
        assert!(descriptors[..12]
            .iter()
            .all(|descriptor| !descriptor.description.is_null()));
        assert!(descriptors[12].description.is_null());
    }

    #[test]
    fn volume_core_option_has_stable_key_and_default() {
        let variables = core_option_variables();
        assert_eq!(
            unsafe { CStr::from_ptr(variables[0].key) },
            c"dingooemu_volume"
        );
        assert!(unsafe { CStr::from_ptr(variables[0].value) }
            .to_str()
            .unwrap()
            .ends_with("; 100|90|80|70|60|50|40|30|20|10|0"));
        assert!(variables.last().unwrap().key.is_null());

        let options =
            read_core_options(|key| (key == c"dingooemu_volume").then(|| "30".to_string()));
        assert_eq!(options.volume, 30);
    }

    #[test]
    fn loads_starts_resets_and_unloads_content() {
        let _guard = TEST_LOCK.lock().unwrap();
        PIXEL_FORMAT.store(u32::MAX, Ordering::SeqCst);
        INPUT_DESCRIPTORS_SET.store(false, Ordering::SeqCst);
        INPUT_POLLED.store(false, Ordering::SeqCst);
        VIDEO_WIDTH.store(0, Ordering::SeqCst);
        AUDIO_BATCH_CALLED.store(false, Ordering::SeqCst);

        let path = std::env::temp_dir().join(format!(
            "dingooemu-libretro-test-{}.app",
            std::process::id()
        ));
        std::fs::write(&path, minimal_app_bytes()).unwrap();
        let path_string = CString::new(path.to_string_lossy().as_bytes()).unwrap();
        let info = RetroGameInfo {
            path: path_string.as_ptr(),
            data: ptr::null(),
            size: 0,
            meta: ptr::null(),
        };

        retro_set_environment(Some(test_environment));
        retro_set_video_refresh(Some(test_video_refresh));
        retro_set_audio_sample_batch(Some(test_audio_batch));
        retro_set_input_poll(Some(test_input_poll));
        retro_set_input_state(Some(test_input_state));
        retro_init();
        assert!(retro_load_game(&info));
        assert_eq!(
            PIXEL_FORMAT.load(Ordering::SeqCst),
            RETRO_PIXEL_FORMAT_XRGB8888
        );
        assert!(INPUT_DESCRIPTORS_SET.load(Ordering::SeqCst));

        retro_run();
        assert!(INPUT_POLLED.load(Ordering::SeqCst));
        assert_eq!(VIDEO_WIDTH.load(Ordering::SeqCst), SCREEN_WIDTH);
        assert!(AUDIO_BATCH_CALLED.load(Ordering::SeqCst));
        unsafe {
            let emulator = EMULATOR.as_mut().unwrap();
            assert!(emulator.is_running());
            assert_eq!(emulator.input.buttons(), BUTTON_A | BUTTON_START);
            emulator.memory.write_u32(0x1000, 0x1234_5678).unwrap();
        }

        retro_reset();
        unsafe {
            let emulator = EMULATOR.as_ref().unwrap();
            assert!(emulator.is_running());
            assert_eq!(emulator.memory.read_u32(0x1000).unwrap(), 0);
        }

        retro_unload_game();
        unsafe { assert!(EMULATOR.is_none()) };
        retro_deinit();
        std::fs::remove_file(path).unwrap();
    }
}
