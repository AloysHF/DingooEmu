use clap::ValueEnum;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
pub enum ScaleFilter {
    #[default]
    Nearest,
    Bilinear,
    Bicubic,
    Xbrz,
}

pub struct DisplayScaler {
    filter: ScaleFilter,
    scaled: Vec<u32>,
    presentation: Vec<u32>,
}

impl DisplayScaler {
    pub fn new(filter: ScaleFilter) -> Self {
        Self {
            filter,
            scaled: Vec::new(),
            presentation: Vec::new(),
        }
    }

    pub fn render(
        &mut self,
        source: &[u32],
        source_width: usize,
        source_height: usize,
        window_width: usize,
        window_height: usize,
    ) -> &[u32] {
        let window_width = window_width.max(1);
        let window_height = window_height.max(1);
        let (width, height) = fit_aspect(source_width, source_height, window_width, window_height);
        self.scaled.resize(width * height, 0);
        scale(
            self.filter,
            source,
            source_width,
            source_height,
            width,
            height,
            &mut self.scaled,
        );

        self.presentation.resize(window_width * window_height, 0);
        self.presentation.fill(0);
        let offset_x = (window_width - width) / 2;
        let offset_y = (window_height - height) / 2;
        for row in 0..height {
            let source_start = row * width;
            let destination_start = (row + offset_y) * window_width + offset_x;
            self.presentation[destination_start..destination_start + width]
                .copy_from_slice(&self.scaled[source_start..source_start + width]);
        }
        &self.presentation
    }
}

fn fit_aspect(
    source_width: usize,
    source_height: usize,
    window_width: usize,
    window_height: usize,
) -> (usize, usize) {
    if window_width * source_height <= window_height * source_width {
        (
            window_width,
            (window_width * source_height / source_width).max(1),
        )
    } else {
        (
            (window_height * source_width / source_height).max(1),
            window_height,
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn scale(
    filter: ScaleFilter,
    source: &[u32],
    source_width: usize,
    source_height: usize,
    width: usize,
    height: usize,
    output: &mut [u32],
) {
    for destination_y in 0..height {
        for destination_x in 0..width {
            let source_x = (destination_x as f32 + 0.5) * source_width as f32 / width as f32 - 0.5;
            let source_y =
                (destination_y as f32 + 0.5) * source_height as f32 / height as f32 - 0.5;
            output[destination_y * width + destination_x] = match filter {
                ScaleFilter::Nearest => {
                    sample_nearest(source, source_width, source_height, source_x, source_y)
                }
                ScaleFilter::Bilinear => {
                    sample_bilinear(source, source_width, source_height, source_x, source_y)
                }
                ScaleFilter::Bicubic => {
                    sample_bicubic(source, source_width, source_height, source_x, source_y)
                }
                ScaleFilter::Xbrz => {
                    sample_edge_aware(source, source_width, source_height, source_x, source_y)
                }
            };
        }
    }
}

fn pixel(source: &[u32], width: usize, height: usize, x: i32, y: i32) -> u32 {
    let x = x.clamp(0, width as i32 - 1) as usize;
    let y = y.clamp(0, height as i32 - 1) as usize;
    source[y * width + x]
}

fn sample_nearest(source: &[u32], width: usize, height: usize, x: f32, y: f32) -> u32 {
    pixel(source, width, height, x.round() as i32, y.round() as i32)
}

fn sample_bilinear(source: &[u32], width: usize, height: usize, x: f32, y: f32) -> u32 {
    let x0 = x.floor() as i32;
    let y0 = y.floor() as i32;
    let tx = x - x.floor();
    let ty = y - y.floor();
    blend(
        &[
            pixel(source, width, height, x0, y0),
            pixel(source, width, height, x0 + 1, y0),
            pixel(source, width, height, x0, y0 + 1),
            pixel(source, width, height, x0 + 1, y0 + 1),
        ],
        &[
            (1.0 - tx) * (1.0 - ty),
            tx * (1.0 - ty),
            (1.0 - tx) * ty,
            tx * ty,
        ],
    )
}

fn sample_bicubic(source: &[u32], width: usize, height: usize, x: f32, y: f32) -> u32 {
    let base_x = x.floor() as i32;
    let base_y = y.floor() as i32;
    let mut pixels = [0; 16];
    let mut weights = [0.0; 16];
    let mut index = 0;
    for offset_y in -1..=2 {
        for offset_x in -1..=2 {
            pixels[index] = pixel(source, width, height, base_x + offset_x, base_y + offset_y);
            weights[index] =
                cubic(x - (base_x + offset_x) as f32) * cubic(y - (base_y + offset_y) as f32);
            index += 1;
        }
    }
    blend(&pixels, &weights)
}

fn cubic(value: f32) -> f32 {
    let value = value.abs();
    if value <= 1.0 {
        1.5 * value.powi(3) - 2.5 * value.powi(2) + 1.0
    } else if value < 2.0 {
        -0.5 * value.powi(3) + 2.5 * value.powi(2) - 4.0 * value + 2.0
    } else {
        0.0
    }
}

fn sample_edge_aware(source: &[u32], width: usize, height: usize, x: f32, y: f32) -> u32 {
    let center_x = x.round() as i32;
    let center_y = y.round() as i32;
    let center = pixel(source, width, height, center_x, center_y);
    let horizontal = if x < center_x as f32 {
        pixel(source, width, height, center_x - 1, center_y)
    } else {
        pixel(source, width, height, center_x + 1, center_y)
    };
    let vertical = if y < center_y as f32 {
        pixel(source, width, height, center_x, center_y - 1)
    } else {
        pixel(source, width, height, center_x, center_y + 1)
    };
    if horizontal == vertical && color_distance(center, horizontal) > 48 {
        blend(&[center, horizontal], &[0.5, 0.5])
    } else {
        center
    }
}

fn color_distance(left: u32, right: u32) -> u32 {
    [0, 8, 16]
        .into_iter()
        .map(|shift| ((left >> shift) & 0xff).abs_diff((right >> shift) & 0xff))
        .sum()
}

fn blend(pixels: &[u32], weights: &[f32]) -> u32 {
    let channel = |shift: u32| {
        pixels
            .iter()
            .zip(weights)
            .map(|(pixel, weight)| ((*pixel >> shift) & 0xffu32) as f32 * weight)
            .sum::<f32>()
            .round()
            .clamp(0.0, 255.0) as u32
    };
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_preserve_a_uniform_frame() {
        for filter in [
            ScaleFilter::Nearest,
            ScaleFilter::Bilinear,
            ScaleFilter::Bicubic,
            ScaleFilter::Xbrz,
        ] {
            let mut scaler = DisplayScaler::new(filter);
            assert_eq!(
                scaler.render(&[0x00123456; 4], 2, 2, 7, 7),
                [0x00123456; 49]
            );
        }
    }

    #[test]
    fn presentation_preserves_aspect_ratio() {
        let mut scaler = DisplayScaler::new(ScaleFilter::Nearest);
        let output = scaler.render(&[0x00123456; 4], 2, 2, 4, 2);
        assert_eq!(
            output,
            [0, 0x00123456, 0x00123456, 0, 0, 0x00123456, 0x00123456, 0]
        );
    }
}
