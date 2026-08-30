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

    fn average(self, total: u128) -> u128 {
        total.checked_div(u128::from(self.frames)).unwrap_or(0)
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
    audio_output_sample_rate_hz: u32,
    audio_buffer_status_callback_supported: Option<bool>,
    audio_buffer_status_callbacks: u64,
    audio_buffer_active_callbacks: u64,
    audio_buffer_occupancy_total: u64,
    audio_buffer_occupancy_min: Option<u32>,
    audio_buffer_occupancy_max: u32,
    audio_buffer_underrun_likely: u64,
    write_failed: bool,
}

impl DiagnosticSession {
    fn new(save_directory: Option<&Path>, content_path: &str, sample_rate_hz: u32) -> Self {
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
            audio_output_sample_rate_hz: sample_rate_hz,
            audio_buffer_status_callback_supported: None,
            audio_buffer_status_callbacks: 0,
            audio_buffer_active_callbacks: 0,
            audio_buffer_occupancy_total: 0,
            audio_buffer_occupancy_min: None,
            audio_buffer_occupancy_max: 0,
            audio_buffer_underrun_likely: 0,
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
        self.audio_buffer_status_callback_supported = None;
        self.audio_buffer_status_callbacks = 0;
        self.audio_buffer_active_callbacks = 0;
        self.audio_buffer_occupancy_total = 0;
        self.audio_buffer_occupancy_min = None;
        self.audio_buffer_occupancy_max = 0;
        self.audio_buffer_underrun_likely = 0;
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
format_version=5\n\
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
audio_output_sample_rate_hz={}\n\
audio_minimum_latency_ms=0\n\
audio_latency_request_status=not_requested\n\
audio_buffer_status_callback_status={}\n\
audio_buffer_status_callbacks={}\n\
audio_buffer_active_callbacks={}\n\
audio_buffer_occupancy_average_percent={}\n\
audio_buffer_occupancy_min_percent={}\n\
audio_buffer_occupancy_max_percent={}\n\
audio_buffer_underrun_likely={}\n\
async_audio_callback_status=unsupported\n\
async_audio_enabled=false\n\
async_audio_state_changes=0\n\
async_audio_callback_calls=0\n\
async_audio_real_frames=0\n\
async_audio_output_frames_requested=0\n\
async_audio_output_frames_accepted=0\n\
async_audio_dropped_frames=0\n\
async_audio_max_queued_frames=0\n\
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
            self.audio_output_sample_rate_hz,
            status(self.audio_buffer_status_callback_supported),
            self.audio_buffer_status_callbacks,
            self.audio_buffer_active_callbacks,
            average(
                self.audio_buffer_occupancy_total,
                self.audio_buffer_status_callbacks,
            ),
            self.audio_buffer_occupancy_min.unwrap_or(0),
            self.audio_buffer_occupancy_max,
            self.audio_buffer_underrun_likely,
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

fn status(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "accepted",
        Some(false) => "rejected",
        None => "pending",
    }
}

fn average(total: u64, count: u64) -> u64 {
    total.checked_div(count).unwrap_or(0)
}

pub fn configure(save_directory: Option<&Path>, content_path: &str, sample_rate_hz: u32) {
    ENABLED.store(false, Ordering::Relaxed);
    *SESSION.lock().unwrap() = Some(DiagnosticSession::new(
        save_directory,
        content_path,
        sample_rate_hz,
    ));
}

pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn set_audio_buffer_status_callback_status(supported: bool) {
    if let Some(session) = SESSION.lock().unwrap().as_mut() {
        session.audio_buffer_status_callback_supported = Some(supported);
    }
}

pub fn record_audio_buffer_status(active: bool, occupancy: u32, underrun_likely: bool) {
    if !is_enabled() {
        return;
    }
    if let Some(session) = SESSION.lock().unwrap().as_mut() {
        session.audio_buffer_status_callbacks =
            session.audio_buffer_status_callbacks.saturating_add(1);
        session.audio_buffer_active_callbacks = session
            .audio_buffer_active_callbacks
            .saturating_add(u64::from(active));
        session.audio_buffer_occupancy_total = session
            .audio_buffer_occupancy_total
            .saturating_add(u64::from(occupancy));
        session.audio_buffer_occupancy_min = Some(
            session
                .audio_buffer_occupancy_min
                .map_or(occupancy, |current| current.min(occupancy)),
        );
        session.audio_buffer_occupancy_max = session.audio_buffer_occupancy_max.max(occupancy);
        session.audio_buffer_underrun_likely = session
            .audio_buffer_underrun_likely
            .saturating_add(u64::from(underrun_likely));
    }
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
    is_enabled().then(Instant::now)
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
    fn diagnostics_are_dormant_by_default() {
        ENABLED.store(false, Ordering::Relaxed);
        assert!(!is_enabled());
        assert!(frame_timer().is_none());
    }

    #[test]
    fn report_contains_the_complete_comparison_schema() {
        let directory = std::env::temp_dir().join(format!(
            "dingooemu-diagnostic-schema-test-{}",
            std::process::id()
        ));
        let mut session = DiagnosticSession::new(Some(&directory), "games/test.app", 22_050);
        session.enabled = true;
        session.audio_buffer_status_callback_supported = Some(true);
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
        for expected in [
            "format_version=5",
            "content=test.app",
            "frames=1",
            "tick_max_us=12345",
            "run_max_us=23456",
            "video_max_us=3456",
            "audio_max_us=7655",
            "recent_tick_average_us=12345",
            "audio_frames_requested=368",
            "audio_frames_accepted=300",
            "audio_short_writes=1",
            "audio_output_sample_rate_hz=22050",
            "audio_minimum_latency_ms=0",
            "audio_latency_request_status=not_requested",
            "audio_buffer_status_callback_status=accepted",
            "async_audio_callback_status=unsupported",
            "async_audio_enabled=false",
            "jit_native_executions=7",
            "jit_zero_exit_fallbacks=0",
        ] {
            assert!(
                report.contains(expected),
                "missing report field: {expected}"
            );
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}
