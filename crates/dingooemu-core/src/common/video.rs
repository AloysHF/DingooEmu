/// Display dimensions shared by the supported devices.
pub const SCREEN_WIDTH: u32 = 320;
pub const SCREEN_HEIGHT: u32 = 240;
pub const FRAMEBUFFER_SIZE: usize = (SCREEN_WIDTH * SCREEN_HEIGHT * 2) as usize; // RGB565

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScreenOrientation {
    #[default]
    Landscape,
    Portrait,
}

impl ScreenOrientation {
    pub const fn dimensions(self) -> (u32, u32) {
        match self {
            Self::Landscape => (SCREEN_WIDTH, SCREEN_HEIGHT),
            Self::Portrait => (SCREEN_HEIGHT, SCREEN_WIDTH),
        }
    }
}

pub fn rotate_rgb565_counterclockwise(source: &[u8], destination: &mut [u8]) {
    assert!(source.len() >= FRAMEBUFFER_SIZE);
    assert!(destination.len() >= FRAMEBUFFER_SIZE);

    let width = SCREEN_WIDTH as usize;
    let height = SCREEN_HEIGHT as usize;
    for y in 0..height {
        for x in 0..width {
            let source_offset = (y * width + x) * 2;
            let destination_offset = ((width - 1 - x) * height + y) * 2;
            destination[destination_offset..destination_offset + 2]
                .copy_from_slice(&source[source_offset..source_offset + 2]);
        }
    }
}

/// Video subsystem
#[derive(serde::Serialize, serde::Deserialize)]
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

    /// Get a reference to the framebuffer
    pub fn framebuffer(&self) -> &[u8] {
        &self.framebuffer
    }

    /// Get a mutable reference to the framebuffer
    pub fn framebuffer_mut(&mut self) -> &mut [u8] {
        &mut self.framebuffer
    }

    pub(crate) fn snapshot_layout_is_valid(&self) -> bool {
        self.framebuffer.len() == FRAMEBUFFER_SIZE
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
        self.to_xrgb8888_oriented(ScreenOrientation::Landscape)
    }

    pub fn to_xrgb8888_oriented(&self, orientation: ScreenOrientation) -> Vec<u32> {
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

        if orientation == ScreenOrientation::Portrait {
            rotate_xrgb8888_counterclockwise(&pixels)
        } else {
            pixels
        }
    }

    /// Calculate a deterministic CRC32 over the raw RGB565 framebuffer.
    pub fn framebuffer_crc32(&self) -> u32 {
        crc32fast::hash(&self.framebuffer)
    }

    /// Save the current framebuffer as a PNG screenshot.
    pub fn save_screenshot(&self, path: &std::path::Path) -> anyhow::Result<()> {
        self.save_screenshot_oriented(path, ScreenOrientation::Landscape)
    }

    pub fn save_screenshot_oriented(
        &self,
        path: &std::path::Path,
        orientation: ScreenOrientation,
    ) -> anyhow::Result<()> {
        use image::RgbaImage;
        let (output_width, output_height) = orientation.dimensions();
        let mut img = RgbaImage::new(output_width, output_height);
        for y in 0..SCREEN_HEIGHT {
            for x in 0..SCREEN_WIDTH {
                let offset = ((y * SCREEN_WIDTH + x) * 2) as usize;
                let rgb565 =
                    u16::from_le_bytes([self.framebuffer[offset], self.framebuffer[offset + 1]]);
                let r5 = (rgb565 >> 11) & 0x1F;
                let g6 = (rgb565 >> 5) & 0x3F;
                let b5 = rgb565 & 0x1F;
                let r = ((r5 << 3) | (r5 >> 2)) as u8;
                let g = ((g6 << 2) | (g6 >> 4)) as u8;
                let b = ((b5 << 3) | (b5 >> 2)) as u8;
                let (output_x, output_y) = match orientation {
                    ScreenOrientation::Landscape => (x, y),
                    ScreenOrientation::Portrait => (y, SCREEN_WIDTH - 1 - x),
                };
                img.put_pixel(output_x, output_y, image::Rgba([r, g, b, 0xFF]));
            }
        }
        img.save(path)?;
        Ok(())
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

fn rotate_xrgb8888_counterclockwise(source: &[u32]) -> Vec<u32> {
    let width = SCREEN_WIDTH as usize;
    let height = SCREEN_HEIGHT as usize;
    let mut destination = vec![0; width * height];
    for y in 0..height {
        for x in 0..width {
            destination[(width - 1 - x) * height + y] = source[y * width + x];
        }
    }
    destination
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

    #[test]
    fn portrait_orientation_rotates_pixels_counterclockwise() {
        let mut video = Video::new();
        let top_left = 0x001f_u16;
        let top_right = 0xf800_u16;
        let bottom_left = 0x07e0_u16;
        let bottom_right = 0xffff_u16;
        for (x, y, color) in [
            (0, 0, top_left),
            (SCREEN_WIDTH - 1, 0, top_right),
            (0, SCREEN_HEIGHT - 1, bottom_left),
            (SCREEN_WIDTH - 1, SCREEN_HEIGHT - 1, bottom_right),
        ] {
            let offset = ((y * SCREEN_WIDTH + x) * 2) as usize;
            video.framebuffer_mut()[offset..offset + 2].copy_from_slice(&color.to_le_bytes());
        }

        let mut rotated = vec![0; FRAMEBUFFER_SIZE];
        rotate_rgb565_counterclockwise(video.framebuffer(), &mut rotated);
        let pixel = |x: u32, y: u32| {
            let offset = ((y * SCREEN_HEIGHT + x) * 2) as usize;
            u16::from_le_bytes([rotated[offset], rotated[offset + 1]])
        };
        assert_eq!(ScreenOrientation::Portrait.dimensions(), (240, 320));
        assert_eq!(pixel(0, 0), top_right);
        assert_eq!(pixel(SCREEN_HEIGHT - 1, 0), bottom_right);
        assert_eq!(pixel(0, SCREEN_WIDTH - 1), top_left);
        assert_eq!(pixel(SCREEN_HEIGHT - 1, SCREEN_WIDTH - 1), bottom_left);

        let xrgb = video.to_xrgb8888_oriented(ScreenOrientation::Portrait);
        assert_eq!(xrgb.len(), (SCREEN_HEIGHT * SCREEN_WIDTH) as usize);
        assert_eq!(xrgb[0], 0xffff_0000);
    }

    #[test]
    fn framebuffer_crc32_tracks_raw_rgb565_pixels() {
        let mut video = Video::new();
        let initial = video.framebuffer_crc32();
        video.framebuffer_mut()[0..2].copy_from_slice(&0xffff_u16.to_le_bytes());

        assert_ne!(video.framebuffer_crc32(), initial);
        assert_eq!(
            video.framebuffer_crc32(),
            crc32fast::hash(video.framebuffer())
        );
    }
}
