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
    for addr in [
        0x80a00330u32,
        0x80a00334,
        0x80a00338,
        0x80a0033c,
        0x80a00340,
        0x80a00344,
        0x80a00348,
        0x80a0034c,
        0x80a00350,
    ] {
        let instr = emu.memory.read_u32(addr).unwrap_or(0);
        eprintln!("  {:#010x}: {:#010x}", addr, instr);
    }

    // Analyze the code flow
    eprintln!("\nCode analysis:");
    eprintln!("0x80a00324: JAL malloc - allocate memory");
    eprintln!("0x80a0032c: JAL malloc - allocate MORE memory");
    eprintln!("0x80a00334: SW $zero, 0($a0) - clear memory loop");
    eprintln!("0x80a00338: ADDIU $a0, $a0, 4 - next word");
    eprintln!("0x80a0033c: BNE $a0, $v0, -12 - loop until $a0 == $v0");
    eprintln!(
        "$v0 = {:#010x} (should be end of allocated buffer)",
        emu.cpu.regs.read(2)
    );
    eprintln!(
        "$a0 = {:#010x} (current clear address)",
        emu.cpu.regs.read(4)
    );

    emu.start();

    // Run and track framebuffer address changes
    let mut last_fb_addr = emu.video.framebuffer_addr();
    for frame in 0..20 {
        match emu.tick() {
            Ok(_) => {
                let fb_addr = emu.video.framebuffer_addr();
                if fb_addr != last_fb_addr {
                    eprintln!(
                        "Frame {}: FB address changed {:#010x} -> {:#010x}",
                        frame, last_fb_addr, fb_addr
                    );
                    last_fb_addr = fb_addr;

                    // Scan this new address for content
                    let mut non_zero = 0u32;
                    let size = 320 * 240 * 2;
                    for i in 0..size {
                        if let Ok(b) = emu.memory.read_u8(fb_addr + i) {
                            if b != 0 {
                                non_zero += 1;
                            }
                        }
                    }
                    eprintln!("  New FB: {}/{} non-zero bytes", non_zero, size);
                }

                if frame % 5 == 0 {
                    let fb = emu.video.framebuffer();
                    let non_zero = fb.iter().filter(|&&b| b != 0).count();
                    let fb_addr = emu.video.framebuffer_addr();
                    // Check if game is writing to this address
                    let mut mem_non_zero = 0u32;
                    let size = 320 * 240 * 2;
                    for i in 0..size {
                        if let Ok(b) = emu.memory.read_u8(fb_addr + i) {
                            if b != 0 {
                                mem_non_zero += 1;
                            }
                        }
                    }
                    eprintln!(
                        "Frame {}: PC={:#010x}, fb_addr={:#010x}, video_fb={}, mem_fb={}",
                        frame, emu.cpu.regs.pc, fb_addr, non_zero, mem_non_zero
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

    // Final memory scan
    let fb_addr = emu.video.framebuffer_addr();
    eprintln!("Final FB address: {:#010x}", fb_addr);
    eprintln!("Scanning memory around FB address:");
    for offset in [
        0u32,
        0x0100_0000,
        0x0200_0000,
        0x0300_0000,
        0x0400_0000,
        0x0500_0000,
    ] {
        let scan_addr = fb_addr.wrapping_add(offset);
        let mut non_zero = 0u32;
        let size = 320 * 240 * 2;
        for i in 0..size {
            if let Ok(b) = emu.memory.read_u8(scan_addr + i) {
                if b != 0 {
                    non_zero += 1;
                }
            }
        }
        if non_zero > 1000 {
            eprintln!(
                "  {:#010x}: {}/{} non-zero ({:.1}%)",
                scan_addr,
                non_zero,
                size,
                non_zero as f64 / size as f64 * 100.0
            );
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
            // Check if this looks like RGB565 pixel data
            let sample: Vec<String> = (0..32)
                .filter_map(|i| emu.memory.read_u8(region_start + i).ok())
                .map(|b| format!("{:02x}", b))
                .collect();
            eprintln!("    sample: {}", sample.join(" "));
        }
    }

    assert!(emu.cpu.instruction_count > 0);
}
