//! Reads lines of text with the PP-OCRv5 Latin recognition model, which
//! gives, at each position along a line, a probability for every character.

use fast_image_resize::images::Image;
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer};
use image::RgbImage;

use crate::detect::TextBox;
use crate::engine::Engine;

/// Height lines are scaled to, as the model was trained.
const HEIGHT: u32 = 48;
/// Narrowest and widest line the engine was built for (trtexec's
/// `--minShapes` and `--maxShapes`); longer lines are squeezed.
const MIN_WIDTH: u32 = 16;
const MAX_WIDTH: u32 = 1280;

/// A word and how sure the model is of it, from 0 to 100 like Tesseract.
#[derive(Debug, Clone)]
pub struct Word {
    pub text: String,
    pub confidence: f32,
    /// Where it starts and ends along its line, from 0 to 1, estimated from
    /// the positions the model read its first and last letters at.
    pub start: f32,
    pub end: f32,
}

pub struct Recognizer {
    engine: Engine,
    /// Index 0 is CTC's "no character" and the last one a space.
    characters: Vec<String>,
    /// Lines read at once.
    batch: usize,
    resizer: Resizer,
}

impl Recognizer {
    /// `dictionary` has one character per line, in the model's order.
    pub fn new(engine: Engine, dictionary: &str, batch: usize) -> Self {
        let mut characters = vec![String::new()];
        characters.extend(
            dictionary
                .lines()
                .map(|c| c.trim_end_matches('\r').to_owned()),
        );
        characters.push(" ".into());
        Self {
            engine,
            characters,
            batch: batch.max(1),
            resizer: Resizer::new(),
        }
    }

    /// The words of each line, in the order of `lines`.
    pub fn read(&mut self, frame: &RgbImage, lines: &[TextBox]) -> Result<Vec<Vec<Word>>, String> {
        let crops = lines
            .iter()
            .map(|b| crop(&mut self.resizer, frame, b))
            .collect::<Result<Vec<_>, _>>()?;
        // Lines of similar width together, to pad less.
        let mut order: Vec<usize> = (0..crops.len()).collect();
        order.sort_by_key(|&i| crops[i].width());
        let mut words = vec![Vec::new(); crops.len()];
        for chunk in order.chunks(self.batch) {
            let width = chunk
                .iter()
                .map(|&i| crops[i].width())
                .max()
                .unwrap_or(MIN_WIDTH);
            let plane = (HEIGHT * width) as usize;
            // Zero after normalizing is mid-grey, which PaddleOCR pads with.
            let mut input = vec![0.0; chunk.len() * 3 * plane];
            for (n, &i) in chunk.iter().enumerate() {
                normalize_into(
                    &crops[i],
                    &mut input[n * 3 * plane..(n + 1) * 3 * plane],
                    width,
                );
            }
            let (output, shape) = self
                .engine
                .run(&input, &[chunk.len(), 3, HEIGHT as usize, width as usize])?;
            let [_, steps, classes] = shape[..] else {
                return Err(format!("unexpected recognition output shape {shape:?}"));
            };
            if classes != self.characters.len() {
                return Err(format!(
                    "the model knows {classes} characters but the dictionary has {}",
                    self.characters.len()
                ));
            }
            // Each step reads this many pixels of the padded batch.
            let step_width = width as f32 / steps as f32;
            for (n, &i) in chunk.iter().enumerate() {
                let line_width = crops[i].width() as f32;
                words[i] = self.decode(
                    &output[n * steps * classes..(n + 1) * steps * classes],
                    classes,
                    |step| (step as f32 * step_width / line_width).clamp(0.0, 1.0),
                );
            }
        }
        Ok(words)
    }

    /// Greedy CTC decoding: the likeliest character at each position,
    /// repeats merged, then split into words at spaces. `position` turns a
    /// step into a place along the line.
    fn decode(
        &self,
        probabilities: &[f32],
        classes: usize,
        position: impl Fn(usize) -> f32,
    ) -> Vec<Word> {
        let mut words = Vec::new();
        let mut text = String::new();
        let mut confidences = Vec::new();
        // Steps of the current word's first and last letters.
        let mut span: Option<(usize, usize)> = None;
        let mut finish =
            |text: &mut String, confidences: &mut Vec<f32>, span: &mut Option<(usize, usize)>| {
                if let (false, Some((first, last))) = (text.is_empty(), span.take()) {
                    let mean = confidences.iter().sum::<f32>() / confidences.len() as f32;
                    words.push(Word {
                        text: std::mem::take(text),
                        confidence: mean * 100.0,
                        start: position(first),
                        end: position(last + 1),
                    });
                }
                text.clear();
                confidences.clear();
            };
        let mut previous = 0;
        for (step, scores) in probabilities.chunks_exact(classes).enumerate() {
            let (best, probability) = scores
                .iter()
                .enumerate()
                .fold((0, f32::MIN), |a, (i, &p)| if p > a.1 { (i, p) } else { a });
            if best != 0 && best != previous {
                if best == self.characters.len() - 1 {
                    finish(&mut text, &mut confidences, &mut span);
                } else {
                    text.push_str(&self.characters[best]);
                    confidences.push(probability);
                    span = Some(span.map_or((step, step), |(first, _)| (first, step)));
                }
            }
            previous = best;
        }
        finish(&mut text, &mut confidences, &mut span);
        words
    }
}

/// The line cut out of the frame and scaled to the model's height.
fn crop(resizer: &mut Resizer, frame: &RgbImage, line: &TextBox) -> Result<Image<'static>, String> {
    let width = ((HEIGHT as f32 * line.width as f32 / line.height as f32).ceil() as u32)
        .clamp(MIN_WIDTH, MAX_WIDTH);
    let mut scaled = Image::new(width, HEIGHT, PixelType::U8x3);
    let options = ResizeOptions::new()
        .crop(
            line.x as f64,
            line.y as f64,
            line.width as f64,
            line.height as f64,
        )
        .resize_alg(ResizeAlg::Convolution(FilterType::Bilinear));
    resizer
        .resize(frame, &mut scaled, &options)
        .map_err(|e| format!("cannot scale a line: {e}"))?;
    Ok(scaled)
}

/// Writes the line as a 3×HEIGHT×`width` tensor in BGR order, scaled to
/// -1..1, leaving the padding on the right at zero.
fn normalize_into(line: &Image, tensor: &mut [f32], width: u32) {
    let plane = (HEIGHT * width) as usize;
    let line_width = line.width() as usize;
    for (n, pixel) in line.buffer().chunks_exact(3).enumerate() {
        let i = (n / line_width) * width as usize + n % line_width;
        for c in 0..3 {
            tensor[c * plane + i] = pixel[2 - c] as f32 / 127.5 - 1.0;
        }
    }
}
