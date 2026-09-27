use dingooemu_core::common::input::{
    BUTTON_A, BUTTON_DOWN, BUTTON_SELECT, BUTTON_START, BUTTON_UP,
};
use dingooemu_core::{Emulator, UnknownHlePolicy, UnknownInstructionPolicy};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

struct Session {
    emu: Emulator,
    samples: usize,
    nonzero_samples: usize,
    captures: Option<PathBuf>,
    name: String,
}

impl Session {
    fn new(path: &Path, saves: &Path, name: &str, jit: bool) -> Self {
        let mut emu = Emulator::from_path(path).unwrap();
        emu.set_save_directory(saves.to_path_buf());
        #[cfg(feature = "standalone")]
        emu.set_host_audio_output_enabled(false);
        emu.set_jit_enabled(jit);
        emu.set_unknown_hle_policy(UnknownHlePolicy::Stop);
        emu.set_unknown_instruction_policy(UnknownInstructionPolicy::Stop);
        emu.start();
        Self {
            emu,
            samples: 0,
            nonzero_samples: 0,
            captures: std::env::var_os("DINGOOEMU_DOOM_CAPTURES").map(PathBuf::from),
            name: name.into(),
        }
    }

    fn run(&mut self, buttons: u32, milliseconds: u64) {
        self.emu.set_buttons(buttons);
        let until = Instant::now() + Duration::from_millis(milliseconds);
        while Instant::now() < until && self.emu.is_running() {
            let start = Instant::now();
            self.emu.tick().unwrap();
            let samples = self.emu.take_audio_samples();
            self.samples += samples.len();
            self.nonzero_samples += samples.iter().filter(|&&sample| sample != 0).count();
            std::thread::sleep(Duration::from_millis(16).saturating_sub(start.elapsed()));
        }
    }

    fn boot(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            self.run(0, 100);
            let frame = self.emu.framebuffer();
            if frame
                .as_chunks::<2>()
                .0
                .iter()
                .any(|pixel| pixel != &frame[..2])
            {
                break;
            }
            assert!(
                self.emu.is_running() && Instant::now() < deadline,
                "Guest must reach its title screen"
            );
        }
        self.run(0, 1000);
    }

    fn press(&mut self, buttons: u32) {
        self.run(buttons, 350);
        self.run(0, 350);
    }

    fn capture(&self, label: &str) {
        if let Some(directory) = &self.captures {
            std::fs::create_dir_all(directory).unwrap();
            self.emu
                .save_screenshot(&directory.join(format!("{}-{label}.png", self.name)))
                .unwrap();
        }
        log::info!(
            "{} {label}: crc={:08x}",
            self.name,
            self.emu.framebuffer_crc32()
        );
    }
}

fn exercise(variable: &str, name: &str, confirmations: usize, quit_steps: usize) {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .filter_module("cranelift_jit::backend", log::LevelFilter::Warn)
        .is_test(true)
        .try_init();
    let path = PathBuf::from(
        std::env::var_os(variable).expect("Set the game package environment variable"),
    );
    let saves = std::env::temp_dir().join(format!("dingooemu-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&saves).unwrap();
    let jit = std::env::var_os("DINGOOEMU_DOOM_INTERPRETER").is_none();
    let mut session = Session::new(&path, &saves, name, jit);
    session.boot();
    session.capture("title");
    assert!(session.emu.is_running());
    session.press(BUTTON_START);
    session.capture("menu");
    for _ in 0..confirmations {
        session.press(BUTTON_A);
    }
    session.run(0, 1500);
    session.capture("level");
    session.run(BUTTON_UP, 700);
    session.run(0, 200);
    session.press(BUTTON_A);
    session.capture("moved-fired");
    session.press(BUTTON_SELECT);
    session.capture("automap");
    session.press(BUTTON_SELECT);
    session.press(BUTTON_START);
    for _ in 0..3 {
        session.press(BUTTON_DOWN);
    }
    session.press(BUTTON_A);
    session.capture("save-menu");
    session.press(BUTTON_A);
    session.run(0, 500);
    session.capture("saved");
    session.emu.flush_save_files();
    let save = [
        saves.join(format!("{name}sav1.dsg")),
        saves.join("a").join(format!("{name}sav0.dsg")),
        saves.join("a").join(format!("{name}sav1.dsg")),
    ]
    .into_iter()
    .find(|path| path.exists())
    .expect("The guest must actually write a save game");
    let data = std::fs::read(&save).unwrap();
    assert!(data.len() > 1000);
    assert!(data.starts_with(b"GAME SAVE 1") || data.starts_with(b"E1M1 SLOT 1"));
    assert_eq!(data.last(), Some(&0x1d));
    assert!(session.samples > 0);
    assert!(
        session.nonzero_samples > 0,
        "Menu and weapon sounds must produce PCM"
    );
    log::info!(
        "{name}: save_bytes={}, PCM_samples={}, nonzero_samples={}",
        data.len(),
        session.samples,
        session.nonzero_samples
    );
    for call in session.emu.unknown_hle_calls() {
        log::info!("unknown HLE: {} count={}", call.name, call.count);
    }
    session.emu.stop();
    // Load the guest-created save in a fresh runtime before checking normal exit.
    let mut loaded = Session::new(&path, &saves, name, jit);
    loaded.boot();
    loaded.press(BUTTON_START);
    for _ in 0..2 {
        loaded.press(BUTTON_DOWN);
    }
    loaded.press(BUTTON_A);
    loaded.capture("load-menu");
    loaded.press(BUTTON_A);
    loaded.run(0, 2500);
    loaded.capture("loaded");
    assert!(loaded.emu.is_running());
    // Saving a second slot proves the fresh runtime loaded an active level.
    loaded.press(BUTTON_START);
    loaded.press(BUTTON_DOWN);
    loaded.press(BUTTON_A);
    loaded.press(BUTTON_DOWN);
    loaded.press(BUTTON_A);
    loaded.run(0, 500);
    loaded.emu.flush_save_files();
    let second_save = save.with_file_name(
        if save.file_name().unwrap().to_string_lossy().contains("sav0") {
            format!("{name}sav1.dsg")
        } else {
            format!("{name}sav2.dsg")
        },
    );
    let second_data = std::fs::read(second_save).expect("Loaded level must support saving again");
    assert!(second_data.len() > 1000);
    assert_eq!(&second_data[40..43], &data[40..43]);
    assert_eq!(second_data.last(), Some(&0x1d));
    loaded.capture("resaved");
    loaded.press(BUTTON_START);
    for _ in 0..quit_steps {
        loaded.press(BUTTON_DOWN);
    }
    loaded.press(BUTTON_A);
    loaded.capture("quit-prompt");
    loaded.press(BUTTON_A);
    loaded.run(0, 500);
    assert!(
        !loaded.emu.is_running(),
        "The guest quit confirmation must terminate execution"
    );
    std::fs::remove_dir_all(saves).unwrap();
}

#[test]
#[ignore = "Requires a legally obtained DOOM-A330.cc package"]
fn doom1_a330_gameplay_save_load_and_quit() {
    exercise("DINGOOEMU_DOOM1_CC", "doom1", 3, 2);
}

#[test]
#[ignore = "Requires a legally obtained DOOM2-A330.cc package"]
fn doom2_a330_gameplay_save_load_and_quit() {
    exercise("DINGOOEMU_DOOM2_CC", "doom2", 2, 1);
}
