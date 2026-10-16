//! Reads the text of game frames on an NVIDIA Jetson's GPU, with the
//! PP-OCRv5 models run by TensorRT.
//!
//!   champollion-gpu-ocr [--fps 4] [--size 1920x1080] [--workers 2]
//!       [--det-width 960] [--det det.engine] [--batch 16] [--frames]
//!       [--game-text GAME_TEXT.sqlite] MODELS_DIR (RECORDING | FRAME.png | --stdin)...
//!
//! Recordings are decoded by the hardware decoder, keeping `--fps` frames
//! per second, scaled to `--size` if given; `--stdin` reads one recording
//! from the standard input. MODELS_DIR has the detection engine
//! (`det.engine` unless `--det` names another), `rec.engine` and the
//! recognizer's `dict.txt`. Each worker loads its own copy of the engines,
//! so the GPU is kept busy while another worker uses the CPU.
//!
//! Prints one line per word (source, x, y, confidence, word) and a speed
//! summary, or with `--frames` one JSON object per frame, in order, with its
//! time and its text in Tesseract's TSV format: what the daemon reads. With
//! `--game-text` (made by `champollion-game-text import`), each line found in
//! the game's own text is replaced by it first.

mod correct;
mod detect;
mod engine;
mod recognize;
mod tesseract;
mod video;

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Barrier, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use image::RgbImage;

use detect::{Detector, TextBox};
use engine::Engine;
use recognize::{Recognizer, Word};

/// A frame to read.
struct Frame {
    /// Position among all the frames read, for the output's order.
    index: u64,
    /// Where it comes from, for the output.
    source: String,
    /// Time in its recording.
    seconds: f64,
    image: RgbImage,
}

/// What a worker read on a frame.
struct Text {
    index: u64,
    source: String,
    seconds: f64,
    lines: Vec<TextBox>,
    words: Vec<Vec<Word>>,
}

enum Input {
    Path(PathBuf),
    Stdin,
}

struct Args {
    models: PathBuf,
    inputs: Vec<Input>,
    fps: u32,
    size: Option<(u32, u32)>,
    workers: usize,
    det_width: u32,
    det_engine: String,
    batch: usize,
    frames: bool,
    game_text: Option<PathBuf>,
}

const USAGE: &str = "usage: champollion-gpu-ocr [--fps N] [--size WxH] [--workers N] [--det-width N] \
                     [--det FILE] [--batch N] [--frames] [--game-text FILE] MODELS_DIR (RECORDING | FRAME.png | --stdin)...";

fn parse_args() -> Result<Args, String> {
    let mut fps = 4;
    let mut size = None;
    let mut workers = 2;
    let mut det_width = 960;
    let mut det_engine = "det.engine".to_owned();
    let mut batch = 16;
    let mut frames = false;
    let mut game_text = None;
    let mut models = None;
    let mut inputs = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--fps" => fps = number(args.next(), &arg)?,
            "--size" => {
                size = args
                    .next()
                    .and_then(|v| {
                        let (w, h) = v.split_once('x')?;
                        Some((w.parse().ok()?, h.parse().ok()?))
                    })
                    .map(Some)
                    .ok_or("--size takes WIDTHxHEIGHT")?
            }
            "--workers" => workers = number(args.next(), &arg)?,
            "--det-width" => det_width = number(args.next(), &arg)?,
            "--det" => det_engine = args.next().ok_or("--det takes a file name")?,
            "--batch" => batch = number(args.next(), &arg)?,
            "--frames" => frames = true,
            "--game-text" => {
                game_text = Some(PathBuf::from(
                    args.next().ok_or("--game-text takes a file name")?,
                ))
            }
            "--stdin" => inputs.push(Input::Stdin),
            _ if models.is_none() => models = Some(PathBuf::from(arg)),
            _ => inputs.push(Input::Path(PathBuf::from(arg))),
        }
    }
    let (Some(models), false) = (models, inputs.is_empty()) else {
        return Err(USAGE.into());
    };
    Ok(Args {
        models,
        inputs,
        fps: fps.max(1),
        size,
        workers: workers.max(1),
        det_width,
        det_engine,
        batch,
        frames,
        game_text,
    })
}

fn number<T: std::str::FromStr>(value: Option<String>, flag: &str) -> Result<T, String> {
    value
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| format!("{flag} takes a number"))
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("champollion-gpu-ocr: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let args = parse_args()?;
    let dictionary = std::fs::read_to_string(args.models.join("dict.txt"))
        .map_err(|e| format!("cannot read dict.txt: {e}"))?;
    let mut corrector = args
        .game_text
        .as_deref()
        .map(correct::Corrector::load)
        .transpose()?;
    if let Some(corrector) = &corrector {
        eprintln!(
            "correcting lines with {} lines of the game's text",
            corrector.lines()
        );
    }
    let (frames_tx, frames_rx) = sync_channel::<Frame>(args.workers * 2);
    let frames_rx = Arc::new(Mutex::new(frames_rx));
    let (texts_tx, texts_rx) = sync_channel::<Text>(args.workers * 2);
    // Times start once every worker has loaded its engines.
    let ready = Barrier::new(args.workers + 1);

    let (times, decoded) = thread::scope(|scope| {
        let workers: Vec<_> = (0..args.workers)
            .map(|_| {
                let (args, dictionary, ready) = (&args, &dictionary, &ready);
                let frames_rx = Arc::clone(&frames_rx);
                let texts_tx = texts_tx.clone();
                scope.spawn(move || {
                    let loaded = load(args, dictionary);
                    ready.wait();
                    let (mut detector, mut recognizer) = loaded?;
                    work(&mut detector, &mut recognizer, &frames_rx, &texts_tx)
                })
            })
            .collect();
        drop(texts_tx);
        ready.wait();
        let start = Instant::now();
        let feeder = scope.spawn(|| feed(&args, frames_tx));
        let printed = print(&args, texts_rx, corrector.as_mut());
        let mut times = Times::default();
        for worker in workers {
            times.add(&worker.join().map_err(|_| "a reader panicked")??);
        }
        times.wall = start.elapsed();
        let decoded = feeder.join().map_err(|_| "the decoder panicked")?;
        printed?;
        Ok::<_, String>((times, decoded))
    })?;
    decoded?;
    times.report();
    Ok(())
}

fn load(args: &Args, dictionary: &str) -> Result<(Detector, Recognizer), String> {
    let detector = Detector::new(
        Engine::open(&args.models.join(&args.det_engine))?,
        args.det_width,
    );
    let recognizer = Recognizer::new(
        Engine::open(&args.models.join("rec.engine"))?,
        dictionary,
        args.batch,
    );
    Ok((detector, recognizer))
}

/// Sends the frames of every input to the workers, numbered in order;
/// dropping `frames` at the end tells them to stop.
fn feed(args: &Args, frames: SyncSender<Frame>) -> Result<(), String> {
    let mut index = 0;
    let mut send = |source: String, seconds: f64, image: RgbImage| {
        let frame = Frame {
            index,
            source,
            seconds,
            image,
        };
        index += 1;
        frames
            .send(frame)
            .map_err(|_| "the readers stopped".to_owned())
    };
    for input in &args.inputs {
        match input {
            Input::Path(path) if is_png(path) => {
                let image = image::open(path)
                    .map_err(|e| format!("cannot read {}: {e}", path.display()))?
                    .to_rgb8();
                send(path.display().to_string(), 0.0, image)?;
            }
            Input::Path(path) => {
                let name = path.display().to_string();
                video::decode(path, args.fps, args.size, &mut |seconds, image| {
                    send(format!("{name}+{seconds:.2}"), seconds, image)
                })?;
            }
            Input::Stdin => {
                // Written to a file first, so the demuxer can seek, as it may
                // need to.
                let path = std::env::temp_dir()
                    .join(format!("champollion-gpu-ocr-{}", std::process::id()));
                let copied = std::fs::File::create(&path)
                    .and_then(|mut file| std::io::copy(&mut std::io::stdin().lock(), &mut file))
                    .map_err(|e| format!("cannot copy the recording from stdin: {e}"));
                let result = copied.and_then(|_| {
                    video::decode(&path, args.fps, args.size, &mut |seconds, image| {
                        send(format!("stdin+{seconds:.2}"), seconds, image)
                    })
                });
                let _ = std::fs::remove_file(&path);
                result?;
            }
        }
    }
    Ok(())
}

fn is_png(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("png"))
}

/// Reads frames until there are none left, sending what they show.
fn work(
    detector: &mut Detector,
    recognizer: &mut Recognizer,
    frames: &Mutex<Receiver<Frame>>,
    texts: &SyncSender<Text>,
) -> Result<Times, String> {
    let mut times = Times::default();
    loop {
        let Ok(frame) = frames.lock().unwrap().recv() else {
            return Ok(times);
        };
        let start = Instant::now();
        let lines = detector.detect(&frame.image)?;
        let detected = Instant::now();
        let words = recognizer.read(&frame.image, &lines)?;
        times.frames += 1;
        times.lines += lines.len();
        times.detect += detected - start;
        times.read += detected.elapsed();
        let text = Text {
            index: frame.index,
            source: frame.source,
            seconds: frame.seconds,
            lines,
            words,
        };
        if texts.send(text).is_err() {
            return Ok(times);
        }
    }
}

/// Prints what the workers read, in the order of the frames.
fn print(
    args: &Args,
    texts: Receiver<Text>,
    mut corrector: Option<&mut correct::Corrector>,
) -> Result<(), String> {
    let mut pending = BTreeMap::new();
    let mut next = 0;
    let mut out = std::io::stdout().lock();
    for text in texts {
        pending.insert(text.index, text);
        while let Some(text) = pending.remove(&next) {
            next += 1;
            let written = if args.frames {
                let mut tsv = tesseract::tsv(&text.lines, &text.words);
                if let Some(corrector) = corrector.as_deref_mut() {
                    tsv = corrector.correct(&tsv);
                }
                let frame = serde_json::json!({
                    "seconds": text.seconds,
                    "tsv": tsv,
                });
                writeln!(out, "{frame}").and_then(|()| out.flush())
            } else {
                text.lines
                    .iter()
                    .zip(&text.words)
                    .flat_map(|(line, words)| words.iter().map(move |w| (line, w)))
                    .try_for_each(|(line, word)| {
                        writeln!(
                            out,
                            "{}\t{}\t{}\t{:.1}\t{}",
                            text.source, line.x, line.y, word.confidence, word.text
                        )
                    })
            };
            written.map_err(|e| format!("cannot write the output: {e}"))?;
        }
    }
    Ok(())
}

#[derive(Default)]
struct Times {
    frames: u32,
    lines: usize,
    detect: Duration,
    read: Duration,
    /// From the engines being loaded to the last frame read.
    wall: Duration,
}

impl Times {
    fn add(&mut self, other: &Times) {
        self.frames += other.frames;
        self.lines += other.lines;
        self.detect += other.detect;
        self.read += other.read;
    }

    fn report(&self) {
        if self.frames == 0 {
            return;
        }
        let per_frame = |d: Duration| d.as_secs_f64() * 1000.0 / self.frames as f64;
        eprintln!(
            "{} frames, {:.1} lines each, in {:.1} s: {:.0} ms per frame overall \
             ({:.1} frames/s); per frame and worker: detect {:.0} ms, read {:.0} ms",
            self.frames,
            self.lines as f64 / self.frames as f64,
            self.wall.as_secs_f64(),
            per_frame(self.wall),
            self.frames as f64 / self.wall.as_secs_f64(),
            per_frame(self.detect),
            per_frame(self.read),
        );
    }
}
