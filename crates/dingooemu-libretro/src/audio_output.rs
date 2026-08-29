use std::collections::VecDeque;
use std::sync::Mutex;

use dingooemu_core::audio::OUTPUT_SAMPLE_RATE;

use crate::callbacks;

const CHANNELS: usize = 2;
const CALLBACK_FRAMES: usize = OUTPUT_SAMPLE_RATE as usize / 60;
const MAX_QUEUED_FRAMES: usize = OUTPUT_SAMPLE_RATE as usize / 2;

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
}

static STATE: Mutex<AudioOutputState> = Mutex::new(AudioOutputState::new());

pub fn reset(registered: bool) {
    let mut state = STATE.lock().unwrap();
    state.registered = registered;
    state.enabled = false;
    state.samples.clear();
}

pub fn set_enabled(enabled: bool) {
    let mut state = STATE.lock().unwrap();
    state.enabled = state.registered && enabled;
    if !state.enabled {
        state.samples.clear();
    }
    crate::diagnostics::record_async_audio_state(state.enabled);
}

pub fn enqueue(samples: &[i16]) -> Option<usize> {
    let mut state = STATE.lock().unwrap();
    if !state.registered {
        return None;
    }

    let frames = samples.len() / CHANNELS;
    let max_samples = MAX_QUEUED_FRAMES * CHANNELS;
    let incoming_samples = if samples.len() > max_samples {
        &samples[samples.len() - max_samples..]
    } else {
        samples
    };
    let overflow_samples = state
        .samples
        .len()
        .saturating_add(incoming_samples.len())
        .saturating_sub(max_samples);
    let dropped_queued_samples = overflow_samples.min(state.samples.len());
    let dropped_incoming_samples = samples.len() - incoming_samples.len();
    state.samples.drain(..dropped_queued_samples);
    state.samples.extend(incoming_samples.iter().copied());
    let queued_frames = state.samples.len() / CHANNELS;
    drop(state);

    crate::diagnostics::record_async_audio_enqueue(
        (dropped_queued_samples + dropped_incoming_samples) / CHANNELS,
        queued_frames,
    );
    Some(frames)
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
