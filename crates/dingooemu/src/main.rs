use clap::Parser;
use dingooemu_core::{video::SCREEN_HEIGHT, video::SCREEN_WIDTH, Emulator};
use minifb::{Key, Window, WindowOptions};

/// Dingoo A320 Emulator
#[derive(Parser, Debug)]
#[command(
    name = "dingoo-emu",
    version,
    about = "A Dingoo A320 emulator written in Rust"
)]
struct Args {
    /// Path to the .app game file
    path: String,

    /// Window scale factor
    #[arg(short, long, default_value_t = 2)]
    scale: u32,

    /// Run in headless mode (no window)
    #[arg(long)]
    headless: bool,
}

fn main() -> anyhow::Result<()> {
    // Initialize logger
    env_logger::init();

    // Parse command line arguments
    let args = Args::parse();

    // Load the game
    log::info!("Loading game: {}", args.path);
    let mut emu = Emulator::from_path(&args.path)?;

    // Start emulation
    emu.start();

    if args.headless {
        // Headless mode: run for a fixed number of frames
        log::info!("Running in headless mode");
        for frame in 0..300 {
            emu.tick()?;
            if frame % 60 == 0 {
                log::info!("Frame {}", frame);
            }
        }
        log::info!("Headless run complete");
    } else {
        // Windowed mode
        let width = (SCREEN_WIDTH * args.scale) as usize;
        let height = (SCREEN_HEIGHT * args.scale) as usize;

        let mut window = Window::new(
            "Dingoo A320 Emulator",
            width,
            height,
            WindowOptions {
                resize: false,
                scale_mode: minifb::ScaleMode::AspectRatioStretch,
                ..WindowOptions::default()
            },
        )?;

        // Limit to ~60fps
        window.set_target_fps(60);

        // Main loop
        while window.is_open() && !window.is_key_down(Key::Escape) {
            // Poll input
            let buttons = poll_input(&window);
            emu.set_buttons(buttons);

            // Run one frame
            emu.tick()?;

            // Get framebuffer and convert to XRGB8888
            let buffer = emu.video.to_xrgb8888();

            // Update window
            window.update_with_buffer(&buffer, SCREEN_WIDTH as usize, SCREEN_HEIGHT as usize)?;
        }
    }

    Ok(())
}

/// Poll keyboard input and convert to Dingoo button mask
fn poll_input(window: &Window) -> u32 {
    let mut buttons = 0u32;

    if window.is_key_down(Key::Up) || window.is_key_down(Key::W) {
        buttons |= dingooemu_core::input::BUTTON_UP;
    }
    if window.is_key_down(Key::Down) || window.is_key_down(Key::S) {
        buttons |= dingooemu_core::input::BUTTON_DOWN;
    }
    if window.is_key_down(Key::Left) || window.is_key_down(Key::A) {
        buttons |= dingooemu_core::input::BUTTON_LEFT;
    }
    if window.is_key_down(Key::Right) || window.is_key_down(Key::D) {
        buttons |= dingooemu_core::input::BUTTON_RIGHT;
    }
    if window.is_key_down(Key::L) {
        buttons |= dingooemu_core::input::BUTTON_A;
    }
    if window.is_key_down(Key::K) {
        buttons |= dingooemu_core::input::BUTTON_B;
    }
    if window.is_key_down(Key::I) {
        buttons |= dingooemu_core::input::BUTTON_X;
    }
    if window.is_key_down(Key::J) {
        buttons |= dingooemu_core::input::BUTTON_Y;
    }
    if window.is_key_down(Key::Key1) || window.is_key_down(Key::Q) {
        buttons |= dingooemu_core::input::BUTTON_SELECT;
    }
    if window.is_key_down(Key::Key0) || window.is_key_down(Key::O) {
        buttons |= dingooemu_core::input::BUTTON_START;
    }
    if window.is_key_down(Key::LeftShift) {
        buttons |= dingooemu_core::input::BUTTON_L;
    }
    if window.is_key_down(Key::RightShift) {
        buttons |= dingooemu_core::input::BUTTON_R;
    }

    buttons
}
