use dingooemu_core::input::{
    BUTTON_A, BUTTON_B, BUTTON_DOWN, BUTTON_L, BUTTON_LEFT, BUTTON_R, BUTTON_RIGHT, BUTTON_SELECT,
    BUTTON_START, BUTTON_UP, BUTTON_X, BUTTON_Y,
};

const IDLE: u32 = 0x00404040;
const PRESSED: u32 = 0x0000d8ff;
const ACTION_PRESSED: u32 = 0x00ff9f1a;

pub fn draw(buffer: &mut [u32], width: usize, height: usize, buttons: u32) {
    if width == 0 || height == 0 || buffer.len() < width.saturating_mul(height) {
        return;
    }
    let unit = (width.min(height) / 48).clamp(1, 6);
    let dpad_x = unit * 6;
    let dpad_y = height.saturating_sub(unit * 7);
    button(
        buffer,
        width,
        height,
        dpad_x,
        dpad_y.saturating_sub(unit * 3),
        unit * 2,
        buttons & BUTTON_UP != 0,
        PRESSED,
    );
    button(
        buffer,
        width,
        height,
        dpad_x,
        dpad_y + unit * 3,
        unit * 2,
        buttons & BUTTON_DOWN != 0,
        PRESSED,
    );
    button(
        buffer,
        width,
        height,
        dpad_x - unit * 3,
        dpad_y,
        unit * 2,
        buttons & BUTTON_LEFT != 0,
        PRESSED,
    );
    button(
        buffer,
        width,
        height,
        dpad_x + unit * 3,
        dpad_y,
        unit * 2,
        buttons & BUTTON_RIGHT != 0,
        PRESSED,
    );

    let action_x = width.saturating_sub(unit * 6);
    let action_y = height.saturating_sub(unit * 7);
    for (offset_x, offset_y, mask) in [
        (0isize, -3isize, BUTTON_X),
        (0, 3, BUTTON_B),
        (-3, 0, BUTTON_Y),
        (3, 0, BUTTON_A),
    ] {
        button(
            buffer,
            width,
            height,
            action_x.saturating_add_signed(offset_x * unit as isize),
            action_y.saturating_add_signed(offset_y * unit as isize),
            unit * 2,
            buttons & mask != 0,
            ACTION_PRESSED,
        );
    }

    let system_y = height.saturating_sub(unit * 3);
    button(
        buffer,
        width,
        height,
        (width / 2).saturating_sub(unit * 4),
        system_y,
        unit * 2,
        buttons & BUTTON_SELECT != 0,
        PRESSED,
    );
    button(
        buffer,
        width,
        height,
        width / 2 + unit * 4,
        system_y,
        unit * 2,
        buttons & BUTTON_START != 0,
        PRESSED,
    );
    button(
        buffer,
        width,
        height,
        unit * 3,
        unit * 2,
        unit * 2,
        buttons & BUTTON_L != 0,
        PRESSED,
    );
    button(
        buffer,
        width,
        height,
        width.saturating_sub(unit * 3),
        unit * 2,
        unit * 2,
        buttons & BUTTON_R != 0,
        PRESSED,
    );
}

#[allow(clippy::too_many_arguments)]
fn button(
    buffer: &mut [u32],
    width: usize,
    height: usize,
    center_x: usize,
    center_y: usize,
    radius: usize,
    pressed: bool,
    pressed_color: u32,
) {
    let color = if pressed { pressed_color } else { IDLE };
    let radius_squared = radius * radius;
    for y in center_y.saturating_sub(radius)..=(center_y + radius).min(height - 1) {
        for x in center_x.saturating_sub(radius)..=(center_x + radius).min(width - 1) {
            let distance_x = x.abs_diff(center_x);
            let distance_y = y.abs_diff(center_y);
            if distance_x * distance_x + distance_y * distance_y <= radius_squared {
                buffer[y * width + x] = color;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_highlights_pressed_direction_and_action_buttons() {
        let mut buffer = vec![0; 320 * 240];
        draw(&mut buffer, 320, 240, BUTTON_UP | BUTTON_A);
        assert!(buffer.contains(&PRESSED));
        assert!(buffer.contains(&ACTION_PRESSED));
    }

    #[test]
    fn overlay_clips_tiny_and_short_buffers() {
        let mut tiny = vec![0; 1];
        draw(&mut tiny, 1, 1, u32::MAX);
        let mut short = vec![7; 3];
        draw(&mut short, 2, 2, u32::MAX);
        assert_eq!(short, [7; 3]);
    }
}
