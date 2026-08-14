use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use dingooemu_core::{Emulator, JitDiagnostics};

const DIAGNOSTIC_FILE_NAME: &str = "dingooemu-diagnostic.txt";
const REPORT_INTERVAL_FRAMES: u64 = 60;

#[derive(Clone, Copy, Default)]
struct FrameTiming {
    frames: u64,
    tick_total_us: u128,
    tick_max_us: u128,
    run_total_us: u128,
    run_max_us: u128,
    video_total_us: u128,
    video_max_us: u128,
    audio_total_us: u128,
    audio_max_us: u128,
}

impl FrameTiming {
    fn record(
        &mut self,
        tick_elapsed: Duration,
        run_elapsed: Duration,
        video_elapsed: Duration,
        audio_elapsed: Duration,
    ) {
        let tick_us = tick_elapsed.as_micros();
        let run_us = run_elapsed.as_micros();
        let video_us = video_elapsed.as_micros();
        let audio_us = audio_elapsed.as_micros();
        self.frames = self.frames.saturating_add(1);
        self.tick_total_us = self.tick_total_us.saturating_add(tick_us);
        self.tick_max_us = self.tick_max_us.max(tick_us);
        self.run_total_us = self.run_total_us.saturating_add(run_us);
        self.run_max_us = self.run_max_us.max(run_us);
        self.video_total_us = self.video_total_us.saturating_add(video_us);
        self.video_max_us = self.video_max_us.max(video_us);
        self.audio_total_us = self.audio_total_us.saturating_add(audio_us);
        self.audio_max_us = self.audio_max_us.max(audio_us);
    }

    fn average(&self, total: u128) -> u128 {
        if self.frames == 0 {
            0
        } else {
            total / u128::from(self.frames)
        }
    }
}

struct DiagnosticSession {
    path: Option<PathBuf>,
    content_name: String,
    enabled: bool,
    started: Instant,
    total: FrameTiming,
    recent: FrameTiming,
    audio_frames_requested: u64,
    audio_frames_accepted: u64,
    audio_short_writes: u64,
    write_failed: bool,
}

impl DiagnosticSession {
    fn new(save_directory: Option<&Path>, content_path: &str) -> Self {
        let content_name = Path::new(content_path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "unknown.app".to_string());
        Self {
            path: save_directory.map(|directory| directory.join(DIAGNOSTIC_FILE_NAME)),
            content_name,
            enabled: false,
            started: Instant::now(),
            total: FrameTiming::default(),
            recent: FrameTiming::default(),
            audio_frames_requested: 0,
            audio_frames_accepted: 0,
            audio_short_writes: 0,
            write_failed: false,
        }
    }

    fn reset(&mut self) {
        self.started = Instant::now();
        self.total = FrameTiming::default();
        self.recent = FrameTiming::default();
        self.audio_frames_requested = 0;
        self.audio_frames_accepted = 0;
        self.audio_short_writes = 0;
        self.write_failed = false;
    }

    fn record_frame(
        &mut self,
        tick_elapsed: Duration,
        run_elapsed: Duration,
        video_elapsed: Duration,
        audio_elapsed: Duration,
        audio_frames_requested: usize,
        audio_frames_accepted: usize,
    ) {
        if self.recent.frames >= REPORT_INTERVAL_FRAMES {
            self.recent = FrameTiming::default();
        }
        self.total
            .record(tick_elapsed, run_elapsed, video_elapsed, audio_elapsed);
        self.recent
            .record(tick_elapsed, run_elapsed, video_elapsed, audio_elapsed);
        self.audio_frames_requested = self
            .audio_frames_requested
            .saturating_add(audio_frames_requested as u64);
        self.audio_frames_accepted = self
            .audio_frames_accepted
            .saturating_add(audio_frames_accepted as u64);
        if audio_frames_accepted < audio_frames_requested {
            self.audio_short_writes = self.audio_short_writes.saturating_add(1);
        }
    }

    fn report(&self, jit: JitDiagnostics) -> String {
        let total = self.total;
        let recent = self.recent;
        format!(
            "DingooEmu performance diagnostics\n\
format_version=3\n\
core_version={}\n\
target_os={}\n\
target_arch={}\n\
pointer_width={}\n\
content={}\n\
elapsed_ms={}\n\
frames={}\n\
tick_average_us={}\n\
tick_max_us={}\n\
run_average_us={}\n\
run_max_us={}\n\
video_average_us={}\n\
video_max_us={}\n\
audio_average_us={}\n\
audio_max_us={}\n\
recent_frames={}\n\
recent_tick_average_us={}\n\
recent_tick_max_us={}\n\
recent_run_average_us={}\n\
recent_run_max_us={}\n\
recent_video_average_us={}\n\
recent_video_max_us={}\n\
recent_audio_average_us={}\n\
recent_audio_max_us={}\n\
audio_frames_requested={}\n\
audio_frames_accepted={}\n\
audio_short_writes={}\n\
jit_feature_available={}\n\
jit_enabled={}\n\
jit_backend_available={}\n\
jit_tracked_blocks={}\n\
jit_compiled_blocks={}\n\
jit_failed_blocks={}\n\
jit_execute_requests={}\n\
jit_native_executions={}\n\
jit_native_instructions={}\n\
jit_interpreter_executions={}\n\
jit_interpreter_instructions={}\n\
jit_compilation_attempts={}\n\
jit_compilation_failures={}\n\
jit_compilation_total_us={}\n\
jit_compilation_max_us={}\n\
jit_cold_fallbacks={}\n\
jit_instruction_limit_fallbacks={}\n\
jit_zero_exit_fallbacks={}\n",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH,
            usize::BITS,
            self.content_name,
            self.started.elapsed().as_millis(),
            total.frames,
            total.average(total.tick_total_us),
            total.tick_max_us,
            total.average(total.run_total_us),
            total.run_max_us,
            total.average(total.video_total_us),
            total.video_max_us,
            total.average(total.audio_total_us),
            total.audio_max_us,
            recent.frames,
            recent.average(recent.tick_total_us),
            recent.tick_max_us,
            recent.average(recent.run_total_us),
            recent.run_max_us,
            recent.average(recent.video_total_us),
            recent.video_max_us,
            recent.average(recent.audio_total_us),
            recent.audio_max_us,
            self.audio_frames_requested,
            self.audio_frames_accepted,
            self.audio_short_writes,
            jit.feature_available,
            jit.enabled,
            jit.backend_available,
            jit.tracked_blocks,
            jit.compiled_blocks,
            jit.failed_blocks,
            jit.execute_requests,
            jit.native_executions,
            jit.native_instructions,
            jit.interpreter_executions,
            jit.interpreter_instructions,
            jit.compilation_attempts,
            jit.compilation_failures,
            jit.compilation_total_us,
            jit.compilation_max_us,
            jit.cold_fallbacks,
            jit.instruction_limit_fallbacks,
            jit.zero_exit_fallbacks,
        )
    }

    fn write_report(&mut self, jit: JitDiagnostics) {
        let Some(path) = self.path.as_ref() else {
            return;
        };
        if let Some(parent) = path.parent() {
            if let Err(error) = std::fs::create_dir_all(parent) {
                self.report_write_error(error);
                return;
            }
        }
        if let Err(error) = std::fs::write(path, self.report(jit)) {
            self.report_write_error(error);
        }
    }

    fn report_write_error(&mut self, error: std::io::Error) {
        if !self.write_failed {
            log::warn!("Unable to write performance diagnostics: {error}");
            self.write_failed = true;
        }
    }
}

static ENABLED: AtomicBool = AtomicBool::new(false);
static SESSION: Mutex<Option<DiagnosticSession>> = Mutex::new(None);

pub fn configure(save_directory: Option<&Path>, content_path: &str) {
    ENABLED.store(false, Ordering::Relaxed);
    *SESSION.lock().unwrap() = Some(DiagnosticSession::new(save_directory, content_path));
}

pub fn set_enabled(enabled: bool, emulator: &Emulator) {
    let mut session = SESSION.lock().unwrap();
    let Some(session) = session.as_mut() else {
        ENABLED.store(false, Ordering::Relaxed);
        return;
    };
    if enabled == session.enabled {
        return;
    }
    if enabled {
        let Some(path) = session.path.clone() else {
            ENABLED.store(false, Ordering::Relaxed);
            log::warn!("Performance diagnostics require a frontend save directory");
            return;
        };
        session.reset();
        session.enabled = true;
        session.write_report(emulator.jit_diagnostics());
        ENABLED.store(true, Ordering::Relaxed);
        log::info!("Performance diagnostics enabled: {}", path.display());
    } else {
        ENABLED.store(false, Ordering::Relaxed);
        session.write_report(emulator.jit_diagnostics());
        session.enabled = false;
    }
}

pub fn frame_timer() -> Option<Instant> {
    ENABLED.load(Ordering::Relaxed).then(Instant::now)
}

pub fn record_frame(
    emulator: &Emulator,
    tick_elapsed: Duration,
    run_elapsed: Duration,
    video_elapsed: Duration,
    audio_elapsed: Duration,
    audio_frames_requested: usize,
    audio_frames_accepted: usize,
) {
    let mut session = SESSION.lock().unwrap();
    let Some(session) = session.as_mut().filter(|session| session.enabled) else {
        return;
    };
    session.record_frame(
        tick_elapsed,
        run_elapsed,
        video_elapsed,
        audio_elapsed,
        audio_frames_requested,
        audio_frames_accepted,
    );
    if session.total.frames % REPORT_INTERVAL_FRAMES == 0 {
        session.write_report(emulator.jit_diagnostics());
    }
}

pub fn finish(emulator: Option<&Emulator>) {
    ENABLED.store(false, Ordering::Relaxed);
    let Some(mut session) = SESSION.lock().unwrap().take() else {
        return;
    };
    if session.enabled {
        session
            .write_report(emulator.map_or_else(JitDiagnostics::default, Emulator::jit_diagnostics));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_is_written_as_one_shareable_text_file() {
        let directory =
            std::env::temp_dir().join(format!("dingooemu-diagnostic-test-{}", std::process::id()));
        let mut session = DiagnosticSession::new(Some(&directory), "games/test.app");
        session.enabled = true;
        session.record_frame(
            Duration::from_micros(12_345),
            Duration::from_micros(23_456),
            Duration::from_micros(3_456),
            Duration::from_micros(7_655),
            368,
            300,
        );
        session.write_report(JitDiagnostics {
            feature_available: true,
            enabled: true,
            backend_available: true,
            native_executions: 7,
            ..JitDiagnostics::default()
        });

        let report = std::fs::read_to_string(directory.join(DIAGNOSTIC_FILE_NAME)).unwrap();
        assert!(report.contains("format_version=3"));
        assert!(report.contains("content=test.app"));
        assert!(report.contains("frames=1"));
        assert!(report.contains("tick_max_us=12345"));
        assert!(report.contains("run_max_us=23456"));
        assert!(report.contains("video_max_us=3456"));
        assert!(report.contains("audio_max_us=7655"));
        assert!(report.contains("recent_tick_average_us=12345"));
        assert!(report.contains("audio_frames_requested=368"));
        assert!(report.contains("audio_frames_accepted=300"));
        assert!(report.contains("audio_short_writes=1"));
        assert!(report.contains("jit_native_executions=7"));
        std::fs::remove_dir_all(directory).unwrap();
    }
}
