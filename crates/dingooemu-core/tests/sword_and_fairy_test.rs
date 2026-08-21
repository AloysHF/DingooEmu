use dingooemu_core::Emulator;
use std::collections::BTreeSet;
use std::path::PathBuf;

fn sword_and_fairy_app_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("DINGOOEMU_SWORD_AND_FAIRY_APP").map(PathBuf::from) {
        return path.exists().then_some(path);
    }

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tmp/dingoo_game/仙剑奇侠传/仙剑奇侠传.APP");
    path.exists().then_some(path)
}

#[test]
fn test_sword_and_fairy_reaches_framebuffer() {
    let Some(app_path) = sword_and_fairy_app_path() else {
        eprintln!("Skipping: set DINGOOEMU_SWORD_AND_FAIRY_APP to test the game");
        return;
    };

    let mut emu = Emulator::from_path(&app_path).expect("Failed to load game");
    emu.start();

    for _ in 0..60 {
        emu.tick().expect("Game tick failed");
    }

    let framebuffer = emu.video.framebuffer();
    let non_zero = framebuffer.iter().filter(|&&byte| byte != 0).count();
    let unique_colors = framebuffer
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pixel| u16::from_le_bytes(*pixel))
        .collect::<BTreeSet<_>>()
        .len();

    assert!(emu.cpu.instruction_count > 0);
    assert!(
        non_zero > 0,
        "Game did not produce framebuffer pixels after {} instructions",
        emu.cpu.instruction_count
    );
    assert!(
        unique_colors > 1,
        "Game framebuffer is still solid after startup: {unique_colors} color"
    );
}
