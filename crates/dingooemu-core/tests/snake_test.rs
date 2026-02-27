use dingooemu_core::Emulator;

/// Test loading and running Snake.app
#[test]
fn test_snake_app() {
    let app_path = "../../tmp/dingoo_emu/GameCollection/Snake.app";

    if !std::path::Path::new(app_path).exists() {
        eprintln!("Skipping: Snake.app not found at {}", app_path);
        return;
    }

    eprintln!("Loading Snake.app...");
    let mut emu = Emulator::from_path(app_path).expect("Failed to load Snake.app");

    eprintln!("Entry point: {:#010x}", emu.cpu.regs.pc);
    eprintln!("Frame buffer addr: {:#010x}", emu.video.framebuffer_addr());

    if let Some(app) = emu.app() {
        eprintln!("Imports ({} total):", app.imports.len());
        for import in app.imports.iter().take(30) {
            let instr = emu.memory.read_u32(import.address).unwrap_or(0);
            eprintln!(
                "  {:#010x}: {} -> instr={:#010x}",
                import.address, import.name, instr
            );
        }
    }

    // Dump instructions at the loop addresses
    eprintln!("Instructions at loop addresses:");
    for addr in [0x80a0033cu32, 0x80a00338, 0x80a00344, 0x80a00340] {
        let instr = emu.memory.read_u32(addr).unwrap_or(0);
        let next = emu.memory.read_u32(addr + 4).unwrap_or(0);
        eprintln!("  {:#010x}: {:#010x} {:#010x}", addr, instr, next);
    }

    emu.start();

    // Run just 1 frame and dump PC trace
    for frame in 0..5 {
        let start_pc = emu.cpu.regs.pc;
        let start_instr = emu.cpu.instruction_count;
        match emu.tick() {
            Ok(_) => {
                let end_pc = emu.cpu.regs.pc;
                let end_instr = emu.cpu.instruction_count;
                eprintln!(
                    "Frame {}: PC {:#010x} -> {:#010x}, {} instructions executed",
                    frame,
                    start_pc,
                    end_pc,
                    end_instr - start_instr
                );

                // Check if we're stuck in a loop
                if end_pc == start_pc {
                    eprintln!("  WARNING: PC didn't change! Might be stuck.");
                    // Dump instructions around current PC
                    for i in 0..5 {
                        let addr = end_pc.wrapping_add(i * 4);
                        let instr = emu.memory.read_u32(addr).unwrap_or(0);
                        eprintln!("  {:#010x}: {:#010x}", addr, instr);
                    }
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

    // Dump memory at various addresses to find where game writes pixels
    eprintln!("Memory scan for framebuffer content:");
    for scan_addr in [
        0x0300_0000u32,
        0x8300_0000,
        0x0320_0000,
        0x8320_0000,
        0x0500_0000,
        0x8500_0000,
    ] {
        let mut non_zero = 0u32;
        for i in 0..(320 * 240 * 2) {
            if let Ok(b) = emu.memory.read_u8(scan_addr + i) {
                if b != 0 {
                    non_zero += 1;
                }
            }
        }
        eprintln!(
            "  {:#010x}: {}/{} non-zero bytes",
            scan_addr,
            non_zero,
            320 * 240 * 2
        );
    }

    // Scan all of RAM for regions with high density of non-zero bytes
    eprintln!("Scanning RAM for framebuffer-like regions:");
    for region_start in (0x0000_0000..0x0200_0000).step_by(0x10_0000) {
        let mut non_zero = 0u32;
        let size = 320 * 240 * 2;
        for i in 0..size {
            if let Ok(b) = emu.memory.read_u8(region_start + i) {
                if b != 0 {
                    non_zero += 1;
                }
            }
        }
        let density = non_zero as f64 / size as f64 * 100.0;
        if density > 10.0 {
            eprintln!(
                "  {:#010x}: {}/{} ({:.1}%) non-zero",
                region_start, non_zero, size, density
            );
            // Dump first 64 bytes
            let bytes: Vec<String> = (0..64)
                .filter_map(|i| emu.memory.read_u8(region_start + i).ok())
                .map(|b| format!("{:02x}", b))
                .collect();
            eprintln!("    sample: {}", bytes.join(" "));
        }
    }

    assert!(emu.cpu.instruction_count > 0);
}
