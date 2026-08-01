//! Dingoo PCM audio handling.

#[cfg(not(feature = "standalone"))]
use std::collections::VecDeque;
#[cfg(feature = "standalone")]
use std::num::NonZero;

pub const OUTPUT_SAMPLE_RATE: u32 = 22_050;

#[cfg(not(feature = "standalone"))]
const VIDEO_FRAMES_PER_SECOND: u32 = 60;
#[cfg(not(feature = "standalone"))]
const MAX_QUEUED_AUDIO_FRAMES: usize = OUTPUT_SAMPLE_RATE as usize / 2;
#[cfg(feature = "standalone")]
const MAX_QUEUED_AUDIO_BUFFERS: usize = 4;

#[cfg(feature = "standalone")]
fn host_output_enabled_default() -> bool {
    true
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SampleFormat {
    U8,
    S16Le,
}

impl SampleFormat {
    pub fn from_sdk_value(value: u16) -> Option<Self> {
        match value {
            8 => Some(Self::U8),
            16 => Some(Self::S16Le),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AudioConfig {
    pub sample_rate: u32,
    pub format: SampleFormat,
    pub channels: u8,
    pub volume: u8,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct AudioDiagnostics {
    pub schema_version: u32,
    pub configurations: Vec<AudioConfig>,
    pub open_count: u64,
    pub close_count: u64,
    pub write_calls: u64,
    pub successful_write_calls: u64,
    pub rejected_write_calls: u64,
    pub silenced_write_calls: u64,
    pub queue_full_events: u64,
    pub submitted_bytes: u64,
    pub decoded_frames: u64,
    pub decoded_samples: u64,
    pub nonzero_samples: u64,
    pub clipped_samples: u64,
    pub peak_amplitude: u16,
    pub rms_amplitude: f64,
    pub pcm_crc32: Option<String>,
    pub observed_video_frames: u64,
    pub active_audio_frames: u64,
    pub underflow_frames: u64,
    pub max_consecutive_underflow_frames: u64,
    pub max_buffered_frames: u64,
    pub buffered_frames_at_end: u64,
}

#[derive(Clone, Default)]
struct AudioDiagnosticsTracker {
    configurations: Vec<AudioConfig>,
    open_count: u64,
    close_count: u64,
    write_calls: u64,
    successful_write_calls: u64,
    rejected_write_calls: u64,
    silenced_write_calls: u64,
    queue_full_events: u64,
    submitted_bytes: u64,
    decoded_frames: u64,
    decoded_samples: u64,
    nonzero_samples: u64,
    clipped_samples: u64,
    peak_amplitude: u16,
    sum_squares: f64,
    pcm_crc32: crc32fast::Hasher,
    observed_video_frames: u64,
    active_audio_frames: u64,
    underflow_frames: u64,
    consecutive_underflow_frames: u64,
    max_consecutive_underflow_frames: u64,
    buffered_frame_units: u64,
    max_buffered_frames: u64,
    stream_started: bool,
}

impl AudioDiagnosticsTracker {
    #[cfg(feature = "standalone")]
    fn can_accept_half_second(&self, sample_rate: u32) -> bool {
        self.buffered_frame_units < u64::from(sample_rate) * 30
    }

    fn record_open(&mut self, config: AudioConfig) {
        self.open_count += 1;
        self.configurations.push(config);
        self.buffered_frame_units = 0;
        self.consecutive_underflow_frames = 0;
        self.stream_started = false;
    }

    fn record_close(&mut self) {
        self.close_count += 1;
        self.buffered_frame_units = 0;
        self.consecutive_underflow_frames = 0;
        self.stream_started = false;
    }

    fn record_write(&mut self, data: &[u8], samples: &[f32], channels: usize) {
        self.successful_write_calls += 1;
        self.submitted_bytes += data.len() as u64;
        self.decoded_samples += samples.len() as u64;
        let frames = (samples.len() / channels) as u64;
        self.decoded_frames += frames;
        self.buffered_frame_units = self
            .buffered_frame_units
            .saturating_add(frames.saturating_mul(60));
        self.max_buffered_frames = self
            .max_buffered_frames
            .max(self.buffered_frame_units.div_ceil(60));
        self.stream_started = true;
        self.pcm_crc32.update(data);

        for sample in samples {
            let amplitude = sample.abs();
            if *sample != 0.0 {
                self.nonzero_samples += 1;
            }
            if amplitude >= 1.0 {
                self.clipped_samples += 1;
            }
            self.peak_amplitude = self
                .peak_amplitude
                .max((amplitude * 32768.0).round().min(32768.0) as u16);
            self.sum_squares += f64::from(*sample) * f64::from(*sample);
        }
    }

    fn advance_frame(&mut self, config: Option<AudioConfig>) {
        self.observed_video_frames += 1;
        let Some(config) = config.filter(|_| self.stream_started) else {
            return;
        };

        self.active_audio_frames += 1;
        if self.buffered_frame_units >= u64::from(config.sample_rate) {
            self.buffered_frame_units -= u64::from(config.sample_rate);
            self.consecutive_underflow_frames = 0;
        } else {
            self.buffered_frame_units = 0;
            self.underflow_frames += 1;
            self.consecutive_underflow_frames += 1;
            self.max_consecutive_underflow_frames = self
                .max_consecutive_underflow_frames
                .max(self.consecutive_underflow_frames);
        }
    }

    fn snapshot(&self) -> AudioDiagnostics {
        let rms_amplitude = if self.decoded_samples == 0 {
            0.0
        } else {
            (self.sum_squares / self.decoded_samples as f64).sqrt()
        };
        AudioDiagnostics {
            schema_version: 1,
            configurations: self.configurations.clone(),
            open_count: self.open_count,
            close_count: self.close_count,
            write_calls: self.write_calls,
            successful_write_calls: self.successful_write_calls,
            rejected_write_calls: self.rejected_write_calls,
            silenced_write_calls: self.silenced_write_calls,
            queue_full_events: self.queue_full_events,
            submitted_bytes: self.submitted_bytes,
            decoded_frames: self.decoded_frames,
            decoded_samples: self.decoded_samples,
            nonzero_samples: self.nonzero_samples,
            clipped_samples: self.clipped_samples,
            peak_amplitude: self.peak_amplitude,
            rms_amplitude,
            pcm_crc32: (self.successful_write_calls > 0)
                .then(|| format!("{:08x}", self.pcm_crc32.clone().finalize())),
            observed_video_frames: self.observed_video_frames,
            active_audio_frames: self.active_audio_frames,
            underflow_frames: self.underflow_frames,
            max_consecutive_underflow_frames: self.max_consecutive_underflow_frames,
            max_buffered_frames: self.max_buffered_frames,
            buffered_frames_at_end: self.buffered_frame_units.div_ceil(60),
        }
    }
}

impl AudioConfig {
    pub fn new(sample_rate: u32, format: u16, channels: u8, volume: u8) -> Option<Self> {
        if sample_rate == 0 || !(1..=2).contains(&channels) {
            return None;
        }
        Some(Self {
            sample_rate,
            format: SampleFormat::from_sdk_value(format)?,
            channels,
            volume,
        })
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Audio {
    config: Option<AudioConfig>,
    volume: u8,
    master_volume: u8,
    muted: bool,
    #[cfg(feature = "standalone")]
    #[serde(skip)]
    mixer_device: Option<rodio::MixerDeviceSink>,
    #[cfg(feature = "standalone")]
    #[serde(skip)]
    mixer: Option<rodio::mixer::Mixer>,
    #[cfg(feature = "standalone")]
    #[serde(skip)]
    player: Option<rodio::Player>,
    #[cfg(feature = "standalone")]
    #[serde(skip, default = "host_output_enabled_default")]
    host_output_enabled: bool,
    #[cfg(not(feature = "standalone"))]
    pending_samples: VecDeque<i16>,
    #[cfg(not(feature = "standalone"))]
    output_frame_remainder: u32,
    #[cfg(not(feature = "standalone"))]
    resampler: StreamingResampler,
    #[serde(skip)]
    diagnostics: AudioDiagnosticsTracker,
}

impl Audio {
    pub fn new() -> Self {
        Self {
            config: None,
            volume: 100,
            master_volume: 100,
            muted: false,
            #[cfg(feature = "standalone")]
            mixer_device: None,
            #[cfg(feature = "standalone")]
            mixer: None,
            #[cfg(feature = "standalone")]
            player: None,
            #[cfg(feature = "standalone")]
            host_output_enabled: true,
            #[cfg(not(feature = "standalone"))]
            pending_samples: VecDeque::new(),
            #[cfg(not(feature = "standalone"))]
            output_frame_remainder: 0,
            #[cfg(not(feature = "standalone"))]
            resampler: StreamingResampler::default(),
            diagnostics: AudioDiagnosticsTracker::default(),
        }
    }

    pub fn open(&mut self, config: AudioConfig) -> bool {
        self.close();
        self.volume = config.volume;
        self.config = Some(config);
        self.diagnostics.record_open(config);

        #[cfg(feature = "standalone")]
        {
            if self.host_output_enabled && self.mixer_device.is_none() {
                match rodio::DeviceSinkBuilder::open_default_sink() {
                    Ok(mut device) => {
                        device.log_on_drop(false);
                        self.mixer = Some(device.mixer().clone());
                        self.mixer_device = Some(device);
                    }
                    Err(error) => {
                        log::warn!("Failed to initialize audio output: {error}");
                    }
                }
            }

            if self.host_output_enabled {
                if let Some(mixer) = self.mixer.as_ref() {
                    let player = rodio::Player::connect_new(mixer);
                    player.set_volume(self.effective_volume());
                    self.player = Some(player);
                }
            }
        }

        #[cfg(not(feature = "standalone"))]
        self.resampler.reset(config.sample_rate);

        log::info!(
            "Audio opened: {} Hz, {:?}, {} channel(s), volume {}",
            config.sample_rate,
            config.format,
            config.channels,
            config.volume
        );
        true
    }

    pub fn close(&mut self) -> bool {
        let was_open = self.config.is_some();
        #[cfg(feature = "standalone")]
        if let Some(player) = self.player.take() {
            player.stop();
        }

        #[cfg(not(feature = "standalone"))]
        {
            self.pending_samples.clear();
            self.output_frame_remainder = 0;
            self.resampler = StreamingResampler::default();
        }

        self.config = None;
        if was_open {
            self.diagnostics.record_close();
        }
        true
    }

    pub fn can_write(&self) -> bool {
        if self.config.is_none() || self.muted || self.volume == 0 {
            return true;
        }

        #[cfg(feature = "standalone")]
        {
            if self.host_output_enabled {
                self.player
                    .as_ref()
                    .is_none_or(|player| player.len() < MAX_QUEUED_AUDIO_BUFFERS)
            } else {
                self.config.is_some_and(|config| {
                    self.diagnostics.can_accept_half_second(config.sample_rate)
                })
            }
        }

        #[cfg(not(feature = "standalone"))]
        {
            self.pending_samples.len() / 2 < MAX_QUEUED_AUDIO_FRAMES
        }
    }

    pub fn write(&mut self, data: &[u8]) -> bool {
        self.diagnostics.write_calls += 1;
        let Some(config) = self.config else {
            self.diagnostics.rejected_write_calls += 1;
            return false;
        };
        if data.is_empty() {
            self.diagnostics.rejected_write_calls += 1;
            return false;
        }
        if self.muted || self.volume == 0 {
            self.diagnostics.silenced_write_calls += 1;
            return true;
        }
        if !self.can_write() {
            self.diagnostics.rejected_write_calls += 1;
            return false;
        }

        let samples = decode_pcm(data, config.format, config.channels);
        if samples.is_empty() {
            self.diagnostics.rejected_write_calls += 1;
            return false;
        }
        self.diagnostics
            .record_write(data, &samples, config.channels as usize);
        let peak = samples
            .iter()
            .fold(0.0f32, |peak, sample| peak.max(sample.abs()));

        #[cfg(feature = "standalone")]
        {
            let Some(player) = self.player.as_ref() else {
                return true;
            };
            let channels = NonZero::<u16>::new(config.channels as u16).unwrap();
            let sample_rate = NonZero::<u32>::new(config.sample_rate).unwrap();
            player.append(rodio::buffer::SamplesBuffer::new(
                channels,
                sample_rate,
                samples,
            ));
        }

        #[cfg(not(feature = "standalone"))]
        self.resampler.push(
            &samples,
            config.channels as usize,
            self.effective_volume(),
            &mut self.pending_samples,
        );

        log::trace!(
            "Queued {} bytes of guest PCM audio (peak {peak:.3})",
            data.len()
        );
        true
    }

    pub fn set_volume(&mut self, volume: u32) -> bool {
        self.volume = volume.min(u8::MAX as u32) as u8;
        #[cfg(feature = "standalone")]
        if let Some(player) = self.player.as_ref() {
            player.set_volume(self.effective_volume());
        }
        true
    }

    pub fn set_master_volume(&mut self, volume: u8) {
        self.master_volume = volume.min(100);
        #[cfg(feature = "standalone")]
        if let Some(player) = self.player.as_ref() {
            player.set_volume(self.effective_volume());
        }
    }

    pub fn master_volume(&self) -> u8 {
        self.master_volume
    }

    #[cfg(feature = "standalone")]
    pub fn set_host_output_enabled(&mut self, enabled: bool) {
        self.host_output_enabled = enabled;
        if !enabled {
            if let Some(player) = self.player.take() {
                player.stop();
            }
            self.mixer = None;
            self.mixer_device = None;
        }
    }

    #[cfg(feature = "standalone")]
    pub fn host_output_enabled(&self) -> bool {
        self.host_output_enabled
    }

    pub fn set_muted(&mut self, muted: bool) -> bool {
        self.muted = muted;
        #[cfg(feature = "standalone")]
        if let Some(player) = self.player.as_ref() {
            player.set_volume(self.effective_volume());
        }
        #[cfg(not(feature = "standalone"))]
        if muted {
            self.pending_samples.clear();
        }
        true
    }

    pub fn take_frame_samples(&mut self) -> Vec<i16> {
        #[cfg(feature = "standalone")]
        {
            Vec::new()
        }

        #[cfg(not(feature = "standalone"))]
        {
            self.output_frame_remainder += OUTPUT_SAMPLE_RATE;
            let frame_count = (self.output_frame_remainder / VIDEO_FRAMES_PER_SECOND) as usize;
            self.output_frame_remainder %= VIDEO_FRAMES_PER_SECOND;

            let mut output = Vec::with_capacity(frame_count * 2);
            for _ in 0..frame_count * 2 {
                output.push(self.pending_samples.pop_front().unwrap_or(0));
            }
            output
        }
    }

    pub fn config(&self) -> Option<AudioConfig> {
        self.config
    }

    pub fn diagnostics(&self) -> AudioDiagnostics {
        self.diagnostics.snapshot()
    }

    pub fn record_queue_full(&mut self) {
        self.diagnostics.queue_full_events += 1;
    }

    pub fn advance_frame(&mut self) {
        self.diagnostics.advance_frame(self.config);
    }

    pub(crate) fn resume_after_state_load(&mut self) {
        #[cfg(feature = "standalone")]
        if let Some(config) = self.config.take() {
            let volume = self.volume;
            let master_volume = self.master_volume;
            let muted = self.muted;
            self.open(config);
            self.set_volume(volume as u32);
            self.set_master_volume(master_volume);
            self.set_muted(muted);
        }
    }

    fn effective_volume(&self) -> f32 {
        if self.muted {
            return 0.0;
        }
        if self.volume <= 100 {
            self.volume as f32 / 100.0 * self.master_volume as f32 / 100.0
        } else {
            self.volume as f32 / 255.0 * self.master_volume as f32 / 100.0
        }
    }
}

impl Default for Audio {
    fn default() -> Self {
        Self::new()
    }
}

fn decode_pcm(data: &[u8], format: SampleFormat, channels: u8) -> Vec<f32> {
    let mut samples = match format {
        SampleFormat::U8 => data
            .iter()
            .map(|&sample| (sample as f32 - 128.0) / 128.0)
            .collect::<Vec<_>>(),
        SampleFormat::S16Le => data
            .chunks_exact(2)
            .map(|sample| i16::from_le_bytes([sample[0], sample[1]]) as f32 / 32768.0)
            .collect::<Vec<_>>(),
    };
    samples.truncate(samples.len() / channels as usize * channels as usize);
    samples
}

#[cfg(not(feature = "standalone"))]
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct StreamingResampler {
    input_rate: u32,
    input_frames_seen: u64,
    next_output_time: u64,
    previous_frame: Option<[f32; 2]>,
}

#[cfg(not(feature = "standalone"))]
impl StreamingResampler {
    fn reset(&mut self, input_rate: u32) {
        self.input_rate = input_rate;
        self.input_frames_seen = 0;
        self.next_output_time = 0;
        self.previous_frame = None;
    }

    fn push(&mut self, samples: &[f32], channels: usize, volume: f32, output: &mut VecDeque<i16>) {
        for input in samples.chunks_exact(channels) {
            let current = [input[0], input[1.min(channels - 1)]];
            let current_index = self.input_frames_seen;

            if let Some(previous) = self.previous_frame {
                let segment_start = (current_index - 1) * OUTPUT_SAMPLE_RATE as u64;
                let segment_end = current_index * OUTPUT_SAMPLE_RATE as u64;
                while self.next_output_time <= segment_end {
                    let fraction =
                        (self.next_output_time - segment_start) as f32 / OUTPUT_SAMPLE_RATE as f32;
                    let left = previous[0] + (current[0] - previous[0]) * fraction;
                    let right = previous[1] + (current[1] - previous[1]) * fraction;
                    output.push_back(float_to_i16(left * volume));
                    output.push_back(float_to_i16(right * volume));
                    self.next_output_time += self.input_rate as u64;
                }
            } else {
                output.push_back(float_to_i16(current[0] * volume));
                output.push_back(float_to_i16(current[1] * volume));
                self.next_output_time = self.input_rate as u64;
            }

            self.previous_frame = Some(current);
            self.input_frames_seen += 1;
        }
    }
}

#[cfg(not(feature = "standalone"))]
fn float_to_i16(sample: f32) -> i16 {
    (sample * 32767.0).clamp(i16::MIN as f32, i16::MAX as f32) as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_s16le_mono_pcm() {
        let samples = decode_pcm(&[0x00, 0x80, 0x00, 0x40], SampleFormat::S16Le, 1);

        assert_eq!(samples, vec![-1.0, 0.5]);
    }

    #[test]
    fn drops_incomplete_pcm_frames() {
        let samples = decode_pcm(&[0, 128, 255], SampleFormat::U8, 2);

        assert_eq!(samples.len(), 2);
    }

    #[test]
    fn master_volume_is_clamped_to_percent_range() {
        let mut audio = Audio::new();
        audio.set_master_volume(35);
        assert_eq!(audio.master_volume(), 35);
        audio.set_master_volume(255);
        assert_eq!(audio.master_volume(), 100);
    }

    #[test]
    fn diagnostics_measure_guest_pcm_and_virtual_underflow() {
        let mut audio = Audio::new();
        let config = AudioConfig::new(600, 8, 1, 100).unwrap();
        assert!(audio.open(config));
        assert!(audio.write(&[128, 255, 0, 128, 192, 64]));
        audio.record_queue_full();
        audio.advance_frame();

        let diagnostics = audio.diagnostics();
        assert_eq!(diagnostics.configurations, [config]);
        assert_eq!(diagnostics.open_count, 1);
        assert_eq!(diagnostics.write_calls, 1);
        assert_eq!(diagnostics.successful_write_calls, 1);
        assert_eq!(diagnostics.submitted_bytes, 6);
        assert_eq!(diagnostics.decoded_frames, 6);
        assert_eq!(diagnostics.decoded_samples, 6);
        assert_eq!(diagnostics.nonzero_samples, 4);
        assert_eq!(diagnostics.clipped_samples, 1);
        assert_eq!(diagnostics.peak_amplitude, 32768);
        assert!(diagnostics.rms_amplitude > 0.0);
        assert!(diagnostics.pcm_crc32.is_some());
        assert_eq!(diagnostics.queue_full_events, 1);
        assert_eq!(diagnostics.active_audio_frames, 1);
        assert_eq!(diagnostics.underflow_frames, 1);
        assert_eq!(diagnostics.max_consecutive_underflow_frames, 1);
    }

    #[cfg(feature = "standalone")]
    #[test]
    fn disabled_host_output_uses_the_emulated_frame_clock_for_backpressure() {
        let mut audio = Audio::new();
        audio.set_host_output_enabled(false);
        assert!(!audio.host_output_enabled());
        assert!(audio.open(AudioConfig::new(8_000, 16, 1, 100).unwrap()));

        assert!(audio.write(&vec![0; 8_000]));
        assert!(!audio.can_write());
        audio.advance_frame();
        assert!(audio.can_write());
    }

    #[cfg(not(feature = "standalone"))]
    #[test]
    fn resamples_mono_audio_for_one_libretro_frame() {
        let mut audio = Audio::new();
        let config = AudioConfig::new(16_000, 16, 1, 100).unwrap();
        assert!(audio.open(config));

        let mut pcm = Vec::new();
        for sample in 0..1_600 {
            let value = if sample % 2 == 0 {
                10_000i16
            } else {
                -10_000i16
            };
            pcm.extend_from_slice(&value.to_le_bytes());
        }
        assert!(audio.write(&pcm));

        let output = audio.take_frame_samples();
        assert_eq!(output.len(), (OUTPUT_SAMPLE_RATE as usize / 60) * 2);
        assert!(output.iter().any(|&sample| sample != 0));
        assert!(output.chunks_exact(2).all(|frame| frame[0] == frame[1]));
    }
}
