use dingooemu_core::Emulator;

/// Test loading and running Snake.app
#[test]
fn test_snake_app() {
    // Path relative to workspace root
    let app_path = "../../tmp/dingoo_emu/GameCollection/Snake.app";

    // Check if file exists
    if !std::path::Path::new(app_path).exists() {
        eprintln!("Skipping: Snake.app not found at {}", app_path);
        return;
    }

    eprintln!("Loading Snake.app...");

    // Load the game
    let mut emu = Emulator::from_path(app_path).expect("Failed to load Snake.app");

    eprintln!("Game loaded successfully!");
    eprintln!("Entry point: {:#010x}", emu.cpu.regs.pc);
    eprintln!(
        "Frame buffer address: {:#010x}",
        emu.video.framebuffer_addr()
    );

    // Check imports for framebuffer-related functions
    if let Some(app) = emu.app() {
        eprintln!("Imports ({} total):", app.imports.len());
        for import in app.imports.iter().take(20) {
            eprintln!("  {:#010x}: {}", import.address, import.name);
        }
    }

    // Start emulation
    emu.start();

    // Run for 100 frames
    for frame in 0..100 {
        match emu.tick() {
            Ok(_) => {
                if frame % 20 == 0 {
                    // Check framebuffer content
                    let fb = emu.video.framebuffer();
                    let non_zero = fb.iter().filter(|&&b| b != 0).count();
                    eprintln!(
                        "Frame {}: PC={:#010x}, instructions={}, fb_non_zero={}",
                        frame, emu.cpu.regs.pc, emu.cpu.instruction_count, non_zero
                    );
                }
            }
            Err(e) => {
                eprintln!("Frame {} - Error: {:?}", frame, e);
                break;
            }
        }

        if !emu.cpu.is_running() {
            eprintln!("CPU stopped at frame {}", frame);
            break;
        }
    }

    eprintln!("Test completed!");
    eprintln!("Final PC: {:#010x}", emu.cpu.regs.pc);
    eprintln!("Total instructions: {}", emu.cpu.instruction_count);

    assert!(
        emu.cpu.instruction_count > 0,
        "CPU should have executed some instructions"
    );
}
