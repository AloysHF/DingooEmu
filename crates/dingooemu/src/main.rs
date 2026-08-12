mod gamepad_overlay;
mod keyboard;
mod scaler;

use clap::{Parser, ValueEnum};
use dingooemu_core::cheats::CheatRule;
use dingooemu_core::cpu::UnknownInstructionPolicy;
use dingooemu_core::{video::SCREEN_HEIGHT, video::SCREEN_WIDTH, Emulator, UnknownHlePolicy};
use minifb::{Key, Window, WindowOptions};
use std::path::{Path, PathBuf};

use keyboard::{KeyboardMapper, RemapSpec};
use scaler::{DisplayScaler, ScaleFilter};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
enum UnknownInstructionMode {
    Stop,
    #[default]
    Skip,
}

impl From<UnknownInstructionMode> for UnknownInstructionPolicy {
    fn from(mode: UnknownInstructionMode) -> Self {
        match mode {
            UnknownInstructionMode::Stop => Self::Stop,
            UnknownInstructionMode::Skip => Self::Skip,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
enum UnknownHleMode {
    #[default]
    Report,
    Stop,
}

impl UnknownHleMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Report => "report",
            Self::Stop => "stop",
        }
    }
}

impl From<UnknownHleMode> for UnknownHlePolicy {
    fn from(mode: UnknownHleMode) -> Self {
        match mode {
            UnknownHleMode::Report => Self::Report,
            UnknownHleMode::Stop => Self::Stop,
        }
    }
}

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

    /// Remap a Dingoo button using BUTTON:KEY syntax
    #[arg(long = "remap", value_name = "BUTTON:KEY")]
    remappings: Vec<RemapSpec>,

    /// Swap the emulated A and B buttons
    #[arg(long = "swap-ab")]
    swap_ab: bool,

    /// Pixel scaling filter for display output
    #[arg(long, value_enum, default_value_t = ScaleFilter::Nearest)]
    filter: ScaleFilter,

    /// Show the current Dingoo button state over the game frame
    #[arg(long)]
    show_gamepad: bool,

    /// Frames before a held button starts repeating
    #[arg(long = "repeat-delay", default_value_t = 24)]
    repeat_delay: u32,

    /// Frames between repeated button presses
    #[arg(long = "repeat-period", default_value_t = 6, value_parser = clap::value_parser!(u32).range(1..))]
    repeat_period: u32,

    /// Freeze a memory address or MIPS register using TARGET=VALUE syntax
    #[arg(long = "cheat", value_name = "RULE")]
    cheats: Vec<CheatRule>,

    /// Behavior when an unknown MIPS instruction is encountered
    #[arg(long, value_enum, default_value_t = UnknownInstructionMode::Skip)]
    unknown_instruction_policy: UnknownInstructionMode,

    /// Behavior when an unknown SDK HLE function is called
    #[arg(long, value_enum, default_value_t = UnknownHleMode::Report)]
    unknown_hle_policy: UnknownHleMode,

    /// Allow an exact unknown SDK function name in strict HLE mode
    #[arg(long = "allow-unknown-hle", value_name = "NAME")]
    allowed_unknown_hle: Vec<String>,

    /// Write aggregated unknown SDK HLE diagnostics as JSON
    #[arg(long = "hle-report", value_name = "PATH")]
    hle_report: Option<PathBuf>,

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
    emu.cpu
        .set_unknown_instruction_policy(args.unknown_instruction_policy.into());
    emu.set_unknown_hle_policy(args.unknown_hle_policy.into());
    emu.set_unknown_hle_allowlist(args.allowed_unknown_hle.iter().cloned());
    emu.audio.set_master_volume(args.volume);
    emu.input
        .set_repeat_timing(args.repeat_delay, args.repeat_period);
    for (index, cheat) in args.cheats.iter().cloned().enumerate() {
        emu.set_parsed_cheat(index as u32, true, cheat)?;
    }

    emu.audio
        .set_host_output_enabled(args.screenshot.is_none() && !args.headless);

    emu.start();

    let emulation_result = run_emulation(&args, &mut emu);
    log_unknown_hle_summary(&emu);
    let report_result = write_unknown_hle_report(&args, &emu);
    if let Err(error) = &report_result {
        log::error!("Failed to write HLE diagnostics: {error}");
    }
    emulation_result?;
    report_result?;
    Ok(())
}

fn run_emulation(args: &Args, emu: &mut Emulator) -> anyhow::Result<()> {
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
                scale_mode: minifb::ScaleMode::Stretch,
                ..WindowOptions::default()
            },
        )?;

        if args.fullscreen {
            window.topmost(true);
            window.set_position(0, 0);
        }

        // Limit to ~60fps
        window.set_target_fps(60);
        let keyboard = KeyboardMapper::new(&args.remappings, args.swap_ab);
        let mut display_scaler = DisplayScaler::new(args.filter);

        // Main loop
        while window.is_open() && !window.is_key_down(Key::Escape) {
            // Poll input
            let buttons = keyboard.pressed_buttons(&window);
            emu.set_buttons(buttons);

            // Run one frame
            emu.tick()?;

            // Get framebuffer and convert to XRGB8888
            let mut buffer = emu.video.to_xrgb8888();
            if args.show_gamepad {
                gamepad_overlay::draw(
                    &mut buffer,
                    SCREEN_WIDTH as usize,
                    SCREEN_HEIGHT as usize,
                    buttons,
                );
            }
            let (window_width, window_height) = window.get_size();
            let output = display_scaler.render(
                &buffer,
                SCREEN_WIDTH as usize,
                SCREEN_HEIGHT as usize,
                window_width,
                window_height,
            );

            // Update window
            window.update_with_buffer(output, window_width.max(1), window_height.max(1))?;
        }
    }

    Ok(())
}

fn log_unknown_hle_summary(emu: &Emulator) {
    let calls: Vec<_> = emu.unknown_hle_calls().collect();
    if calls.is_empty() {
        log::info!("Unknown SDK HLE summary: none");
        return;
    }

    log::warn!(
        "Unknown SDK HLE summary: {} function(s); compatibility return values may hide missing behavior",
        calls.len()
    );
    for call in calls {
        log::warn!(
            "  {}: count={}, first_pc={:#010x}, import={:#010x}, arguments={:#010x?}",
            call.name,
            call.count,
            call.first_pc,
            call.import_address,
            call.first_arguments
        );
    }
}

fn unknown_hle_report(args: &Args, emu: &Emulator) -> serde_json::Value {
    let content = Path::new(&args.path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(&args.path);
    let unknown_hle: Vec<_> = emu.unknown_hle_calls().collect();
    serde_json::json!({
        "schema_version": 1,
        "content": content,
        "policy": args.unknown_hle_policy.as_str(),
        "allowlist": args.allowed_unknown_hle,
        "unknown_hle": unknown_hle,
    })
}

fn write_unknown_hle_report(args: &Args, emu: &Emulator) -> anyhow::Result<()> {
    let Some(path) = args.hle_report.as_ref() else {
        return Ok(());
    };
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let output = serde_json::to_vec_pretty(&unknown_hle_report(args, emu))?;
    std::fs::write(path, output)?;
    log::info!("HLE diagnostics saved to: {}", path.display());
    Ok(())
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

    #[test]
    fn repeated_remap_options_are_parsed() {
        let args = Args::try_parse_from([
            "dingoo-emu",
            "--remap",
            "a:space",
            "--remap",
            "select:tab",
            "game.app",
        ])
        .unwrap();
        assert_eq!(args.remappings.len(), 2);
    }

    #[test]
    fn swap_ab_flag_is_parsed() {
        assert!(
            Args::try_parse_from(["dingoo-emu", "--swap-ab", "game.app"])
                .unwrap()
                .swap_ab
        );
    }

    #[test]
    fn every_display_filter_is_parsed() {
        for (name, expected) in [
            ("nearest", ScaleFilter::Nearest),
            ("bilinear", ScaleFilter::Bilinear),
            ("bicubic", ScaleFilter::Bicubic),
            ("xbrz", ScaleFilter::Xbrz),
        ] {
            assert_eq!(
                Args::try_parse_from(["dingoo-emu", "--filter", name, "game.app"])
                    .unwrap()
                    .filter,
                expected
            );
        }
    }

    #[test]
    fn show_gamepad_flag_is_parsed() {
        assert!(
            Args::try_parse_from(["dingoo-emu", "--show-gamepad", "game.app"])
                .unwrap()
                .show_gamepad
        );
    }

    #[test]
    fn repeat_timing_is_configurable_and_period_rejects_zero() {
        let args = Args::try_parse_from([
            "dingoo-emu",
            "--repeat-delay",
            "10",
            "--repeat-period",
            "2",
            "game.app",
        ])
        .unwrap();
        assert_eq!((args.repeat_delay, args.repeat_period), (10, 2));
        assert!(Args::try_parse_from(["dingoo-emu", "--repeat-period", "0", "game.app"]).is_err());
    }

    #[test]
    fn repeated_cheat_rules_are_parsed() {
        let args = Args::try_parse_from([
            "dingoo-emu",
            "--cheat",
            "mem8:0x100=1",
            "--cheat",
            "reg:r4=7",
            "game.app",
        ])
        .unwrap();
        assert_eq!(args.cheats.len(), 2);
    }

    #[test]
    fn unknown_instruction_policy_accepts_stop_and_skip() {
        for (name, expected) in [
            ("stop", UnknownInstructionMode::Stop),
            ("skip", UnknownInstructionMode::Skip),
        ] {
            assert_eq!(
                Args::try_parse_from([
                    "dingoo-emu",
                    "--unknown-instruction-policy",
                    name,
                    "game.app",
                ])
                .unwrap()
                .unknown_instruction_policy,
                expected
            );
        }
    }

    #[test]
    fn unknown_hle_options_default_to_reporting_and_accept_strict_allowlists() {
        let defaults = Args::try_parse_from(["dingoo-emu", "game.app"]).unwrap();
        assert_eq!(defaults.unknown_hle_policy, UnknownHleMode::Report);
        assert!(defaults.allowed_unknown_hle.is_empty());
        assert!(defaults.hle_report.is_none());

        let strict = Args::try_parse_from([
            "dingoo-emu",
            "--unknown-hle-policy",
            "stop",
            "--allow-unknown-hle",
            "legacy_one",
            "--allow-unknown-hle",
            "legacy_two",
            "--hle-report",
            "report.json",
            "game.app",
        ])
        .unwrap();
        assert_eq!(strict.unknown_hle_policy, UnknownHleMode::Stop);
        assert_eq!(strict.allowed_unknown_hle, ["legacy_one", "legacy_two"]);
        assert_eq!(strict.hle_report, Some(PathBuf::from("report.json")));
    }

    #[test]
    fn empty_unknown_hle_report_has_a_stable_schema() {
        let args = Args::try_parse_from([
            "dingoo-emu",
            "--hle-report",
            "report.json",
            "folder/game.app",
        ])
        .unwrap();
        assert_eq!(
            unknown_hle_report(&args, &Emulator::default()),
            serde_json::json!({
                "schema_version": 1,
                "content": "game.app",
                "policy": "report",
                "allowlist": [],
                "unknown_hle": [],
            })
        );
    }
}
