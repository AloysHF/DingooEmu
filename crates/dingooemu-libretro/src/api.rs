use std::ffi::{c_void, CStr};
use std::os::raw::{c_char, c_uint};
use std::ptr;

use dingooemu_core::cpu::UnknownInstructionPolicy;
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
    info.library_version = c"0.2.0".as_ptr();
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
        log::error!("Frontend rejected the required RGB565 pixel format");
        return false;
    }
    register_input_descriptors();
    set_performance_level();

    match Emulator::from_path(path) {
        Ok(mut emulator) => {
            if let Some(save_directory) = frontend_directory(RETRO_ENVIRONMENT_GET_SAVE_DIRECTORY) {
                emulator.set_save_directory(save_directory);
            }
            apply_core_options(&mut emulator);
            emulator.start();
            unsafe { EMULATOR = Some(emulator) };
            if let Some(emulator) = unsafe { EMULATOR.as_mut() } {
                register_memory_maps(emulator);
            }
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
    if let Some(emulator) = unsafe { EMULATOR.as_mut() } {
        emulator.flush_save_files();
    }
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

    callbacks::video_refresh(
        emulator.video.framebuffer().as_ptr().cast(),
        SCREEN_WIDTH,
        SCREEN_HEIGHT,
        SCREEN_WIDTH as usize * std::mem::size_of::<u16>(),
    );

    let samples = emulator.take_audio_samples();
    if callbacks::audio_sample_batch(samples.as_ptr(), samples.len() / 2).is_none() {
        for sample in samples.as_chunks::<2>().0.iter() {
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
    unsafe { EMULATOR.as_ref() }.map_or(0, Emulator::serialized_state_size)
}

#[no_mangle]
pub extern "C" fn retro_serialize(data: *mut c_void, size: usize) -> bool {
    let Some(emulator) = (unsafe { EMULATOR.as_ref() }) else {
        return false;
    };
    if data.is_null() || size < emulator.serialized_state_size() {
        return false;
    }
    let output = unsafe { std::slice::from_raw_parts_mut(data.cast::<u8>(), size) };
    match emulator.serialize_state(output) {
        Ok(()) => true,
        Err(error) => {
            log::error!("Failed to serialize state: {error}");
            false
        }
    }
}

#[no_mangle]
pub extern "C" fn retro_unserialize(data: *const c_void, size: usize) -> bool {
    let Some(emulator) = (unsafe { EMULATOR.as_mut() }) else {
        return false;
    };
    if data.is_null() || size < emulator.serialized_state_size() {
        return false;
    }
    let input = unsafe { std::slice::from_raw_parts(data.cast::<u8>(), size) };
    match emulator.unserialize_state(input) {
        Ok(()) => {
            apply_core_options(emulator);
            true
        }
        Err(error) => {
            log::error!("Failed to unserialize state: {error}");
            false
        }
    }
}

#[no_mangle]
pub extern "C" fn retro_cheat_reset() {
    if let Some(emulator) = unsafe { EMULATOR.as_mut() } {
        emulator.clear_cheats();
    }
}

#[no_mangle]
pub extern "C" fn retro_cheat_set(index: c_uint, enabled: bool, code: *const c_char) {
    let Some(emulator) = (unsafe { EMULATOR.as_mut() }) else {
        return;
    };
    if code.is_null() {
        log::warn!("Ignored null cheat code for slot {index}");
        return;
    }
    let code = unsafe { CStr::from_ptr(code) }.to_string_lossy();
    if let Err(error) = emulator.set_cheat(index, enabled, &code) {
        log::warn!("Rejected cheat slot {index}: {error}");
    }
}

#[no_mangle]
pub extern "C" fn retro_get_memory_data(id: c_uint) -> *mut c_void {
    let Some(emulator) = (unsafe { EMULATOR.as_mut() }) else {
        return ptr::null_mut();
    };
    match id & RETRO_MEMORY_MASK {
        RETRO_MEMORY_SYSTEM_RAM => emulator.memory.system_ram_mut().as_mut_ptr().cast(),
        RETRO_MEMORY_VIDEO_RAM => emulator.memory.framebuffer_mut().as_mut_ptr().cast(),
        _ => ptr::null_mut(),
    }
}

#[no_mangle]
pub extern "C" fn retro_get_memory_size(id: c_uint) -> usize {
    let Some(emulator) = (unsafe { EMULATOR.as_ref() }) else {
        return 0;
    };
    match id & RETRO_MEMORY_MASK {
        RETRO_MEMORY_SYSTEM_RAM => emulator.memory.system_ram().len(),
        RETRO_MEMORY_VIDEO_RAM => emulator.memory.framebuffer().len(),
        _ => 0,
    }
}

fn set_pixel_format() -> bool {
    let mut format = RETRO_PIXEL_FORMAT_RGB565;
    callbacks::environment(
        RETRO_ENVIRONMENT_SET_PIXEL_FORMAT,
        (&mut format as *mut u32).cast(),
    )
}

fn frontend_directory(command: u32) -> Option<std::path::PathBuf> {
    let mut value: *const c_char = ptr::null();
    if !callbacks::environment(command, (&mut value as *mut *const c_char).cast())
        || value.is_null()
    {
        return None;
    }
    let path = unsafe { CStr::from_ptr(value) }.to_string_lossy();
    (!path.is_empty()).then(|| std::path::PathBuf::from(path.as_ref()))
}

fn register_memory_maps(emulator: &mut Emulator) {
    let descriptors = [
        RetroMemoryDescriptor {
            flags: RETRO_MEMDESC_SYSTEM_RAM,
            ptr: emulator.memory.system_ram_mut().as_mut_ptr().cast(),
            offset: 0,
            start: 0,
            select: 0,
            disconnect: 0,
            len: emulator.memory.system_ram().len(),
            addrspace: c"Dingoo".as_ptr(),
        },
        RetroMemoryDescriptor {
            flags: RETRO_MEMDESC_VIDEO_RAM,
            ptr: emulator.memory.framebuffer_mut().as_mut_ptr().cast(),
            offset: 0,
            start: dingooemu_core::video::VM_LCD_FB_ADDRESS as usize,
            select: 0,
            disconnect: 0,
            len: emulator.memory.framebuffer().len(),
            addrspace: c"Dingoo".as_ptr(),
        },
    ];
    let memory_map = RetroMemoryMap {
        descriptors: descriptors.as_ptr(),
        num_descriptors: descriptors.len() as u32,
    };
    if !callbacks::environment(
        RETRO_ENVIRONMENT_SET_MEMORY_MAPS,
        (&memory_map as *const RetroMemoryMap).cast_mut().cast(),
    ) {
        log::warn!("Frontend did not accept memory descriptors");
    }
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
    repeat_delay: u32,
    repeat_period: u32,
    swap_ab: bool,
    debug_logging: bool,
    unknown_instruction_policy: UnknownInstructionPolicy,
    jit_enabled: bool,
}

impl Default for CoreOptions {
    fn default() -> Self {
        Self {
            volume: 100,
            repeat_delay: 24,
            repeat_period: 6,
            swap_ab: false,
            debug_logging: false,
            unknown_instruction_policy: UnknownInstructionPolicy::Skip,
            jit_enabled: true,
        }
    }
}

fn core_option_variables() -> Vec<RetroVariable> {
    let mut variables = vec![
        RetroVariable {
            key: c"dingooemu_volume".as_ptr(),
            value: c"Audio Volume (%); 100|90|80|70|60|50|40|30|20|10|0".as_ptr(),
        },
        RetroVariable {
            key: c"dingooemu_repeat_delay".as_ptr(),
            value: c"Key Auto-Repeat Delay (frames); 24|0|2|4|6|8|10|12|16|20|30|45|60".as_ptr(),
        },
        RetroVariable {
            key: c"dingooemu_repeat_period".as_ptr(),
            value: c"Key Auto-Repeat Period (frames); 6|1|2|3|4|5|8|10|12|15|20|30".as_ptr(),
        },
        RetroVariable {
            key: c"dingooemu_swap_ab".as_ptr(),
            value: c"Swap A/B Buttons; disabled|enabled".as_ptr(),
        },
        RetroVariable {
            key: c"dingooemu_debug_logging".as_ptr(),
            value: c"CPU/HLE Debug Logging; disabled|enabled".as_ptr(),
        },
        RetroVariable {
            key: c"dingooemu_unknown_instruction".as_ptr(),
            value: c"Unknown MIPS Instruction Policy; skip|stop".as_ptr(),
        },
    ];
    #[cfg(all(
        target_os = "android",
        target_pointer_width = "64",
        any(target_arch = "aarch64", target_arch = "x86_64")
    ))]
    variables.push(RetroVariable {
        key: c"dingooemu_cpu_engine".as_ptr(),
        value: c"CPU Execution Engine; jit|interpreter".as_ptr(),
    });
    variables.push(RetroVariable {
        key: ptr::null(),
        value: ptr::null(),
    });
    variables
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
    if let Some(delay) = get(c"dingooemu_repeat_delay").and_then(|value| value.parse::<u32>().ok())
    {
        options.repeat_delay = delay;
    }
    if let Some(period) =
        get(c"dingooemu_repeat_period").and_then(|value| value.parse::<u32>().ok())
    {
        options.repeat_period = period.max(1);
    }
    if let Some(swap) = get(c"dingooemu_swap_ab") {
        options.swap_ab = swap == "enabled";
    }
    if let Some(debug) = get(c"dingooemu_debug_logging") {
        options.debug_logging = debug == "enabled";
    }
    if let Some(policy) = get(c"dingooemu_unknown_instruction") {
        options.unknown_instruction_policy = if policy == "stop" {
            UnknownInstructionPolicy::Stop
        } else {
            UnknownInstructionPolicy::Skip
        };
    }
    if let Some(engine) = get(c"dingooemu_cpu_engine") {
        options.jit_enabled = engine != "interpreter";
    }
    options
}

fn apply_core_options(emulator: &mut Emulator) {
    let options = read_core_options(get_core_option);
    emulator.audio.set_master_volume(options.volume);
    emulator
        .input
        .set_repeat_timing(options.repeat_delay, options.repeat_period);
    emulator.input.set_swap_ab(options.swap_ab);
    crate::logger::set_debug_logging(options.debug_logging);
    emulator
        .cpu
        .set_unknown_instruction_policy(options.unknown_instruction_policy);
    emulator.set_jit_enabled(options.jit_enabled);
    log::info!(
        "Core options applied: volume={} repeat_delay={} repeat_period={} swap_ab={} debug_logging={} unknown_instruction={:?} cpu_engine={}",
        options.volume,
        options.repeat_delay,
        options.repeat_period,
        options.swap_ab,
        options.debug_logging,
        options.unknown_instruction_policy,
        if options.jit_enabled { "jit" } else { "interpreter" }
    );
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
    static MEMORY_MAPS_SET: AtomicBool = AtomicBool::new(false);
    static SAVE_DIRECTORY: Mutex<Option<CString>> = Mutex::new(None);

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
            RETRO_ENVIRONMENT_SET_MEMORY_MAPS => {
                let memory_map = &*data.cast::<RetroMemoryMap>();
                MEMORY_MAPS_SET.store(
                    memory_map.num_descriptors == 2 && !memory_map.descriptors.is_null(),
                    Ordering::SeqCst,
                );
                true
            }
            RETRO_ENVIRONMENT_SET_VARIABLES => true,
            RETRO_ENVIRONMENT_GET_VARIABLE | RETRO_ENVIRONMENT_GET_VARIABLE_UPDATE => false,
            RETRO_ENVIRONMENT_GET_LOG_INTERFACE => false,
            RETRO_ENVIRONMENT_GET_SAVE_DIRECTORY => {
                let directory = SAVE_DIRECTORY.lock().unwrap();
                let Some(directory) = directory.as_ref() else {
                    return false;
                };
                *data.cast::<*const c_char>() = directory.as_ptr();
                true
            }
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
        assert_eq!(pitch, SCREEN_WIDTH as usize * std::mem::size_of::<u16>());
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
    fn repeat_delay_option_supports_live_typematic_control() {
        let variables = core_option_variables();
        assert_eq!(
            unsafe { CStr::from_ptr(variables[1].key) },
            c"dingooemu_repeat_delay"
        );
        let options =
            read_core_options(|key| (key == c"dingooemu_repeat_delay").then(|| "12".to_string()));
        assert_eq!(options.repeat_delay, 12);
    }

    #[test]
    fn repeat_period_option_rejects_zero_semantically() {
        let variables = core_option_variables();
        assert_eq!(
            unsafe { CStr::from_ptr(variables[2].key) },
            c"dingooemu_repeat_period"
        );
        let options =
            read_core_options(|key| (key == c"dingooemu_repeat_period").then(|| "0".to_string()));
        assert_eq!(options.repeat_period, 1);
    }

    #[test]
    fn swap_ab_option_defaults_to_disabled() {
        let variables = core_option_variables();
        assert_eq!(
            unsafe { CStr::from_ptr(variables[3].key) },
            c"dingooemu_swap_ab"
        );
        assert!(!read_core_options(|_| None).swap_ab);
        assert!(
            read_core_options(|key| {
                (key == c"dingooemu_swap_ab").then(|| "enabled".to_string())
            })
            .swap_ab
        );
    }

    #[test]
    fn debug_logging_option_defaults_to_disabled() {
        let variables = core_option_variables();
        assert_eq!(
            unsafe { CStr::from_ptr(variables[4].key) },
            c"dingooemu_debug_logging"
        );
        assert!(!read_core_options(|_| None).debug_logging);
        assert!(
            read_core_options(|key| {
                (key == c"dingooemu_debug_logging").then(|| "enabled".to_string())
            })
            .debug_logging
        );
    }

    #[test]
    fn unknown_instruction_option_preserves_skip_default() {
        let variables = core_option_variables();
        assert_eq!(
            unsafe { CStr::from_ptr(variables[5].key) },
            c"dingooemu_unknown_instruction"
        );
        assert_eq!(
            read_core_options(|_| None).unknown_instruction_policy,
            UnknownInstructionPolicy::Skip
        );
        assert_eq!(
            read_core_options(|key| {
                (key == c"dingooemu_unknown_instruction").then(|| "stop".to_string())
            })
            .unknown_instruction_policy,
            UnknownInstructionPolicy::Stop
        );
    }

    #[test]
    #[cfg(all(
        target_os = "android",
        target_pointer_width = "64",
        any(target_arch = "aarch64", target_arch = "x86_64")
    ))]
    fn cpu_engine_option_defaults_to_jit_and_allows_interpreter() {
        let variables = core_option_variables();
        assert_eq!(
            unsafe { CStr::from_ptr(variables[6].key) },
            c"dingooemu_cpu_engine"
        );
        assert!(read_core_options(|_| None).jit_enabled);
        assert!(
            !read_core_options(|key| {
                (key == c"dingooemu_cpu_engine").then(|| "interpreter".to_string())
            })
            .jit_enabled
        );
    }

    #[test]
    fn loads_starts_resets_and_unloads_content() {
        let _guard = TEST_LOCK.lock().unwrap();
        PIXEL_FORMAT.store(u32::MAX, Ordering::SeqCst);
        INPUT_DESCRIPTORS_SET.store(false, Ordering::SeqCst);
        INPUT_POLLED.store(false, Ordering::SeqCst);
        VIDEO_WIDTH.store(0, Ordering::SeqCst);
        AUDIO_BATCH_CALLED.store(false, Ordering::SeqCst);
        MEMORY_MAPS_SET.store(false, Ordering::SeqCst);

        let path = std::env::temp_dir().join(format!(
            "dingooemu-libretro-test-{}.app",
            std::process::id()
        ));
        std::fs::write(&path, minimal_app_bytes()).unwrap();
        let path_string = CString::new(path.to_string_lossy().as_bytes()).unwrap();
        *SAVE_DIRECTORY.lock().unwrap() =
            Some(CString::new(path.parent().unwrap().to_string_lossy().as_bytes()).unwrap());
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
            RETRO_PIXEL_FORMAT_RGB565
        );
        assert!(INPUT_DESCRIPTORS_SET.load(Ordering::SeqCst));
        assert!(MEMORY_MAPS_SET.load(Ordering::SeqCst));
        assert_eq!(
            retro_get_memory_size(RETRO_MEMORY_SYSTEM_RAM),
            32 * 1024 * 1024
        );
        assert!(retro_get_memory_size(RETRO_MEMORY_VIDEO_RAM) >= (320 * 240 * 2) as usize);
        let system_ram = retro_get_memory_data(RETRO_MEMORY_SYSTEM_RAM);
        let video_ram = retro_get_memory_data(RETRO_MEMORY_VIDEO_RAM);
        assert!(!system_ram.is_null());
        assert!(!video_ram.is_null());

        retro_run();
        assert!(INPUT_POLLED.load(Ordering::SeqCst));
        assert_eq!(VIDEO_WIDTH.load(Ordering::SeqCst), SCREEN_WIDTH);
        assert!(AUDIO_BATCH_CALLED.load(Ordering::SeqCst));
        let cheat = CString::new("mem32:0x1000=0xfeedbeef").unwrap();
        retro_cheat_set(0, true, cheat.as_ptr());
        retro_run();
        unsafe {
            let emulator = EMULATOR.as_mut().unwrap();
            assert!(emulator.is_running());
            assert_eq!(emulator.input.buttons(), BUTTON_A | BUTTON_START);
            assert_eq!(emulator.memory.read_u32(0x1000).unwrap(), 0xfeed_beef);
            emulator.memory.write_u32(0x1000, 0x1234_5678).unwrap();
        }

        let mut state = vec![0u8; retro_serialize_size()];
        assert!(retro_serialize(state.as_mut_ptr().cast(), state.len()));
        unsafe {
            EMULATOR
                .as_mut()
                .unwrap()
                .memory
                .write_u32(0x1000, 0)
                .unwrap();
        }
        assert!(retro_unserialize(state.as_ptr().cast(), state.len()));
        assert_eq!(retro_get_memory_data(RETRO_MEMORY_SYSTEM_RAM), system_ram);
        assert_eq!(retro_get_memory_data(RETRO_MEMORY_VIDEO_RAM), video_ram);
        unsafe {
            assert_eq!(
                EMULATOR.as_ref().unwrap().memory.read_u32(0x1000).unwrap(),
                0x1234_5678
            );
        }
        retro_cheat_reset();
        unsafe {
            EMULATOR
                .as_mut()
                .unwrap()
                .memory
                .write_u32(0x1000, 0)
                .unwrap();
        }
        retro_run();
        unsafe {
            assert_eq!(
                EMULATOR.as_ref().unwrap().memory.read_u32(0x1000).unwrap(),
                0
            );
        }

        retro_reset();
        assert_eq!(retro_get_memory_data(RETRO_MEMORY_SYSTEM_RAM), system_ram);
        assert_eq!(retro_get_memory_data(RETRO_MEMORY_VIDEO_RAM), video_ram);
        unsafe {
            let emulator = EMULATOR.as_ref().unwrap();
            assert!(emulator.is_running());
            assert_eq!(emulator.memory.read_u32(0x1000).unwrap(), 0);
        }

        retro_unload_game();
        assert!(retro_get_memory_data(RETRO_MEMORY_SYSTEM_RAM).is_null());
        assert_eq!(retro_get_memory_size(RETRO_MEMORY_SYSTEM_RAM), 0);
        unsafe { assert!(EMULATOR.is_none()) };
        retro_deinit();
        *SAVE_DIRECTORY.lock().unwrap() = None;
        std::fs::remove_file(path).unwrap();
    }
}
