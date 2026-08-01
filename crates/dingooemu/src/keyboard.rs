use std::str::FromStr;

use dingooemu_core::input::{
    BUTTON_A, BUTTON_B, BUTTON_DOWN, BUTTON_L, BUTTON_LEFT, BUTTON_R, BUTTON_RIGHT, BUTTON_SELECT,
    BUTTON_START, BUTTON_UP, BUTTON_X, BUTTON_Y,
};
use minifb::{Key, Window};

const DEFAULT_MAPPINGS: &[(u32, Key)] = &[
    (BUTTON_UP, Key::Up),
    (BUTTON_DOWN, Key::Down),
    (BUTTON_LEFT, Key::Left),
    (BUTTON_RIGHT, Key::Right),
    (BUTTON_A, Key::X),
    (BUTTON_B, Key::Z),
    (BUTTON_X, Key::S),
    (BUTTON_Y, Key::A),
    (BUTTON_SELECT, Key::RightShift),
    (BUTTON_START, Key::Enter),
    (BUTTON_L, Key::Q),
    (BUTTON_R, Key::W),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RemapSpec {
    button: u32,
    key: Key,
}

impl FromStr for RemapSpec {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (button, key) = value
            .split_once(':')
            .ok_or_else(|| "expected BUTTON:KEY, for example a:space".to_string())?;
        Ok(Self {
            button: parse_button(button.trim())?,
            key: parse_key(key.trim())?,
        })
    }
}

pub struct KeyboardMapper {
    mappings: Vec<(u32, Key)>,
    swap_ab: bool,
}

impl KeyboardMapper {
    pub fn new(remappings: &[RemapSpec], swap_ab: bool) -> Self {
        let mut mappings = DEFAULT_MAPPINGS.to_vec();
        for remapping in remappings {
            mappings.retain(|(button, _)| *button != remapping.button);
            mappings.push((remapping.button, remapping.key));
        }
        Self { mappings, swap_ab }
    }

    pub fn pressed_buttons(&self, window: &Window) -> u32 {
        self.buttons_from_key_state(|key| window.is_key_down(key))
    }

    fn buttons_from_key_state(&self, mut is_down: impl FnMut(Key) -> bool) -> u32 {
        let buttons = self
            .mappings
            .iter()
            .filter(|(_, key)| is_down(*key))
            .fold(0, |buttons, (button, _)| buttons | button);
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

fn parse_button(name: &str) -> Result<u32, String> {
    match name.to_ascii_lowercase().as_str() {
        "up" => Ok(BUTTON_UP),
        "down" => Ok(BUTTON_DOWN),
        "left" => Ok(BUTTON_LEFT),
        "right" => Ok(BUTTON_RIGHT),
        "a" => Ok(BUTTON_A),
        "b" => Ok(BUTTON_B),
        "x" => Ok(BUTTON_X),
        "y" => Ok(BUTTON_Y),
        "start" => Ok(BUTTON_START),
        "select" => Ok(BUTTON_SELECT),
        "l" => Ok(BUTTON_L),
        "r" => Ok(BUTTON_R),
        _ => Err(format!("unknown Dingoo button '{name}'")),
    }
}

fn parse_key(name: &str) -> Result<Key, String> {
    let key = match name.to_ascii_lowercase().as_str() {
        "a" => Key::A,
        "b" => Key::B,
        "c" => Key::C,
        "d" => Key::D,
        "e" => Key::E,
        "f" => Key::F,
        "g" => Key::G,
        "h" => Key::H,
        "i" => Key::I,
        "j" => Key::J,
        "k" => Key::K,
        "l" => Key::L,
        "m" => Key::M,
        "n" => Key::N,
        "o" => Key::O,
        "p" => Key::P,
        "q" => Key::Q,
        "r" => Key::R,
        "s" => Key::S,
        "t" => Key::T,
        "u" => Key::U,
        "v" => Key::V,
        "w" => Key::W,
        "x" => Key::X,
        "y" => Key::Y,
        "z" => Key::Z,
        "0" => Key::Key0,
        "1" => Key::Key1,
        "2" => Key::Key2,
        "3" => Key::Key3,
        "4" => Key::Key4,
        "5" => Key::Key5,
        "6" => Key::Key6,
        "7" => Key::Key7,
        "8" => Key::Key8,
        "9" => Key::Key9,
        "f1" => Key::F1,
        "f2" => Key::F2,
        "f3" => Key::F3,
        "f4" => Key::F4,
        "f5" => Key::F5,
        "f6" => Key::F6,
        "f7" => Key::F7,
        "f8" => Key::F8,
        "f9" => Key::F9,
        "f10" => Key::F10,
        "f11" => Key::F11,
        "f12" => Key::F12,
        "up" => Key::Up,
        "down" => Key::Down,
        "left" => Key::Left,
        "right" => Key::Right,
        "space" => Key::Space,
        "enter" | "return" => Key::Enter,
        "backspace" => Key::Backspace,
        "tab" => Key::Tab,
        "delete" => Key::Delete,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        "leftshift" => Key::LeftShift,
        "rightshift" => Key::RightShift,
        "leftctrl" => Key::LeftCtrl,
        "rightctrl" => Key::RightCtrl,
        "leftalt" => Key::LeftAlt,
        "rightalt" => Key::RightAlt,
        "escape" | "esc" => return Err("escape is reserved for exiting the emulator".to_string()),
        _ => return Err(format!("unknown key '{name}'")),
    };
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_mapping_matches_retroarch_keyboard_defaults() {
        let mapper = KeyboardMapper::new(&[], false);
        for (key, button) in [
            (Key::Up, BUTTON_UP),
            (Key::Down, BUTTON_DOWN),
            (Key::Left, BUTTON_LEFT),
            (Key::Right, BUTTON_RIGHT),
            (Key::X, BUTTON_A),
            (Key::Z, BUTTON_B),
            (Key::S, BUTTON_X),
            (Key::A, BUTTON_Y),
            (Key::Enter, BUTTON_START),
            (Key::RightShift, BUTTON_SELECT),
            (Key::Q, BUTTON_L),
            (Key::W, BUTTON_R),
        ] {
            assert_eq!(
                mapper.buttons_from_key_state(|pressed| pressed == key),
                button
            );
        }
    }

    #[test]
    fn remapping_replaces_all_default_keys_for_a_button() {
        let mapper = KeyboardMapper::new(&["a:space".parse().unwrap()], false);
        assert_eq!(
            mapper.buttons_from_key_state(|key| key == Key::Space),
            BUTTON_A
        );
        assert_eq!(mapper.buttons_from_key_state(|key| key == Key::X), 0);
    }

    #[test]
    fn swap_ab_exchanges_logical_button_masks() {
        let mapper = KeyboardMapper::new(&[], true);
        assert_eq!(mapper.buttons_from_key_state(|key| key == Key::X), BUTTON_B);
        assert_eq!(mapper.buttons_from_key_state(|key| key == Key::Z), BUTTON_A);
    }

    #[test]
    fn parser_accepts_every_button_and_rejects_escape() {
        for button in [
            "up", "down", "left", "right", "a", "b", "x", "y", "start", "select", "l", "r",
        ] {
            assert!(format!("{button}:space").parse::<RemapSpec>().is_ok());
        }
        assert!("a:escape".parse::<RemapSpec>().is_err());
    }
}
