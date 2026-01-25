/// Dingoo A320 button masks
pub const BUTTON_UP: u32 = 0x0001;
pub const BUTTON_DOWN: u32 = 0x0002;
pub const BUTTON_LEFT: u32 = 0x0004;
pub const BUTTON_RIGHT: u32 = 0x0008;
pub const BUTTON_A: u32 = 0x0010;
pub const BUTTON_B: u32 = 0x0020;
pub const BUTTON_X: u32 = 0x0040;
pub const BUTTON_Y: u32 = 0x0080;
pub const BUTTON_START: u32 = 0x0100;
pub const BUTTON_SELECT: u32 = 0x0200;
pub const BUTTON_L: u32 = 0x0400;
pub const BUTTON_R: u32 = 0x0800;

/// Input subsystem
pub struct Input {
    /// Current button state (bitmask)
    buttons: u32,
}

impl Input {
    /// Create a new input subsystem
    pub fn new() -> Self {
        Self { buttons: 0 }
    }

    /// Get the current button state
    pub fn buttons(&self) -> u32 {
        self.buttons
    }

    /// Set the button state
    pub fn set_buttons(&mut self, buttons: u32) {
        self.buttons = buttons;
    }

    /// Check if a specific button is pressed
    pub fn is_pressed(&self, button: u32) -> bool {
        (self.buttons & button) != 0
    }

    /// Press a button
    pub fn press(&mut self, button: u32) {
        self.buttons |= button;
    }

    /// Release a button
    pub fn release(&mut self, button: u32) {
        self.buttons &= !button;
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
}
