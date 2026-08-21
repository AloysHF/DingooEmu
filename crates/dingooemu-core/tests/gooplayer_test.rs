use dingooemu_core::Emulator;
use std::path::PathBuf;

fn gooplayer_app_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("DINGOOEMU_GOOPLAYER_APP").map(PathBuf::from) {
        return path.exists().then_some(path);
    }

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tmp/dingoo_game/GooPlayer/GooPlayer.app");
    path.exists().then_some(path)
}

#[test]
fn test_gooplayer_discovers_tracks_and_opens_playlist() {
    let Some(app_path) = gooplayer_app_path() else {
        eprintln!("Skipping: set DINGOOEMU_GOOPLAYER_APP to test GooPlayer.app");
        return;
    };

    let mut emu = Emulator::from_path(&app_path).expect("Failed to load GooPlayer.app");
    emu.start();
    for _ in 0..300 {
        emu.tick().expect("GooPlayer.app tick failed");
    }

    let highlighted_pixels = emu
        .video
        .framebuffer()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pixel| u16::from_le_bytes(*pixel))
        .filter(|pixel| {
            let red = (pixel >> 11) & 0x1f;
            let green = (pixel >> 5) & 0x3f;
            let blue = pixel & 0x1f;
            red >= 24 && green >= 48 && blue <= 12
        })
        .count();

    assert!(
        highlighted_pixels > 10,
        "GooPlayer.app did not leave its title screen and render a selected track"
    );
}
