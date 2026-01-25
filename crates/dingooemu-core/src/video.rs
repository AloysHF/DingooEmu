/// Video constants for Dingoo A320
pub const SCREEN_WIDTH: u32 = 320;
pub const SCREEN_HEIGHT: u32 = 240;
pub const FRAMEBUFFER_SIZE: usize = (SCREEN_WIDTH * SCREEN_HEIGHT * 2) as usize; // RGB565

/// Video subsystem
pub struct Video {
    /// Framebuffer in RGB565 format
    framebuffer: Box<[u8]>,
    /// Framebuffer base address in guest memory
    framebuffer_addr: u32,
    /// Frame count for FPS tracking
    frame_count: u64,
}

impl Video {
    /// Create a new video subsystem
    pub fn new(framebuffer_addr: u32) -> Self {
        Self {
            framebuffer: vec![0u8; FRAMEBUFFER_SIZE].into_boxed_slice(),
            framebuffer_addr,
            frame_count: 0,
        }
    }

    /// Get the framebuffer base address
    pub fn framebuffer_addr(&self) -> u32 {
        self.framebuffer_addr
    }

    /// Get a reference to the framebuffer
    pub fn framebuffer(&self) -> &[u8] {
        &self.framebuffer
    }

    /// Get a mutable reference to the framebuffer
    pub fn framebuffer_mut(&mut self) -> &mut [u8] {
        &mut self.framebuffer
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
        Self::new(0x0300_0000) // Default framebuffer address
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_video_creation() {
        let video = Video::new(0x0300_0000);
        assert_eq!(video.framebuffer_addr(), 0x0300_0000);
        assert_eq!(video.framebuffer().len(), FRAMEBUFFER_SIZE);
    }

    #[test]
    fn test_rgb565_conversion() {
        let mut video = Video::new(0);
        // Set a white pixel (0xFFFF in RGB565)
        video.framebuffer_mut()[0] = 0xFF;
        video.framebuffer_mut()[1] = 0xFF;

        let xrgb = video.to_xrgb8888();
        assert_eq!(xrgb[0], 0xFFFF_FFFF); // White
    }
}
