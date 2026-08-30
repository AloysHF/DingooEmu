use std::collections::VecDeque;
use std::sync::Mutex;

use dingooemu_core::audio::OUTPUT_SAMPLE_RATE;

use crate::callbacks;

const CHANNELS: usize = 2;
const CALLBACK_FRAMES: usize = OUTPUT_SAMPLE_RATE as usize / 60;
const MAX_QUEUED_FRAMES: usize = CALLBACK_FRAMES * 3;

struct EnqueueResult {
    frames: usize,
    dropped_frames: usize,
    queued_frames: usize,
}

struct AudioOutputState {
    registered: bool,
    enabled: bool,
    samples: VecDeque<i16>,
}

impl AudioOutputState {
    const fn new() -> Self {
        Self {
            registered: false,
            enabled: false,
            samples: VecDeque::new(),
        }
    }

    fn reset(&mut self, registered: bool) {
        self.registered = registered;
        self.enabled = false;
        self.samples.clear();
    }

    fn set_enabled(&mut self, enabled: bool) {
        let enabled = self.registered && enabled;
        if self.enabled != enabled {
            self.samples.clear();
        }
        self.enabled = enabled;
    }

    fn enqueue(&mut self, samples: &[i16]) -> Option<EnqueueResult> {
        if !self.registered {
            return None;
        }

        let frames = samples.len() / CHANNELS;
        if !self.enabled {
            return Some(EnqueueResult {
                frames,
                dropped_frames: frames,
                queued_frames: 0,
            });
        }

        let max_samples = MAX_QUEUED_FRAMES * CHANNELS;
        let incoming_samples = if samples.len() > max_samples {
            &samples[samples.len() - max_samples..]
        } else {
            samples
        };
        let overflow_samples = self
            .samples
            .len()
            .saturating_add(incoming_samples.len())
            .saturating_sub(max_samples);
        let dropped_queued_samples = overflow_samples.min(self.samples.len());
        let dropped_incoming_samples = samples.len() - incoming_samples.len();
        self.samples.drain(..dropped_queued_samples);
        self.samples.extend(incoming_samples.iter().copied());

        Some(EnqueueResult {
            frames,
            dropped_frames: (dropped_queued_samples + dropped_incoming_samples) / CHANNELS,
            queued_frames: self.samples.len() / CHANNELS,
        })
    }
}

static STATE: Mutex<AudioOutputState> = Mutex::new(AudioOutputState::new());

pub fn reset(registered: bool) {
    let mut state = STATE.lock().unwrap();
    state.reset(registered);
}

pub fn set_enabled(enabled: bool) {
    let enabled = {
        let mut state = STATE.lock().unwrap();
        state.set_enabled(enabled);
        state.enabled
    };
    crate::diagnostics::record_async_audio_state(enabled);
}

pub fn enqueue(samples: &[i16]) -> Option<usize> {
    let result = {
        let mut state = STATE.lock().unwrap();
        state.enqueue(samples)
    };
    result.map(|result| {
        crate::diagnostics::record_async_audio_enqueue(result.dropped_frames, result.queued_frames);
        result.frames
    })
}

pub unsafe extern "C" fn callback() {
    let mut output = [0_i16; CALLBACK_FRAMES * CHANNELS];
    let real_frames = {
        let mut state = STATE.lock().unwrap();
        if !state.enabled {
            return;
        }
        let samples_to_copy = output.len().min(state.samples.len());
        for sample in output.iter_mut().take(samples_to_copy) {
            *sample = state.samples.pop_front().unwrap();
        }
        samples_to_copy / CHANNELS
    };

    let accepted = callbacks::audio_sample_batch(output.as_ptr(), CALLBACK_FRAMES).map_or_else(
        || {
            for sample in output.as_chunks::<CHANNELS>().0 {
                callbacks::audio_sample(sample[0], sample[1]);
            }
            CALLBACK_FRAMES
        },
        |accepted| accepted.min(CALLBACK_FRAMES),
    );
    crate::diagnostics::record_async_audio_callback(real_frames, accepted, CALLBACK_FRAMES);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stereo_samples(frames: usize, value: i16) -> Vec<i16> {
        vec![value; frames * CHANNELS]
    }

    #[test]
    fn disabled_output_discards_samples_without_queueing_them() {
        let mut state = AudioOutputState::new();
        state.reset(true);

        let result = state.enqueue(&stereo_samples(CALLBACK_FRAMES, 1)).unwrap();

        assert_eq!(result.frames, CALLBACK_FRAMES);
        assert_eq!(result.dropped_frames, CALLBACK_FRAMES);
        assert_eq!(result.queued_frames, 0);
        assert!(state.samples.is_empty());
    }

    #[test]
    fn state_transitions_remove_stale_audio() {
        let mut state = AudioOutputState::new();
        state.reset(true);
        state.set_enabled(true);
        state.enqueue(&stereo_samples(CALLBACK_FRAMES, 1)).unwrap();
        assert!(!state.samples.is_empty());

        state.set_enabled(false);
        assert!(state.samples.is_empty());
        state.enqueue(&stereo_samples(CALLBACK_FRAMES, 2)).unwrap();
        state.set_enabled(true);

        assert!(state.samples.is_empty());
    }

    #[test]
    fn queue_is_limited_to_three_callback_periods() {
        let mut state = AudioOutputState::new();
        state.reset(true);
        state.set_enabled(true);

        for value in 1..=3 {
            let result = state
                .enqueue(&stereo_samples(CALLBACK_FRAMES, value))
                .unwrap();
            assert_eq!(result.dropped_frames, 0);
        }
        let result = state.enqueue(&stereo_samples(CALLBACK_FRAMES, 4)).unwrap();

        assert_eq!(result.dropped_frames, CALLBACK_FRAMES);
        assert_eq!(result.queued_frames, MAX_QUEUED_FRAMES);
        assert_eq!(state.samples.front(), Some(&2));
        assert_eq!(state.samples.back(), Some(&4));
    }
}
