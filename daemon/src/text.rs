//! Reads the text in finished recordings while the game is not running.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::sync_channel;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app::AppSink;
use image::{RgbImage, imageops};
use serde::Serialize;

use crate::ocr::{Ocr, Reader, Word};

pub struct Config {
    pub recordings_dir: PathBuf,
    pub text_dir: PathBuf,
    /// Frames read per second of video.
    pub samples_per_second: u32,
    /// Frames read in parallel.
    pub workers: usize,
    pub ocr: Ocr,
}

/// Background thread reading recordings; it only works while `allowed`.
pub struct Indexer {
    pub allowed: Arc<AtomicBool>,
    /// What the indexer is doing, for the tray; `None` when idle.
    pub status: Arc<Mutex<Option<String>>>,
    handle: thread::JoinHandle<()>,
}

impl Indexer {
    pub fn spawn(config: Config, stop: Arc<AtomicBool>) -> Self {
        let allowed = Arc::new(AtomicBool::new(false));
        let status = Arc::new(Mutex::new(None));
        let ctx = Context {
            config,
            stop,
            allowed: allowed.clone(),
            status: status.clone(),
        };
        let handle = thread::Builder::new()
            .name("text-indexer".into())
            .spawn(move || ctx.run())
            .expect("spawn text indexer thread");
        Self {
            allowed,
            status,
            handle,
        }
    }

    pub fn join(self) {
        let _ = self.handle.join();
    }
}

struct Context {
    config: Config,
    stop: Arc<AtomicBool>,
    allowed: Arc<AtomicBool>,
    status: Arc<Mutex<Option<String>>>,
}

/// Output files of one game, kept in memory across its videos.
struct GameText {
    dir: PathBuf,
    vocabulary: HashSet<String>,
    /// Words of the screens saved since the daemon started, so a screen seen
    /// in every session is saved once.
    screens: Vec<HashSet<String>>,
    /// Screens not to read at all, from `skip-screens.txt`.
    skip: Vec<HashSet<String>>,
}

/// Written to `skip-screens.txt` the first time a game is read.
const SKIP_SCREENS_HEADER: &str = "\
# Screens whose text is not read: one screen per line, as words it shows.
# A frame containing at least 3 of a line's words (or all of them, if the
# line has fewer) is skipped: not saved, and its words are not added to the
# vocabulary. Lines starting with # are ignored.

# Steam's window, visible before the game's window appears.
store library community
";
/// Kingdom Come: Deliverance II's main menu; its pause menu shares the items.
const KCD2_MENU: &str = "\
# Main and pause menus of Kingdom Come: Deliverance II.
pokračovat nahrát nastavení nápověda autoři ukončit
";

impl GameText {
    fn load(dir: PathBuf) -> Self {
        let vocabulary = fs::read_to_string(dir.join("vocabulary.tsv"))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| l.split('\t').next())
            .map(str::to_owned)
            .collect();
        let skip = load_skip_screens(&dir);
        Self {
            dir,
            vocabulary,
            screens: Vec::new(),
            skip,
        }
    }

    /// Whether the frame shows one of the screens of `skip-screens.txt`.
    fn is_skipped_screen(&self, words: &[Word]) -> bool {
        let keys: HashSet<String> = words.iter().map(Word::key).collect();
        self.skip
            .iter()
            .any(|screen| screen.intersection(&keys).count() >= screen.len().min(3))
    }

    /// Returns true, and remembers the screen, unless a screen with nearly
    /// the same words was already saved.
    fn first_time_screen(&mut self, words: &[Word]) -> bool {
        let screen: HashSet<String> = words.iter().map(Word::key).collect();
        let repeat = self.screens.iter().any(|saved| {
            let common = saved.intersection(&screen).count() as f64;
            let all = saved.union(&screen).count().max(1) as f64;
            common / all >= SAME_SCREEN
        });
        if !repeat {
            self.screens.push(screen);
        }
        !repeat
    }

    fn processed(dir: &Path) -> HashSet<String> {
        fs::read_to_string(dir.join("processed.txt"))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| l.split('\t').next())
            .map(str::to_owned)
            .collect()
    }

    fn append(&self, file: &str, line: &str) {
        let path = self.dir.join(file);
        let result = fs::create_dir_all(&self.dir).and_then(|()| {
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)?
                .write_all(line.as_bytes())
        });
        if let Err(e) = result {
            tracing::error!("could not write {}: {e}", path.display());
        }
    }
}

/// A video waiting to be read.
struct Video {
    path: PathBuf,
    /// Path relative to the recordings directory, used as its identifier.
    id: String,
    /// `<game>` directory name, shared by recordings and text output.
    game: String,
}

/// What happened to a video.
enum Outcome {
    Done,
    Interrupted,
}

struct Frame {
    index: u64,
    seconds: f64,
    image: RgbImage,
}

#[derive(Serialize)]
struct FrameText<'a> {
    video: &'a str,
    seconds: f64,
    words: &'a [Word],
}

impl Context {
    fn run(self) {
        if let Err(e) = self.config.ocr.check() {
            tracing::warn!("not reading text from recordings: {e}");
            return;
        }
        // Kept for the whole run: each reader loads the language model once.
        let mut readers: Vec<Reader> = (0..self.config.workers.max(1))
            .map(|_| self.config.ocr.reader())
            .collect();
        let mut games: HashMap<String, GameText> = HashMap::new();
        while !self.stop.load(Ordering::Relaxed) {
            if !self.allowed.load(Ordering::Relaxed) {
                self.set_status(None);
                thread::sleep(Duration::from_secs(1));
                continue;
            }
            let Some(video) = self.next_video() else {
                self.set_status(None);
                self.sleep(Duration::from_secs(30));
                continue;
            };
            let game = games
                .entry(video.game.clone())
                .or_insert_with(|| GameText::load(self.config.text_dir.join(&video.game)));
            tracing::info!("reading text from {}", video.id);
            match self.read_video(&video, game, &mut readers) {
                Ok(Outcome::Done) => {
                    tracing::info!("finished reading {}", video.id);
                    game.append("processed.txt", &format!("{}\n", video.id));
                }
                Ok(Outcome::Interrupted) => return,
                Err(e) => {
                    // Marked as processed anyway so a broken file is not
                    // retried forever.
                    tracing::error!("could not read {}: {e}", video.id);
                    game.append("processed.txt", &format!("{}\tfailed: {e}\n", video.id));
                }
            }
        }
    }

    fn set_status(&self, status: Option<String>) {
        *self.status.lock().unwrap() = status;
    }

    fn sleep(&self, duration: Duration) {
        let step = Duration::from_millis(500);
        let mut slept = Duration::ZERO;
        while slept < duration && !self.stop.load(Ordering::Relaxed) {
            thread::sleep(step);
            slept += step;
        }
    }

    /// The oldest recording not read yet. Names sort chronologically, and
    /// reading in order makes "first seen" in the vocabulary accurate.
    fn next_video(&self) -> Option<Video> {
        let root = &self.config.recordings_dir;
        for game_dir in read_dir_sorted(root).into_iter().filter(|p| p.is_dir()) {
            let game = game_dir.file_name()?.to_string_lossy().into_owned();
            let processed = GameText::processed(&self.config.text_dir.join(&game));
            for day in read_dir_sorted(&game_dir)
                .into_iter()
                .filter(|p| p.is_dir())
            {
                for path in read_dir_sorted(&day) {
                    if path.extension().is_none_or(|e| e != "mkv") {
                        continue;
                    }
                    let id = path.strip_prefix(root).ok()?.to_string_lossy().into_owned();
                    if !processed.contains(&id) {
                        return Some(Video { path, id, game });
                    }
                }
            }
        }
        None
    }

    fn read_video(
        &self,
        video: &Video,
        game: &mut GameText,
        readers: &mut [Reader],
    ) -> Result<Outcome, String> {
        let rate = self.config.samples_per_second.max(1);
        // decodebin picks the GPU decoder (nvh265dec) when there is one.
        let pipeline = gst::parse::launch(&format!(
            "filesrc name=src ! decodebin ! videoconvert ! videorate \
             ! video/x-raw,format=RGB,framerate={rate}/1 \
             ! appsink name=frames sync=false max-buffers=2"
        ))
        .map_err(|e| e.to_string())?
        .downcast::<gst::Pipeline>()
        .map_err(|_| "not a pipeline")?;
        pipeline
            .by_name("src")
            .ok_or("no filesrc")?
            .set_property("location", &video.path);
        let sink = pipeline
            .by_name("frames")
            .and_then(|e| e.downcast::<AppSink>().ok())
            .ok_or("no appsink")?;
        pipeline
            .set_state(gst::State::Playing)
            .map_err(|e| e.to_string())?;

        let result = self.read_frames(video, game, &pipeline, &sink, readers);
        let _ = pipeline.set_state(gst::State::Null);
        result
    }

    /// Decodes frames on one thread, reads them on `workers` threads, and
    /// handles the results in order on this one.
    fn read_frames(
        &self,
        video: &Video,
        game: &mut GameText,
        pipeline: &gst::Pipeline,
        sink: &AppSink,
        readers: &mut [Reader],
    ) -> Result<Outcome, String> {
        let workers = readers.len();
        let (frames_tx, frames_rx) = sync_channel::<Frame>(workers * 2);
        let frames_rx = Arc::new(Mutex::new(frames_rx));
        let (read_tx, read_rx) = sync_channel::<(Frame, Result<Vec<Word>, String>)>(workers * 2);
        let name = video.path.file_name().unwrap_or_default().to_string_lossy();

        thread::scope(|scope| {
            let decoder = scope.spawn(move || self.decode(pipeline, sink, frames_tx));
            for reader in readers.iter_mut() {
                let frames_rx = Arc::clone(&frames_rx);
                let read_tx = read_tx.clone();
                scope.spawn(move || {
                    loop {
                        let Ok(frame) = frames_rx.lock().unwrap().recv() else {
                            break;
                        };
                        let words = reader
                            .read(&imageops::grayscale(&frame.image))
                            .map_err(|e| format!("tesseract failed: {e}"));
                        if read_tx.send((frame, words)).is_err() {
                            break;
                        }
                    }
                });
            }
            drop(read_tx);
            drop(frames_rx);

            let rate = self.config.samples_per_second.max(1) as f64;
            let mut duration = None;
            let mut pending = BTreeMap::new();
            let mut next = 0;
            let mut tracker = Tracker::new(rate);
            for (frame, words) in read_rx {
                pending.insert(frame.index, (frame, words));
                while let Some((frame, words)) = pending.remove(&next) {
                    next += 1;
                    self.handle_frame(video, game, &mut tracker, frame, words?);
                }
                duration = duration.or_else(|| {
                    pipeline
                        .query_duration::<gst::ClockTime>()
                        .map(|d| d.seconds_f64())
                });
                if let Some(duration) = duration.filter(|d| *d > 0.0) {
                    let percent = (next as f64 / rate / duration * 100.0).min(100.0);
                    self.set_status(Some(format!("Reading text from {name} · {percent:.0}%")));
                }
            }
            decoder
                .join()
                .map_err(|_| "decoder thread panicked".to_owned())?
        })
    }

    /// Pulls frames from the pipeline, pausing it while reading is not allowed.
    fn decode(
        &self,
        pipeline: &gst::Pipeline,
        sink: &AppSink,
        frames: std::sync::mpsc::SyncSender<Frame>,
    ) -> Result<Outcome, String> {
        let mut index = 0;
        loop {
            if !self.allowed.load(Ordering::Relaxed) {
                let _ = pipeline.set_state(gst::State::Paused);
                self.set_status(Some("Reading text paused while playing".into()));
                while !self.allowed.load(Ordering::Relaxed) {
                    if self.stop.load(Ordering::Relaxed) {
                        return Ok(Outcome::Interrupted);
                    }
                    thread::sleep(Duration::from_millis(500));
                }
                let _ = pipeline.set_state(gst::State::Playing);
            }
            if self.stop.load(Ordering::Relaxed) {
                return Ok(Outcome::Interrupted);
            }
            let Ok(sample) = sink.pull_sample() else {
                // End of stream, or an error reported on the bus.
                return match pipeline
                    .bus()
                    .and_then(|b| b.pop_filtered(&[gst::MessageType::Error]))
                {
                    Some(msg) => match msg.view() {
                        gst::MessageView::Error(err) => Err(err.error().to_string()),
                        _ => Ok(Outcome::Done),
                    },
                    None => Ok(Outcome::Done),
                };
            };
            let frame = to_frame(&sample, index).ok_or("unexpected frame format")?;
            index += 1;
            if frames.send(frame).is_err() {
                return Err("frame readers stopped".into());
            }
        }
    }

    /// Saves the frame when confirmed words appeared, and records words
    /// never seen before in the vocabulary.
    fn handle_frame(
        &self,
        video: &Video,
        game: &mut GameText,
        tracker: &mut Tracker,
        frame: Frame,
        words: Vec<Word>,
    ) {
        let (mut words, appeared) = tracker.update(frame.index, words);
        if game.is_skipped_screen(&words) {
            return;
        }
        // One recurring word (often a misread logo) is not new text, unless
        // it was never seen at all.
        let worth_saving =
            appeared.len() >= 2 || appeared.iter().any(|w| !game.vocabulary.contains(w));
        if !worth_saving || !game.first_time_screen(&words) {
            return;
        }

        for word in &mut words {
            word.new = !game.vocabulary.contains(&word.key());
        }
        let stem = video.path.file_stem().unwrap_or_default().to_string_lossy();
        let day = video
            .path
            .parent()
            .and_then(Path::file_name)
            .unwrap_or_default();
        let secs = frame.seconds as u64;
        let name = format!("{stem}+{:02}m{:02}s", secs / 60, secs % 60);
        let png = game.dir.join(day).join(format!("{name}.png"));
        let json = png.with_extension("json");

        let saved = fs::create_dir_all(png.parent().unwrap())
            .map_err(|e| e.to_string())
            .and_then(|()| frame.image.save(&png).map_err(|e| e.to_string()))
            .and_then(|()| {
                let text = FrameText {
                    video: &video.id,
                    seconds: frame.seconds,
                    words: &words,
                };
                let body = serde_json::to_string_pretty(&text).map_err(|e| e.to_string())?;
                fs::write(&json, body).map_err(|e| e.to_string())
            });
        if let Err(e) = saved {
            tracing::error!("could not save {}: {e}", png.display());
            return;
        }

        let png_id = png
            .strip_prefix(&game.dir)
            .unwrap_or(&png)
            .to_string_lossy()
            .into_owned();
        for word in words.iter().filter(|w| w.new) {
            if game.vocabulary.insert(word.key()) {
                let line = format!(
                    "{}\t{png_id}\t{}\t{:.1}\t{}\n",
                    word.key(),
                    video.id,
                    frame.seconds,
                    word.sentence.replace('\t', " ")
                );
                game.append("vocabulary.tsv", &line);
            }
        }
    }
}

/// Frames, among the latest ones, in which a word must be read to count:
/// OCR noise on game scenery flickers for a single frame, real text stays.
const CONFIRM_FRAMES: usize = 3;
const CONFIRM_MIN_SIGHTINGS: usize = 2;
/// A confirmed word only counts as appearing again after being absent this
/// long, since OCR also misses real words for a frame or two.
const REAPPEAR_SECS: f64 = 10.0;
/// Two screens sharing this fraction of their words show the same text
/// (e.g. a menu whose news carousel came back to the same slide).
const SAME_SCREEN: f64 = 0.8;

/// Decides which words are real and which frames show new text.
struct Tracker {
    /// Words read in the latest `CONFIRM_FRAMES` frames.
    recent: VecDeque<HashSet<String>>,
    /// Confirmed word -> index of the last frame it was seen in.
    last_seen: HashMap<String, u64>,
    reappear_frames: u64,
}

impl Tracker {
    fn new(frames_per_second: f64) -> Self {
        Self {
            recent: VecDeque::with_capacity(CONFIRM_FRAMES),
            last_seen: HashMap::new(),
            reappear_frames: (REAPPEAR_SECS * frames_per_second).ceil() as u64,
        }
    }

    /// Returns the confirmed words of frame `index`, and the keys of those
    /// that appeared (were not seen recently).
    fn update(&mut self, index: u64, words: Vec<Word>) -> (Vec<Word>, Vec<String>) {
        if self.recent.len() == CONFIRM_FRAMES {
            self.recent.pop_front();
        }
        self.recent.push_back(words.iter().map(Word::key).collect());

        let confirmed: Vec<Word> = words
            .into_iter()
            .filter(|w| {
                let key = w.key();
                self.recent
                    .iter()
                    .filter(|frame| frame.contains(&key))
                    .count()
                    >= CONFIRM_MIN_SIGHTINGS
            })
            .collect();
        let mut appeared = Vec::new();
        for word in &confirmed {
            let last = self.last_seen.insert(word.key(), index);
            if last.is_none_or(|last| index - last > self.reappear_frames) {
                appeared.push(word.key());
            }
        }
        appeared.sort();
        appeared.dedup();
        (confirmed, appeared)
    }
}

fn to_frame(sample: &gst::Sample, index: u64) -> Option<Frame> {
    let caps = sample.caps()?;
    let s = caps.structure(0)?;
    let width = u32::try_from(s.get::<i32>("width").ok()?).ok()?;
    let height = u32::try_from(s.get::<i32>("height").ok()?).ok()?;
    let buffer = sample.buffer()?;
    let seconds = buffer.pts().map(|t| t.seconds_f64()).unwrap_or(0.0);
    let map = buffer.map_readable().ok()?;
    // Rows may be padded to a multiple of 4 bytes.
    let stride = map.len() / height as usize;
    let row = width as usize * 3;
    let mut pixels = Vec::with_capacity(row * height as usize);
    for y in 0..height as usize {
        pixels.extend_from_slice(map.get(y * stride..y * stride + row)?);
    }
    Some(Frame {
        index,
        seconds,
        image: RgbImage::from_raw(width, height, pixels)?,
    })
}

/// Reads `skip-screens.txt`, creating it with defaults the first time.
fn load_skip_screens(dir: &Path) -> Vec<HashSet<String>> {
    let path = dir.join("skip-screens.txt");
    let text = fs::read_to_string(&path).unwrap_or_else(|_| {
        let mut defaults = SKIP_SCREENS_HEADER.to_owned();
        let kcd2 = dir
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("1771300-"));
        if kcd2 {
            defaults = format!("{defaults}\n{KCD2_MENU}");
        }
        if let Err(e) = fs::create_dir_all(dir).and_then(|()| fs::write(&path, &defaults)) {
            tracing::warn!("could not write {}: {e}", path.display());
        }
        defaults
    });
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.split_whitespace().map(str::to_lowercase).collect())
        .collect()
}

fn read_dir_sorted(dir: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<_> = fs::read_dir(dir)
        .map(|entries| entries.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    paths.sort();
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(text: &str) -> Word {
        Word {
            text: text.into(),
            confidence: 95.0,
            bbox: [0; 4],
            new: false,
            sentence: String::new(),
        }
    }

    fn frame(tracker: &mut Tracker, index: u64, texts: &[&str]) -> (Vec<String>, bool) {
        let (words, appeared) = tracker.update(index, texts.iter().map(|t| word(t)).collect());
        (
            words.into_iter().map(|w| w.text).collect(),
            !appeared.is_empty(),
        )
    }

    #[test]
    fn skips_listed_screens() {
        let dir = std::env::temp_dir().join(format!(
            "champollion-test-{}/1771300-kcd2",
            std::process::id()
        ));
        let game = GameText::load(dir.clone());
        let screen = |texts: &[&str]| texts.iter().map(|t| word(t)).collect::<Vec<_>>();
        assert!(dir.join("skip-screens.txt").exists());
        assert!(game.is_skipped_screen(&screen(&["Pokračovat", "Nahrát", "hru", "Nastavení"])));
        assert!(game.is_skipped_screen(&screen(&["STORE", "LIBRARY", "COMMUNITY", "Home"])));
        // One menu word in a dialogue line is not the menu.
        assert!(!game.is_skipped_screen(&screen(&["Pokračovat", "do", "Skalice"])));
        let _ = fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn recognizes_repeated_screens() {
        let mut t = GameText::load(PathBuf::from("/nonexistent"));
        let screen = |texts: &[&str]| texts.iter().map(|t| word(t)).collect::<Vec<_>>();
        let menu = ["nová", "hra", "nahrát", "hru", "nastavení"];
        assert!(t.first_time_screen(&screen(&[&menu[..], &["šaška", "hospodou"]].concat())));
        assert!(t.first_time_screen(&screen(
            &[&menu[..], &["deskovou", "hru", "board", "game"]].concat()
        )));
        // The carousel is back on its first slide, with one word missed.
        assert!(!t.first_time_screen(&screen(&[&menu[..4], &["šaška", "hospodou"]].concat())));
    }

    #[test]
    fn ignores_flicker_and_short_dropouts() {
        let mut t = Tracker::new(1.0);
        // One-frame noise never counts; real words count from their 2nd frame.
        assert_eq!(frame(&mut t, 0, &["hra", "gome"]), (vec![], false));
        assert_eq!(frame(&mut t, 1, &["hra"]), (vec!["hra".into()], true));
        assert_eq!(
            frame(&mut t, 2, &["hra", "dsa"]),
            (vec!["hra".into()], false)
        );
        // Missed for one frame, then back: still the same text.
        assert_eq!(frame(&mut t, 3, &[]), (vec![], false));
        assert_eq!(frame(&mut t, 4, &["hra"]), (vec!["hra".into()], false));
        // Gone for longer than REAPPEAR_SECS: appears again.
        for index in 5..20 {
            frame(&mut t, index, &[]);
        }
        assert_eq!(frame(&mut t, 20, &["hra"]), (vec![], false));
        assert_eq!(frame(&mut t, 21, &["hra"]), (vec!["hra".into()], true));
    }
}
