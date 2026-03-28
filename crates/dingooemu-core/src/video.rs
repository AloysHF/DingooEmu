/// Video constants for Dingoo A320
pub const SCREEN_WIDTH: u32 = 320;
pub const SCREEN_HEIGHT: u32 = 240;
pub const FRAMEBUFFER_SIZE: usize = (SCREEN_WIDTH * SCREEN_HEIGHT * 2) as usize; // RGB565

/// Fixed framebuffer address (like DingooPie's VM_LCD_FB_ADDRESS)
/// The game writes directly to this address
pub const VM_LCD_FB_ADDRESS: u32 = 0x1400_0000;

/// Video subsystem
pub struct Video {
    /// Framebuffer in RGB565 format (host-side copy)
    framebuffer: Box<[u8]>,
    /// Whether the framebuffer has been updated
    fb_dirty: bool,
    /// Frame count for FPS tracking
    frame_count: u64,
}

impl Video {
    /// Create a new video subsystem
    pub fn new() -> Self {
        Self {
            framebuffer: vec![0u8; FRAMEBUFFER_SIZE].into_boxed_slice(),
            fb_dirty: false,
            frame_count: 0,
        }
    }

    /// Get the fixed framebuffer address
    pub fn framebuffer_addr(&self) -> u32 {
        VM_LCD_FB_ADDRESS
    }

    /// Get a reference to the framebuffer
    pub fn framebuffer(&self) -> &[u8] {
        &self.framebuffer
    }

    /// Get a mutable reference to the framebuffer
    pub fn framebuffer_mut(&mut self) -> &mut [u8] {
        &mut self.framebuffer
    }

    /// Mark framebuffer as dirty (needs sync from guest memory)
    pub fn mark_dirty(&mut self) {
        self.fb_dirty = true;
    }

    /// Check if framebuffer needs sync
    pub fn is_dirty(&self) -> bool {
        self.fb_dirty
    }

    /// Clear dirty flag after sync
    pub fn clear_dirty(&mut self) {
        self.fb_dirty = false;
    }

    /// Convert RGB565 framebuffer to XRGB8888 (for rendering)
    pub fn to_xrgb8888(&self) -> Vec<u32> {
        let mut pixels = Vec::with_capacity((SCREEN_WIDTH * SCREEN_HEIGHT) as usize);

        for y in 0..SCREEN_HEIGHT {
            for x in 0..SCREEN_WIDTH {
                let offset = ((y * SCREEN_WIDTH + x) * 2) as usize;
                let rgb565 =
                    u16::from_le_bytes([self.framebuffer[offset], self.framebuffer[offset + 1]]);

                // RGB565 to XRGB8888
                let r = ((rgb565 >> 11) & 0x1F) as u32;
                let g = ((rgb565 >> 5) & 0x3F) as u32;
                let b = (rgb565 & 0x1F) as u32;

                // Expand to 8-bit
                let r8 = (r << 3) | (r >> 2);
                let g8 = (g << 2) | (g >> 4);
                let b8 = (b << 3) | (b >> 2);

                pixels.push((0xFF << 24) | (r8 << 16) | (g8 << 8) | b8);
            }
        }

        pixels
    }

    /// Increment frame counter
    pub fn advance_frame(&mut self) {
        self.frame_count += 1;
    }

    /// Get current frame count
    pub fn frame_count(&self) -> u64 {
        self.frame_count
    }
}

impl Default for Video {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_video_creation() {
        let video = Video::new();
        assert_eq!(video.framebuffer_addr(), VM_LCD_FB_ADDRESS);
        assert_eq!(video.framebuffer().len(), FRAMEBUFFER_SIZE);
    }

    #[test]
    fn test_rgb565_conversion() {
        let mut video = Video::new();
        // Set a white pixel (0xFFFF in RGB565)
        video.framebuffer_mut()[0] = 0xFF;
        video.framebuffer_mut()[1] = 0xFF;

        let xrgb = video.to_xrgb8888();
        assert_eq!(xrgb[0], 0xFFFF_FFFF); // White
    }
}
