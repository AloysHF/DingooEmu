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
    #[cfg(feature = "standalone")]
    #[serde(skip)]
    virtual_buffered_frame_units: u64,
    #[cfg(not(feature = "standalone"))]
    pending_samples: VecDeque<i16>,
    #[cfg(not(feature = "standalone"))]
    output_frame_remainder: u32,
    #[cfg(not(feature = "standalone"))]
    resampler: StreamingResampler,
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
            #[cfg(feature = "standalone")]
            virtual_buffered_frame_units: 0,
            #[cfg(not(feature = "standalone"))]
            pending_samples: VecDeque::new(),
            #[cfg(not(feature = "standalone"))]
            output_frame_remainder: 0,
            #[cfg(not(feature = "standalone"))]
            resampler: StreamingResampler::default(),
        }
    }

    pub fn open(&mut self, config: AudioConfig) -> bool {
        self.close();
        self.volume = config.volume;
        self.config = Some(config);
        #[cfg(feature = "standalone")]
        {
            self.virtual_buffered_frame_units = 0;
        }

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
        #[cfg(feature = "standalone")]
        {
            if let Some(player) = self.player.take() {
                player.stop();
            }
            self.virtual_buffered_frame_units = 0;
        }

        #[cfg(not(feature = "standalone"))]
        {
            self.pending_samples.clear();
            self.output_frame_remainder = 0;
            self.resampler = StreamingResampler::default();
        }

        self.config = None;
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
                    self.virtual_buffered_frame_units < u64::from(config.sample_rate) * 30
                })
            }
        }

        #[cfg(not(feature = "standalone"))]
        {
            self.pending_samples.len() / 2 < MAX_QUEUED_AUDIO_FRAMES
        }
    }

    pub fn write(&mut self, data: &[u8]) -> bool {
        let Some(config) = self.config else {
            return false;
        };
        if data.is_empty() {
            return false;
        }
        if self.muted || self.volume == 0 {
            return true;
        }
        if !self.can_write() {
            return false;
        }

        let samples = decode_pcm(data, config.format, config.channels);
        if samples.is_empty() {
            return false;
        }
        let peak = samples
            .iter()
            .fold(0.0f32, |peak, sample| peak.max(sample.abs()));

        #[cfg(feature = "standalone")]
        {
            if !self.host_output_enabled {
                let frames = (samples.len() / config.channels as usize) as u64;
                self.virtual_buffered_frame_units = self
                    .virtual_buffered_frame_units
                    .saturating_add(frames.saturating_mul(60));
                return true;
            }
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
            self.virtual_buffered_frame_units = 0;
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

    pub fn advance_frame(&mut self) {
        #[cfg(feature = "standalone")]
        if !self.host_output_enabled {
            if let Some(config) = self.config {
                self.virtual_buffered_frame_units = self
                    .virtual_buffered_frame_units
                    .saturating_sub(u64::from(config.sample_rate));
            }
        }
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
            .as_chunks::<2>()
            .0
            .iter()
            .map(|sample| i16::from_le_bytes(*sample) as f32 / 32768.0)
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
        assert!(output
            .as_chunks::<2>()
            .0
            .iter()
            .all(|frame| frame[0] == frame[1]));
    }
}
