use clap::Parser;
use dingooemu_core::{video::SCREEN_HEIGHT, video::SCREEN_WIDTH, Emulator};
use minifb::{Key, Window, WindowOptions};
use std::path::PathBuf;

#[cfg(target_os = "windows")]
mod screen {
    unsafe extern "system" {
        fn GetSystemMetrics(index: i32) -> i32;
    }

    pub fn size() -> (usize, usize) {
        unsafe { (GetSystemMetrics(0) as usize, GetSystemMetrics(1) as usize) }
    }
}

#[cfg(target_os = "linux")]
mod screen {
    type Display = *mut core::ffi::c_void;

    #[link(name = "X11")]
    unsafe extern "system" {
        fn XOpenDisplay(name: *const u8) -> Display;
        fn XCloseDisplay(display: Display) -> i32;
        fn XDisplayWidth(display: Display, screen: i32) -> i32;
        fn XDisplayHeight(display: Display, screen: i32) -> i32;
    }

    pub fn size() -> (usize, usize) {
        unsafe {
            let display = XOpenDisplay(std::ptr::null());
            if display.is_null() {
                return (800, 600);
            }
            let size = (
                XDisplayWidth(display, 0) as usize,
                XDisplayHeight(display, 0) as usize,
            );
            let _ = XCloseDisplay(display);
            size
        }
    }
}

#[cfg(target_os = "macos")]
mod screen {
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGMainDisplayID() -> u32;
        fn CGDisplayPixelsWide(display: u32) -> usize;
        fn CGDisplayPixelsHigh(display: u32) -> usize;
    }

    pub fn size() -> (usize, usize) {
        unsafe {
            let display = CGMainDisplayID();
            (CGDisplayPixelsWide(display), CGDisplayPixelsHigh(display))
        }
    }
}

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
    #[arg(
        short,
        long,
        default_value_t = 2,
        value_parser = clap::value_parser!(u32).range(1..=16)
    )]
    scale: u32,

    /// Run in fullscreen mode
    #[arg(short, long)]
    fullscreen: bool,

    /// Master audio volume (0-100)
    #[arg(short, long, default_value_t = 100, value_parser = clap::value_parser!(u8).range(0..=100))]
    volume: u8,

    /// Enable emulator debug logging
    #[arg(long)]
    debug_logging: bool,

    /// Run in headless mode (no window)
    #[arg(long)]
    headless: bool,

    /// Number of frames to run in headless mode
    #[arg(long, default_value_t = 300)]
    frames: u32,

    /// Take a screenshot after N frames and exit (saves as PNG)
    #[arg(short = 'S', long = "screenshot", value_name = "PATH")]
    screenshot: Option<PathBuf>,

    /// Number of frames to run before taking screenshot (default: 30)
    #[arg(long = "screenshot-frames", default_value = "30")]
    screenshot_frames: u32,
}

fn main() -> anyhow::Result<()> {
    // Parse command line arguments
    let args = Args::parse();

    let default_log_filter = if args.debug_logging { "debug" } else { "info" };
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(default_log_filter))
        .format_timestamp_millis()
        .init();

    // Load the game
    log::info!("Loading game: {}", args.path);
    let mut emu = Emulator::from_path(&args.path)?;
    emu.audio.set_master_volume(args.volume);

    // Start emulation
    emu.start();

    // Screenshot mode: run headless for N frames, save PNG, and exit
    if let Some(ref screenshot_path) = args.screenshot {
        for frame in 0..args.screenshot_frames {
            emu.tick()?;
            if frame % 60 == 0 {
                log::info!("Frame {}", frame);
            }
        }
        emu.video.save_screenshot(screenshot_path)?;
        log::info!("Screenshot saved to: {}", screenshot_path.display());
        return Ok(());
    }

    if args.headless {
        // Headless mode: run for the requested number of frames
        log::info!("Running in headless mode");
        for frame in 0..args.frames {
            emu.tick()?;
            if frame % 60 == 0 {
                log::info!("Frame {}", frame);
            }
        }
        log::info!("Headless run complete: {} frames", args.frames);
    } else {
        // Windowed mode
        let (width, height) = if args.fullscreen {
            screen::size()
        } else {
            (
                (SCREEN_WIDTH * args.scale) as usize,
                (SCREEN_HEIGHT * args.scale) as usize,
            )
        };

        let mut window = Window::new(
            "Dingoo A320 Emulator",
            width,
            height,
            WindowOptions {
                resize: !args.fullscreen,
                borderless: args.fullscreen,
                scale_mode: minifb::ScaleMode::AspectRatioStretch,
                ..WindowOptions::default()
            },
        )?;

        if args.fullscreen {
            window.topmost(true);
            window.set_position(0, 0);
        }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_accepts_supported_range() {
        assert_eq!(
            Args::try_parse_from(["dingoo-emu", "--scale", "1", "game.app"])
                .unwrap()
                .scale,
            1
        );
        assert_eq!(
            Args::try_parse_from(["dingoo-emu", "--scale", "16", "game.app"])
                .unwrap()
                .scale,
            16
        );
    }

    #[test]
    fn scale_rejects_zero_and_excessive_values() {
        assert!(Args::try_parse_from(["dingoo-emu", "--scale", "0", "game.app"]).is_err());
        assert!(Args::try_parse_from(["dingoo-emu", "--scale", "17", "game.app"]).is_err());
    }

    #[test]
    fn headless_frame_count_defaults_to_300_and_is_configurable() {
        assert_eq!(
            Args::try_parse_from(["dingoo-emu", "game.app"])
                .unwrap()
                .frames,
            300
        );
        assert_eq!(
            Args::try_parse_from(["dingoo-emu", "--headless", "--frames", "12", "game.app"])
                .unwrap()
                .frames,
            12
        );
    }

    #[test]
    fn fullscreen_flag_is_parsed() {
        assert!(
            Args::try_parse_from(["dingoo-emu", "--fullscreen", "game.app"])
                .unwrap()
                .fullscreen
        );
    }

    #[test]
    fn volume_accepts_percent_range() {
        assert_eq!(
            Args::try_parse_from(["dingoo-emu", "game.app"])
                .unwrap()
                .volume,
            100
        );
        assert_eq!(
            Args::try_parse_from(["dingoo-emu", "--volume", "0", "game.app"])
                .unwrap()
                .volume,
            0
        );
        assert!(Args::try_parse_from(["dingoo-emu", "--volume", "101", "game.app"]).is_err());
    }

    #[test]
    fn debug_logging_flag_is_parsed() {
        assert!(
            Args::try_parse_from(["dingoo-emu", "--debug-logging", "game.app"])
                .unwrap()
                .debug_logging
        );
    }
}
