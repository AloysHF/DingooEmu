use dingooemu_core::common::input::{
    BUTTON_A, BUTTON_B, BUTTON_DOWN, BUTTON_L, BUTTON_LEFT, BUTTON_R, BUTTON_RIGHT, BUTTON_SELECT,
    BUTTON_START, BUTTON_UP, BUTTON_X, BUTTON_Y,
};
use gilrs::{Axis, Button, EventType, Gilrs};

const STICK_DEADZONE: f32 = 0.5;

/// Polls the first connected physical gamepad and maps it onto Dingoo buttons.
///
/// Face buttons follow the same RetroPad convention as the default keyboard
/// bindings: South→B, East→A, West→Y, North→X. Left/right sticks also act as
/// a digital D-pad once past [`STICK_DEADZONE`].
pub struct GamepadMapper {
    gilrs: Option<Gilrs>,
    swap_ab: bool,
}

impl GamepadMapper {
    pub fn new(enabled: bool, swap_ab: bool) -> Self {
        let gilrs = if enabled {
            match Gilrs::new() {
                Ok(gilrs) => {
                    log::info!("Gamepad support enabled");
                    Some(gilrs)
                }
                Err(error) => {
                    log::warn!("Gamepad support unavailable: {error}");
                    None
                }
            }
        } else {
            None
        };
        Self { gilrs, swap_ab }
    }

    pub fn pressed_buttons(&mut self) -> u32 {
        let Some(gilrs) = self.gilrs.as_mut() else {
            return 0;
        };

        while let Some(event) = gilrs.next_event() {
            match event.event {
                EventType::Connected => {
                    log::info!("Gamepad connected: {}", gilrs.gamepad(event.id).name());
                }
                EventType::Disconnected => {
                    log::info!("Gamepad disconnected: {}", event.id);
                }
                _ => {}
            }
        }

        let mut buttons = 0;
        for (_id, gamepad) in gilrs.gamepads() {
            if !gamepad.is_connected() {
                continue;
            }
            buttons = map_buttons(&gamepad);
            break;
        }

        if self.swap_ab {
            let without_ab = buttons & !(BUTTON_A | BUTTON_B);
            without_ab
                | if buttons & BUTTON_A != 0 { BUTTON_B } else { 0 }
                | if buttons & BUTTON_B != 0 { BUTTON_A } else { 0 }
        } else {
            buttons
        }
    }
}

fn map_buttons(gamepad: &gilrs::Gamepad<'_>) -> u32 {
    let mut buttons = 0;

    if gamepad.is_pressed(Button::DPadUp)
        || stick_axis(gamepad, Axis::LeftStickY) > STICK_DEADZONE
        || stick_axis(gamepad, Axis::RightStickY) > STICK_DEADZONE
    {
        buttons |= BUTTON_UP;
    }
    if gamepad.is_pressed(Button::DPadDown)
        || stick_axis(gamepad, Axis::LeftStickY) < -STICK_DEADZONE
        || stick_axis(gamepad, Axis::RightStickY) < -STICK_DEADZONE
    {
        buttons |= BUTTON_DOWN;
    }
    if gamepad.is_pressed(Button::DPadLeft)
        || stick_axis(gamepad, Axis::LeftStickX) < -STICK_DEADZONE
        || stick_axis(gamepad, Axis::RightStickX) < -STICK_DEADZONE
    {
        buttons |= BUTTON_LEFT;
    }
    if gamepad.is_pressed(Button::DPadRight)
        || stick_axis(gamepad, Axis::LeftStickX) > STICK_DEADZONE
        || stick_axis(gamepad, Axis::RightStickX) > STICK_DEADZONE
    {
        buttons |= BUTTON_RIGHT;
    }

    if gamepad.is_pressed(Button::East) {
        buttons |= BUTTON_A;
    }
    if gamepad.is_pressed(Button::South) {
        buttons |= BUTTON_B;
    }
    if gamepad.is_pressed(Button::North) {
        buttons |= BUTTON_X;
    }
    if gamepad.is_pressed(Button::West) {
        buttons |= BUTTON_Y;
    }
    if gamepad.is_pressed(Button::Start) {
        buttons |= BUTTON_START;
    }
    if gamepad.is_pressed(Button::Select) {
        buttons |= BUTTON_SELECT;
    }
    if gamepad.is_pressed(Button::LeftTrigger) {
        buttons |= BUTTON_L;
    }
    if gamepad.is_pressed(Button::RightTrigger) {
        buttons |= BUTTON_R;
    }

    buttons
}

fn stick_axis(gamepad: &gilrs::Gamepad<'_>, axis: Axis) -> f32 {
    gamepad.value(axis)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idle_buttons() -> u32 {
        0
    }

    #[test]
    fn disabled_mapper_reports_no_buttons() {
        let mut mapper = GamepadMapper::new(false, false);
        assert_eq!(mapper.pressed_buttons(), idle_buttons());
    }

    #[test]
    fn stick_deadzone_threshold_rejects_small_values() {
        assert!(0.4_f32.abs() < STICK_DEADZONE);
        assert!(0.5_f32.abs() >= STICK_DEADZONE);
    }

    #[test]
    fn swap_ab_exchanges_face_button_masks() {
        // Logical mapping used by the frontend when --swap-ab is set.
        let buttons = BUTTON_A;
        let without_ab = buttons & !(BUTTON_A | BUTTON_B);
        let swapped = without_ab
            | if buttons & BUTTON_A != 0 { BUTTON_B } else { 0 }
            | if buttons & BUTTON_B != 0 { BUTTON_A } else { 0 };
        assert_eq!(swapped, BUTTON_B);
    }
}
