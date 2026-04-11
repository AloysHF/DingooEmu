use dingooemu_core::Emulator;

/// Test loading and rendering the bundled local Snake.app sample.
#[test]
fn test_snake_app_reaches_framebuffer() {
    let app_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tmp/GameCollection/Snake.app");

    if !app_path.exists() {
        eprintln!("Skipping: Snake.app not found at {}", app_path.display());
        return;
    }

    let mut emu = Emulator::from_path(&app_path).expect("Failed to load Snake.app");
    emu.start();

    for _ in 0..12 {
        emu.tick().expect("Snake.app tick failed");
    }

    let framebuffer = emu.video.framebuffer();
    let non_zero = framebuffer.iter().filter(|&&b| b != 0).count();
    let unique_colors = framebuffer
        .chunks_exact(2)
        .map(|pixel| u16::from_le_bytes([pixel[0], pixel[1]]))
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
