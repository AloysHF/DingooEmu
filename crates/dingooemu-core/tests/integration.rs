use dingooemu_core::Emulator;

/// Test loading a simple .app file
#[test]
#[ignore] // Requires actual .app files
fn test_load_simple_game() {
    let app_path = "test_games/simple.app";
    if !std::path::Path::new(app_path).exists() {
        eprintln!("Skipping: test game not found at {}", app_path);
        return;
    }

    let mut emu = Emulator::from_path(app_path).unwrap();
    emu.start();

    // Run 100 frames
    for _frame in 0..100 {
        emu.tick().unwrap();
    }

    // Verify CPU is still running or has executed instructions
    assert!(
        emu.cpu.is_running() || emu.cpu.instruction_count > 0,
        "CPU should have executed some instructions"
    );
}

/// Test .app parsing
#[test]
fn test_app_parsing() {
    use dingooemu_core::app_loader::AppImage;

    // Create a minimal .app file for testing
    let mut data = vec![0u8; 256];

    // CCDL magic at offset 0
    data[0..4].copy_from_slice(b"CCDL");

    // IMPT descriptor at offset 0x20
    data[0x20..0x24].copy_from_slice(b"IMPT");

    // EXPT descriptor at offset 0x40
    data[0x40..0x44].copy_from_slice(b"EXPT");

    // RAWD descriptor at offset 0x60
    data[0x60..0x64].copy_from_slice(b"RAWD");

    // RAWD header fields
    data[0x74..0x78].copy_from_slice(&0x8000_0000u32.to_le_bytes()); // entry point
    data[0x78..0x7C].copy_from_slice(&0x8000_0000u32.to_le_bytes()); // origin
    data[0x7C..0x80].copy_from_slice(&0x80u32.to_le_bytes()); // program_size

    // Parse the .app file
    let app = AppImage::parse(&data).unwrap();

    assert_eq!(app.entry_point(), 0x8000_0000);
    assert_eq!(app.load_base(), 0x8000_0000);
    assert_eq!(app.program_size(), 0x80);
}

/// Test CPU instruction execution
#[test]
fn test_cpu_instructions() {
    use dingooemu_core::cpu::Cpu;
    use dingooemu_core::memory::Memory;

    let mut cpu = Cpu::new(0);
    let mut mem = Memory::new();

    // ADDIU $t0, $zero, 0x1234
    let instr = (0x09 << 26) | (8 << 16) | 0x1234;
    mem.write_u32(0, instr).unwrap();

    cpu.start();
    cpu.step(&mut mem).unwrap();

    assert_eq!(cpu.regs.read(8), 0x1234);
}

/// Test branch instructions
#[test]
fn test_branch_instructions() {
    use dingooemu_core::cpu::Cpu;
    use dingooemu_core::memory::Memory;

    let mut cpu = Cpu::new(0);
    let mut mem = Memory::new();

    // BEQ $zero, $zero, offset=1 (branch to PC+4+1*4 = 8)
    // opcode=0x04, rs=0, rt=0, offset=1
    let beq = (0x04 << 26) | 1;
    mem.write_u32(0, beq).unwrap();

    // NOP (delay slot)
    mem.write_u32(4, 0).unwrap();

    // ADDIU $t0, $zero, 0x1234 (target at address 8)
    let addiu = (0x09 << 26) | (8 << 16) | 0x1234;
    mem.write_u32(8, addiu).unwrap();

    cpu.start();

    // Execute BEQ (sets delay slot)
    cpu.step(&mut mem).unwrap();
    eprintln!(
        "Step 1 - After BEQ: PC={:#010x}, branch_delay={}, branch_target={:#010x}",
        cpu.regs.pc, cpu.branch_delay, cpu.branch_target
    );

    // Execute delay slot (NOP)
    cpu.step(&mut mem).unwrap();
    eprintln!(
        "Step 2 - After NOP: PC={:#010x}, branch_delay={}",
        cpu.regs.pc, cpu.branch_delay
    );

    // Execute target instruction
    cpu.step(&mut mem).unwrap();
    eprintln!(
        "Step 3 - After ADDIU: PC={:#010x}, $t0={:#010x}",
        cpu.regs.pc,
        cpu.regs.read(8)
    );

    assert_eq!(cpu.regs.read(8), 0x1234);
}

/// Test memory operations
#[test]
fn test_memory_operations() {
    use dingooemu_core::memory::Memory;

    let mut mem = Memory::new();

    // Test malloc
    let ptr = mem.malloc(100);
    assert!(ptr != 0);

    // Test write and read
    mem.write_u32(ptr, 0x1234_5678).unwrap();
    assert_eq!(mem.read_u32(ptr).unwrap(), 0x1234_5678);

    // Test free
    mem.free(ptr);
}
