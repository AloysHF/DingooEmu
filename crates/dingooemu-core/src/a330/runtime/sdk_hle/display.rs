use super::super::*;

impl RuntimeBus<'_> {
    pub(super) fn dispatch_display(&mut self, cpu: &mut Cpu, name: &str) -> Result<bool> {
        match name {
            "lcd_get_frame" | "_lcd_get_frame" | "LCDGetFB" => {
                cpu.r[0] = *self.active_framebuffer;
            }
            "LCDGetWidth" | "get_lcd_width" => cpu.r[0] = SCREEN_WIDTH,
            "LCDGetHeight" | "get_lcd_height" => cpu.r[0] = SCREEN_HEIGHT,
            "lcd_set_frame" | "_lcd_set_frame" | "LCDFlushFB" | "LCDFlushFBZoom" => {
                *self.frame_address = Some(*self.active_framebuffer);
                cpu.r[0] = 0;
            }
            "LCDIsDoubleFBEnabled" => cpu.r[0] = 1,
            "LCDGetFBFormat" => cpu.r[0] = 0,
            "LCDSetFBBit" => {
                if matches!(cpu.r[0], 16 | 32) {
                    *self.framebuffer_bits = cpu.r[0] | FRAMEBUFFER_BITS_EXPLICIT;
                }
                cpu.r[0] = 0;
            }
            "BMF_SetLcdFramePtr" => {
                if self.memory.read_bytes(cpu.r[0], FRAMEBUFFER_SIZE).is_ok() {
                    *self.active_framebuffer = cpu.r[0];
                }
                cpu.r[0] = 0;
            }
            "SysLcdClear" => {
                let clear = vec![0; FRAMEBUFFER_SIZE];
                self.write_memory(FRAMEBUFFER_BASE, &clear)?;
                self.write_memory(LEGACY_FRAMEBUFFER_ADDRESS, &clear)?;
                cpu.r[0] = 0;
            }
            "FlushDCache" | "__dcache_writeback_all" => {
                // Homebrew ports flush software framebuffers (including the
                // 0x11800000 alias). Present any readable pointer; otherwise
                // treat the call as a no-op like a cache maintenance op.
                let address = cpu.r[0];
                if address != 0
                    && (self.memory.read_bytes(address, FRAMEBUFFER_SIZE).is_ok()
                        || self
                            .memory
                            .read_bytes(address, FRAMEBUFFER_SIZE * 2)
                            .is_ok())
                {
                    *self.active_framebuffer = address;
                    *self.frame_address = Some(address);
                }
                cpu.r[0] = 0;
            }
            "InvalidICache" => {
                self.clear_instruction_cache();
                cpu.r[0] = 0;
            }
            "BMF_SelectPixelFunc"
            | "LCDEnableDoubleFB"
            | "LCDDisableDoubleFB"
            | "LCDSetFBFormat"
            | "LCDInit"
            | "LCDSetRefreshRate"
            | "LCDSetBrightness"
            | "fsys_RefreshCache"
            | "consoleEnable"
            | "consoleDisable"
            | "PMSetMode" => cpu.r[0] = 0,
            _ => return Ok(false),
        }
        Ok(true)
    }
}
