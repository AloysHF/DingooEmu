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

    // Dump first few instructions at entry point
    eprintln!("Instructions at entry point:");
    for i in 0..10 {
        let addr = emu.cpu.regs.pc + i * 4;
        match emu.memory.read_u32(addr) {
            Ok(instr) => eprintln!("  {:#010x}: {:#010x}", addr, instr),
            Err(e) => eprintln!("  {:#010x}: Error: {:?}", addr, e),
        }
    }

    // Start emulation
    emu.start();

    // Run for 100 frames with detailed logging
    for frame in 0..100 {
        // Log before tick
        if frame < 5 {
            eprintln!(
                "Frame {} - Before tick: PC={:#010x}, $sp={:#010x}, $ra={:#010x}",
                frame,
                emu.cpu.regs.pc,
                emu.cpu.regs.read(29),
                emu.cpu.regs.read(31)
            );
        }

        match emu.tick() {
            Ok(_) => {
                if frame < 5 {
                    eprintln!(
                        "Frame {} - After tick: PC={:#010x}, instructions={}",
                        frame, emu.cpu.regs.pc, emu.cpu.instruction_count
                    );
                }
            }
            Err(e) => {
                eprintln!("Frame {} - Error: {:?}", frame, e);
                eprintln!("PC at error: {:#010x}", emu.cpu.regs.pc);
                eprintln!("Registers at error:");
                for i in 0..32 {
                    eprintln!("  ${:2} = {:#010x}", i, emu.cpu.regs.read(i));
                }
                break;
            }
        }

        // Stop if CPU stops running
        if !emu.cpu.is_running() {
            eprintln!("CPU stopped at frame {}", frame);
            break;
        }
    }

    eprintln!("Test completed!");
    eprintln!("Final PC: {:#010x}", emu.cpu.regs.pc);
    eprintln!("Total instructions: {}", emu.cpu.instruction_count);

    // Verify that some instructions were executed
    assert!(
        emu.cpu.instruction_count > 0,
        "CPU should have executed some instructions"
    );
}
