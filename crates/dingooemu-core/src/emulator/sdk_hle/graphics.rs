use super::{Emulator, HandlerResult};
use crate::error::Result;

pub(super) fn handle(emu: &mut Emulator, func_name: &str) -> Result<HandlerResult> {
    match func_name {
        "_lcd_get_frame" | "lcd_get_frame" | "lcd_get_cframe" => {
            let address = crate::video::VM_LCD_FB_ADDRESS;
            emu.cpu.regs.write(2, address);
            log::trace!("  lcd_get_frame() = {address:#010x}");
        }
        "_lcd_set_frame" | "lcd_set_frame" | "ap_lcd_set_frame" => {
            emu.sync_framebuffer();
            log::trace!("  lcd_set_frame() - framebuffer updated");
        }
        "lcd_flip" => {
            emu.sync_framebuffer();
            log::trace!("  lcd_flip() - framebuffer updated");
        }
        "LcdGetDisMode" => {
            emu.cpu.regs.write(2, 0);
            log::trace!("  LcdGetDisMode() = 0");
        }
        "LCD_GetXSize" => {
            emu.cpu.regs.write(2, crate::video::SCREEN_WIDTH);
            log::trace!("  LCD_GetXSize() = {}", crate::video::SCREEN_WIDTH);
        }
        "LCD_GetYSize" => {
            emu.cpu.regs.write(2, crate::video::SCREEN_HEIGHT);
            log::trace!("  LCD_GetYSize() = {}", crate::video::SCREEN_HEIGHT);
        }
        _ => return Ok(HandlerResult::NotHandled),
    }
    Ok(HandlerResult::Complete)
}
