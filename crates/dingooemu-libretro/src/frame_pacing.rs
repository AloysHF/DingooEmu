use std::sync::Mutex;

const FRAME_UNITS: u64 = 1_000_000;
const UNITS_PER_MICROSECOND: u64 = 60;

struct FramePacing {
    elapsed_us: Option<i64>,
    accumulated_units: u64,
}

impl FramePacing {
    const fn new() -> Self {
        Self {
            elapsed_us: None,
            accumulated_units: 0,
        }
    }

    fn should_advance(&mut self) -> bool {
        let Some(elapsed_us) = self.elapsed_us.take().filter(|elapsed| *elapsed >= 0) else {
            // Frontends without timing callbacks still run one frame per call.
            self.accumulated_units = 0;
            return true;
        };
        self.accumulated_units = (self.accumulated_units
            + (elapsed_us as u64).min(FRAME_UNITS) * UNITS_PER_MICROSECOND)
            .min(FRAME_UNITS * 2);
        // Allow one microsecond of rounding in the frontend's 60 Hz reference.
        if self.accumulated_units < FRAME_UNITS - UNITS_PER_MICROSECOND {
            return false;
        }
        // Carry timing jitter forward, with at most one pending frame after a stall.
        self.accumulated_units = self.accumulated_units.saturating_sub(FRAME_UNITS);
        true
    }
}

static STATE: Mutex<FramePacing> = Mutex::new(FramePacing::new());

pub fn reset() {
    *STATE.lock().unwrap() = FramePacing::new();
}

pub fn set_elapsed(elapsed_us: i64) {
    STATE.lock().unwrap().elapsed_us = Some(elapsed_us);
}

pub fn should_advance() -> bool {
    STATE.lock().unwrap().should_advance()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn advance(pacing: &mut FramePacing, elapsed_us: i64) -> bool {
        pacing.elapsed_us = Some(elapsed_us);
        pacing.should_advance()
    }

    #[test]
    fn high_refresh_rates_keep_sixty_guest_frames_per_second() {
        for hz in [60, 75, 90, 120, 144, 165, 240] {
            let mut pacing = FramePacing::new();
            let frames = (0..hz * 100)
                .filter(|frame| {
                    let elapsed = (frame + 1) * 1_000_000 / hz - frame * 1_000_000 / hz;
                    advance(&mut pacing, elapsed)
                })
                .count();
            assert_eq!(frames, 6_000, "{hz} Hz");
        }
    }

    #[test]
    fn reference_time_preserves_fast_forward_and_frame_stepping() {
        let mut pacing = FramePacing::new();
        for _ in 0..100_000 {
            assert!(advance(&mut pacing, 16_666));
        }
    }

    #[test]
    fn missing_or_invalid_timing_does_not_reuse_the_previous_interval() {
        let mut pacing = FramePacing::new();
        assert!(!advance(&mut pacing, 8_333));
        assert!(pacing.should_advance());
        assert!(!advance(&mut pacing, 8_333));
        assert!(advance(&mut pacing, -1));
        assert!(!advance(&mut pacing, 0));
    }

    #[test]
    fn stalls_leave_at_most_one_pending_guest_frame() {
        let mut pacing = FramePacing::new();
        assert!(advance(&mut pacing, i64::MAX));
        assert!(advance(&mut pacing, 0));
        assert!(!advance(&mut pacing, 0));
        assert!(!advance(&mut pacing, 8_333));
        assert!(advance(&mut pacing, 8_334));
    }

    #[test]
    fn timing_jitter_does_not_accumulate_slowdown() {
        let mut pacing = FramePacing::new();
        let frames = (0..6_000)
            .filter(|frame| {
                let elapsed = match frame % 6 {
                    0 | 2 | 4 => 16_000,
                    1 | 3 => 17_333,
                    _ => 17_334,
                };
                advance(&mut pacing, elapsed)
            })
            .count();
        assert_eq!(frames, 5_999);
        assert!(advance(&mut pacing, 0));
        assert!(!advance(&mut pacing, 0));
    }

    #[test]
    fn only_the_latest_callback_is_used_after_a_pause() {
        let mut pacing = FramePacing::new();
        for _ in 0..120 {
            pacing.elapsed_us = Some(8_333);
        }
        assert!(!pacing.should_advance());
    }
}
