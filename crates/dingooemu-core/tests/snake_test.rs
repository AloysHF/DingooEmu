#[cfg(not(feature = "standalone"))]
use dingooemu_core::audio::{AudioConfig, SampleFormat};
#[cfg(not(feature = "standalone"))]
use dingooemu_core::input::{BUTTON_A, BUTTON_DOWN};
use dingooemu_core::Emulator;
use std::path::PathBuf;

fn snake_app_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("DINGOOEMU_SNAKE_APP").map(PathBuf::from) {
        return path.exists().then_some(path);
    }

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tmp/dingoo_game/Snake.app");
    path.exists().then_some(path)
}

/// Test loading and rendering the bundled local Snake.app sample.
#[test]
fn test_snake_app_reaches_framebuffer() {
    let Some(app_path) = snake_app_path() else {
        eprintln!("Skipping: set DINGOOEMU_SNAKE_APP to test Snake.app");
        return;
    };

    let mut emu = Emulator::from_path(&app_path).expect("Failed to load Snake.app");
    emu.start();

    for _ in 0..12 {
        emu.tick().expect("Snake.app tick failed");
    }

    let framebuffer = emu.video.framebuffer();
    let non_zero = framebuffer.iter().filter(|&&b| b != 0).count();
    let unique_colors = framebuffer
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pixel| u16::from_le_bytes(*pixel))
        .collect::<std::collections::BTreeSet<_>>()
        .len();

    assert!(emu.cpu.instruction_count > 0);
    assert!(
        non_zero > 0,
        "Snake.app did not produce framebuffer pixels after {} instructions",
        emu.cpu.instruction_count
    );
    assert!(
        unique_colors > 8,
        "Snake.app framebuffer is still effectively solid: {unique_colors} colors"
    );
}

#[test]
fn test_snake_app_does_not_force_startup_audio() {
    let Some(app_path) = snake_app_path() else {
        eprintln!("Skipping: set DINGOOEMU_SNAKE_APP to test Snake.app");
        return;
    };

    let mut emu = Emulator::from_path(&app_path).expect("Failed to load Snake.app");
    emu.start();

    for _ in 0..20 {
        emu.tick().expect("Snake.app tick failed");
    }

    assert!(
        emu.audio.config().is_none(),
        "Snake.app audio was started without a guest request"
    );
}

#[test]
#[cfg(not(feature = "standalone"))]
fn test_snake_app_enables_audio_from_menu() {
    let Some(app_path) = snake_app_path() else {
        eprintln!("Skipping: set DINGOOEMU_SNAKE_APP to test Snake.app");
        return;
    };

    let mut emu = Emulator::from_path(&app_path).expect("Failed to load Snake.app");
    emu.start();

    let inputs = [
        (BUTTON_DOWN, 30..36),
        (BUTTON_DOWN, 90..96),
        (BUTTON_DOWN, 150..156),
        (BUTTON_A, 210..216),
    ];
    let mut non_zero_samples = 0usize;
    for frame in 0..300 {
        let buttons = inputs
            .iter()
            .find_map(|(button, frames)| frames.contains(&frame).then_some(*button))
            .unwrap_or(0);
        emu.set_buttons(buttons);
        emu.tick().expect("Snake.app tick failed");
        non_zero_samples += emu
            .take_audio_samples()
            .into_iter()
            .filter(|sample| *sample != 0)
            .count();
    }

    assert_eq!(
        emu.audio.config(),
        Some(AudioConfig {
            sample_rate: 8_000,
            format: SampleFormat::S16Le,
            channels: 1,
            volume: 50,
        })
    );
    assert!(
        non_zero_samples > 0,
        "Snake.app produced only silent audio after enabling sound"
    );
}
