/// Dingoo A320 button masks
pub const BUTTON_UP: u32 = 1 << 20;
pub const BUTTON_DOWN: u32 = 1 << 27;
pub const BUTTON_LEFT: u32 = 1 << 28;
pub const BUTTON_RIGHT: u32 = 1 << 18;
pub const BUTTON_A: u32 = 1 << 31;
pub const BUTTON_B: u32 = 1 << 21;
pub const BUTTON_X: u32 = 1 << 16;
pub const BUTTON_Y: u32 = 1 << 6;
pub const BUTTON_START: u32 = 1 << 11;
pub const BUTTON_SELECT: u32 = 1 << 10;
pub const BUTTON_L: u32 = 1 << 8;
pub const BUTTON_R: u32 = 1 << 29;

/// Input subsystem
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Input {
    /// Current button state (bitmask)
    buttons: u32,
    /// Buttons pressed since the last status poll
    pressed: u32,
    /// Buttons released since the last status poll
    released: u32,
    /// Whether a system/input event is pending
    event_pending: bool,
    repeat_delay: u32,
    repeat_period: u32,
    held_frames: [u32; 32],
    swap_ab: bool,
}

impl Input {
    /// Create a new input subsystem
    pub fn new() -> Self {
        Self {
            buttons: 0,
            pressed: 0,
            released: 0,
            event_pending: false,
            repeat_delay: 0,
            repeat_period: 1,
            held_frames: [0; 32],
            swap_ab: false,
        }
    }

    /// Get the current button state
    pub fn buttons(&self) -> u32 {
        self.buttons
    }

    /// Set the button state
    pub fn set_buttons(&mut self, buttons: u32) {
        let buttons = if self.swap_ab {
            let without_ab = buttons & !(BUTTON_A | BUTTON_B);
            without_ab
                | if buttons & BUTTON_A != 0 { BUTTON_B } else { 0 }
                | if buttons & BUTTON_B != 0 { BUTTON_A } else { 0 }
        } else {
            buttons
        };
        let changed = self.buttons ^ buttons;
        self.pressed |= changed & buttons;
        self.released |= changed & self.buttons;
        if changed != 0 {
            self.event_pending = true;
        }
        for bit in 0..32 {
            let mask = 1u32 << bit;
            if buttons & mask == 0 {
                self.held_frames[bit] = 0;
            } else if self.buttons & mask == 0 {
                self.held_frames[bit] = 1;
            } else {
                self.held_frames[bit] = self.held_frames[bit].saturating_add(1);
                let held = self.held_frames[bit];
                if self.repeat_delay > 0
                    && held >= self.repeat_delay
                    && (held - self.repeat_delay).is_multiple_of(self.repeat_period)
                {
                    self.pressed |= mask;
                    self.event_pending = true;
                }
            }
        }
        self.buttons = buttons;
    }

    pub fn set_repeat_timing(&mut self, delay: u32, period: u32) {
        self.repeat_delay = delay;
        self.repeat_period = period.max(1);
    }

    pub fn set_swap_ab(&mut self, swap_ab: bool) {
        self.swap_ab = swap_ab;
    }

    /// Get and clear the Dingoo key status structure fields.
    pub fn take_status(&mut self) -> (u32, u32, u32) {
        let status = (self.pressed, self.released, self.buttons);
        self.pressed = 0;
        self.released = 0;
        status
    }

    /// Get and clear whether an input event is pending.
    pub fn take_pending_event(&mut self) -> bool {
        let pending = self.event_pending;
        self.event_pending = false;
        pending
    }

    /// Check if a specific button is pressed
    pub fn is_pressed(&self, button: u32) -> bool {
        (self.buttons & button) != 0
    }

    /// Press a button
    pub fn press(&mut self, button: u32) {
        self.set_buttons(self.buttons | button);
    }

    /// Release a button
    pub fn release(&mut self, button: u32) {
        self.set_buttons(self.buttons & !button);
    }
}

impl Default for Input {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_input_creation() {
        let input = Input::new();
        assert_eq!(input.buttons(), 0);
    }

    #[test]
    fn test_button_press_release() {
        let mut input = Input::new();
        input.press(BUTTON_A);
        assert!(input.is_pressed(BUTTON_A));
        assert!(!input.is_pressed(BUTTON_B));

        input.release(BUTTON_A);
        assert!(!input.is_pressed(BUTTON_A));
    }

    #[test]
    fn held_button_repeats_after_configured_delay() {
        let mut input = Input::new();
        input.set_repeat_timing(3, 2);
        input.set_buttons(BUTTON_A);
        assert_eq!(input.take_status().0, BUTTON_A);
        input.set_buttons(BUTTON_A);
        assert_eq!(input.take_status().0, 0);
        input.set_buttons(BUTTON_A);
        assert_eq!(input.take_status().0, BUTTON_A);
        input.set_buttons(BUTTON_A);
        assert_eq!(input.take_status().0, 0);
        input.set_buttons(BUTTON_A);
        assert_eq!(input.take_status().0, BUTTON_A);
    }

    #[test]
    fn swap_ab_changes_logical_button_state() {
        let mut input = Input::new();
        input.set_swap_ab(true);
        input.set_buttons(BUTTON_A);
        assert_eq!(input.buttons(), BUTTON_B);
        input.set_buttons(BUTTON_B);
        assert_eq!(input.buttons(), BUTTON_A);
    }
}
