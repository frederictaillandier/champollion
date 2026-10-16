//! Finds the lines of text in a frame with the PP-OCRv5 detection model,
//! which gives for each pixel the probability that it is part of text.

use fast_image_resize::images::Image;
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer};
use image::RgbImage;

use crate::engine::Engine;

/// Probability above which a pixel counts as text.
const PIXEL_THRESHOLD: f32 = 0.3;
/// Mean probability under which a region is not text.
const BOX_THRESHOLD: f32 = 0.6;
/// The model marks a shrunk core of each line; regions are grown back by
/// this much (PaddleOCR's "unclip ratio").
const UNCLIP_RATIO: f32 = 1.5;
/// Regions thinner than this, in model pixels, are specks.
const MIN_SIZE: f32 = 3.0;
/// Statistics PaddleOCR normalizes with, applied as listed to the B, G and
/// R channels of the OpenCV images it trained on.
const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const STD: [f32; 3] = [0.229, 0.224, 0.225];

/// A line of text, in frame pixels.
#[derive(Debug, Clone, Copy)]
pub struct TextBox {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

pub struct Detector {
    engine: Engine,
    /// Width frames are scaled to: wider finds smaller text, but is slower.
    width: u32,
    resizer: Resizer,
}

impl Detector {
    pub fn new(engine: Engine, width: u32) -> Self {
        Self {
            engine,
            width,
            resizer: Resizer::new(),
        }
    }

    pub fn detect(&mut self, frame: &RgbImage) -> Result<Vec<TextBox>, String> {
        let width = multiple_of_32(self.width as f32);
        let height =
            multiple_of_32(frame.height() as f32 * self.width as f32 / frame.width() as f32);
        let mut small = Image::new(width, height, PixelType::U8x3);
        self.resizer
            .resize(
                frame,
                &mut small,
                &ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Bilinear)),
            )
            .map_err(|e| format!("cannot scale the frame: {e}"))?;
        let (map, shape) = self.engine.run(
            &normalize(small.buffer()),
            &[1, 3, height as usize, width as usize],
        )?;
        let [_, _, map_height, map_width] = shape[..] else {
            return Err(format!("unexpected detection output shape {shape:?}"));
        };
        let scale_x = frame.width() as f32 / map_width as f32;
        let scale_y = frame.height() as f32 / map_height as f32;
        Ok(regions(&map, map_width, map_height)
            .into_iter()
            .filter_map(|r| {
                let x0 = (r.x0 * scale_x).max(0.0);
                let y0 = (r.y0 * scale_y).max(0.0);
                let x1 = (r.x1 * scale_x).min(frame.width() as f32);
                let y1 = (r.y1 * scale_y).min(frame.height() as f32);
                (x1 - x0 >= 1.0 && y1 - y0 >= 1.0).then(|| TextBox {
                    x: x0 as u32,
                    y: y0 as u32,
                    width: (x1 - x0) as u32,
                    height: (y1 - y0) as u32,
                })
            })
            .collect())
    }
}

fn multiple_of_32(size: f32) -> u32 {
    ((size / 32.0).round() as u32).max(1) * 32
}

/// RGB pixels as a 3×height×width tensor, channels in BGR order like
/// OpenCV loads them, which PaddleOCR trained with.
fn normalize(rgb: &[u8]) -> Vec<f32> {
    let plane = rgb.len() / 3;
    let mut tensor = vec![0.0; rgb.len()];
    for (i, pixel) in rgb.chunks_exact(3).enumerate() {
        for c in 0..3 {
            let value = pixel[2 - c] as f32 / 255.0;
            tensor[c * plane + i] = (value - MEAN[c]) / STD[c];
        }
    }
    tensor
}

/// A region in model pixels.
struct Region {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
}

/// Groups the text pixels of the probability map into lines. Game text is
/// horizontal, so regions are upright rectangles.
fn regions(map: &[f32], width: usize, height: usize) -> Vec<Region> {
    let mut seen = vec![false; map.len()];
    let mut stack = Vec::new();
    let mut found = Vec::new();
    for start in 0..map.len() {
        if seen[start] || map[start] <= PIXEL_THRESHOLD {
            continue;
        }
        seen[start] = true;
        stack.push(start);
        let (mut x0, mut y0, mut x1, mut y1) = (width, height, 0, 0);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % width, i / width);
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx < 0 || ny < 0 || nx >= width as i32 || ny >= height as i32 {
                        continue;
                    }
                    let n = ny as usize * width + nx as usize;
                    if !seen[n] && map[n] > PIXEL_THRESHOLD {
                        seen[n] = true;
                        stack.push(n);
                    }
                }
            }
        }
        let (w, h) = ((x1 - x0 + 1) as f32, (y1 - y0 + 1) as f32);
        if w.min(h) < MIN_SIZE {
            continue;
        }
        let mut sum = 0.0;
        for y in y0..=y1 {
            sum += map[y * width + x0..=y * width + x1].iter().sum::<f32>();
        }
        if sum / (w * h) < BOX_THRESHOLD {
            continue;
        }
        let grow = w * h * UNCLIP_RATIO / (2.0 * (w + h));
        if (w + 2.0 * grow).min(h + 2.0 * grow) < MIN_SIZE + 2.0 {
            continue;
        }
        found.push(Region {
            x0: x0 as f32 - grow,
            y0: y0 as f32 - grow,
            x1: x1 as f32 + 1.0 + grow,
            y1: y1 as f32 + 1.0 + grow,
        });
    }
    found
}
