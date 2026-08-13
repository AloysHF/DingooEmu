use dingooemu_core::input::BUTTON_A;
use dingooemu_core::Emulator;
use std::path::PathBuf;

fn overlord_fighter_app_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("DINGOOEMU_OVERLORD_FIGHTER_APP").map(PathBuf::from) {
        return path.exists().then_some(path);
    }

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tmp/dingoo_game/Overlord-Fighter.app");
    path.exists().then_some(path)
}

fn run_menu(app_path: &PathBuf, with_input: bool) -> Emulator {
    let mut emu = Emulator::from_path(app_path).expect("Failed to load Overlord-Fighter.app");
    emu.start();
    for frame in 0..120 {
        let pressed = with_input && (80..85).contains(&frame);
        emu.set_buttons(if pressed { BUTTON_A } else { 0 });
        emu.tick().expect("Overlord-Fighter.app tick failed");
    }
    emu
}

#[test]
fn test_overlord_fighter_accepts_title_screen_input() {
    let Some(app_path) = overlord_fighter_app_path() else {
        eprintln!("Skipping: set DINGOOEMU_OVERLORD_FIGHTER_APP to test Overlord-Fighter.app");
        return;
    };

    let control = run_menu(&app_path, false);
    let input = run_menu(&app_path, true);
    assert_eq!(control.video.framebuffer_crc32(), 0xd7d5_e307);
    assert_eq!(input.video.framebuffer_crc32(), 0x7999_d125);

    let unknown: Vec<_> = input
        .unknown_hle_calls()
        .map(|call| call.name.as_str())
        .collect();
    for implemented in [
        "open_gui_key_msg",
        "WM_CreateWindow",
        "WM_SetFocus",
        "GUI_Exec",
        "U8TOX16",
        "U8TOX32",
    ] {
        assert!(
            !unknown.contains(&implemented),
            "implemented GUI call remained unknown: {implemented}"
        );
    }
}
