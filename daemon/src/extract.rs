//! Reads the text in finished recordings, while the game runs too: a Jetson
//! reads them on its GPU (`remote`), and this thread keeps track of the words
//! and screens seen, saving those new to the vocabulary.

mod ocr;
mod remote;

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app::AppSink;
use image::RgbImage;
use serde::Serialize;

use ocr::{Screen, Word};

pub struct Config {
    pub recordings_dir: PathBuf,
    pub text_dir: PathBuf,
    /// Frames read per second of video.
    pub samples_per_second: u32,
    /// OCR confidence (0-100) under which a word is not kept.
    pub min_confidence: f32,
    /// SSH destination of the machine reading the recordings, with
    /// `~/ocr/read-recording` (a Jetson).
    pub remote: String,
}

/// Background thread reading recordings.
pub struct Indexer {
    /// File prefix of the recording being made (see `record::file_prefix`),
    /// whose last file is not finished.
    pub recording: Arc<Mutex<Option<PathBuf>>>,
    /// What the indexer is doing, for the tray; `None` when idle.
    pub status: Arc<Mutex<Option<String>>>,
    handle: thread::JoinHandle<()>,
}

impl Indexer {
    pub fn spawn(config: Config, stop: Arc<AtomicBool>) -> Self {
        let recording = Arc::new(Mutex::new(None));
        let status = Arc::new(Mutex::new(None));
        let ctx = Context {
            config,
            stop,
            recording: recording.clone(),
            status: status.clone(),
        };
        let handle = thread::Builder::new()
            .name("text-indexer".into())
            .spawn(move || ctx.run())
            .expect("spawn text indexer thread");
        Self {
            recording,
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
    recording: Arc<Mutex<Option<PathBuf>>>,
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
    /// The remote reader could not be reached; read it later.
    Retry(String),
}

struct Frame {
    index: u64,
    seconds: f64,
}

/// Time to wait before trying an unreachable remote reader again.
const REMOTE_RETRY: Duration = Duration::from_secs(60);

#[derive(Serialize)]
struct FrameText<'a> {
    video: &'a str,
    seconds: f64,
    words: &'a [Word],
    lines: &'a [String],
}

impl Context {
    fn run(self) {
        run_when_idle();
        tracing::info!("reading text from recordings on {}", self.config.remote);
        let mut games: HashMap<String, GameText> = HashMap::new();
        let mut unreachable = false;
        while !self.stop.load(Ordering::Relaxed) {
            let Some(video) = self.next_video() else {
                self.set_status(None);
                self.sleep(Duration::from_secs(30));
                continue;
            };
            let game = games
                .entry(video.game.clone())
                .or_insert_with(|| GameText::load(self.config.text_dir.join(&video.game)));
            if !unreachable {
                tracing::info!("reading text from {}", video.id);
            }
            match self.read_remote(&video, game) {
                Ok(Outcome::Done) => {
                    tracing::info!("finished reading {}", video.id);
                    game.append("processed.txt", &format!("{}\n", video.id));
                    unreachable = false;
                }
                Ok(Outcome::Interrupted) => return,
                Ok(Outcome::Retry(e)) => {
                    // Logged once, not every minute while it is off.
                    if !unreachable {
                        tracing::warn!("cannot reach the reader, trying again every minute: {e}");
                        unreachable = true;
                    }
                    self.set_status(Some("Waiting for the Jetson".into()));
                    self.sleep(REMOTE_RETRY);
                }
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

    /// The oldest recording not read yet, leaving out the file being
    /// recorded. Names sort chronologically, and reading in order makes
    /// "first seen" in the vocabulary accurate.
    fn next_video(&self) -> Option<Video> {
        let root = &self.config.recordings_dir;
        let recording = self.recording.lock().unwrap().clone();
        for game_dir in read_dir_sorted(root).into_iter().filter(|p| p.is_dir()) {
            let game = game_dir.file_name()?.to_string_lossy().into_owned();
            let processed = GameText::processed(&self.config.text_dir.join(&game));
            for day in read_dir_sorted(&game_dir)
                .into_iter()
                .filter(|p| p.is_dir())
            {
                let files: Vec<PathBuf> = read_dir_sorted(&day)
                    .into_iter()
                    .filter(|p| p.extension().is_some_and(|e| e == "mkv"))
                    .collect();
                let unfinished = recording
                    .as_deref()
                    .and_then(|prefix| being_recorded(&files, prefix));
                for path in files.iter().filter(|p| Some(*p) != unfinished) {
                    let path = path.clone();
                    let id = path.strip_prefix(root).ok()?.to_string_lossy().into_owned();
                    if !processed.contains(&id) {
                        return Some(Video { path, id, game });
                    }
                }
            }
        }
        None
    }

    /// Reads the recording on the remote machine, handling its frames here
    /// in order.
    fn read_remote(&self, video: &Video, game: &mut GameText) -> Result<Outcome, String> {
        let rate = self.config.samples_per_second.max(1);
        let name = video.path.file_name().unwrap_or_default().to_string_lossy();
        let mut tracker = Tracker::new(rate as f64);
        let mut index = 0;
        let host = &self.config.remote;
        let result = remote::read(host, &video.path, rate, &self.stop, &mut |frame| {
            let screen = ocr::parse_tsv(&frame.tsv, self.config.min_confidence);
            let seconds = frame.seconds;
            let frame = Frame { index, seconds };
            self.handle_frame(video, game, &mut tracker, frame, screen);
            index += 1;
            let secs = seconds as u64;
            self.set_status(Some(format!(
                "Reading text on the Jetson from {name} · {}:{:02}",
                secs / 60,
                secs % 60
            )));
        });
        match result {
            Ok(true) => Ok(Outcome::Done),
            Ok(false) => Ok(Outcome::Interrupted),
            Err(remote::Failure::Unreachable(e)) => Ok(Outcome::Retry(e)),
            Err(remote::Failure::Failed(e)) => Err(e),
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
        screen: Screen,
    ) {
        let (mut words, appeared) = tracker.update(frame.index, screen.words);
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

        // The frame was read elsewhere: it is decoded here, for reference;
        // the cards only need its text.
        let image = grab_frame(&video.path, frame.seconds)
            .inspect_err(|e| tracing::warn!("could not save the image of {name}: {e}"))
            .ok();
        let saved = fs::create_dir_all(png.parent().unwrap())
            .map_err(|e| e.to_string())
            .and_then(|()| match image {
                Some(image) => image.save(&png).map_err(|e| e.to_string()),
                None => Ok(()),
            })
            .and_then(|()| {
                let text = FrameText {
                    video: &video.id,
                    seconds: frame.seconds,
                    words: &words,
                    lines: &screen.lines,
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

/// The pixels of a decoded RGB frame.
fn to_image(sample: &gst::Sample) -> Option<RgbImage> {
    let caps = sample.caps()?;
    let s = caps.structure(0)?;
    let width = u32::try_from(s.get::<i32>("width").ok()?).ok()?;
    let height = u32::try_from(s.get::<i32>("height").ok()?).ok()?;
    let buffer = sample.buffer()?;
    let map = buffer.map_readable().ok()?;
    // Rows may be padded to a multiple of 4 bytes.
    let stride = map.len() / height as usize;
    let row = width as usize * 3;
    let mut pixels = Vec::with_capacity(row * height as usize);
    for y in 0..height as usize {
        pixels.extend_from_slice(map.get(y * stride..y * stride + row)?);
    }
    RgbImage::from_raw(width, height, pixels)
}

/// The frame at `seconds` in the recording at `path`, decoded on its own.
fn grab_frame(path: &Path, seconds: f64) -> Result<RgbImage, String> {
    let pipeline = gst::parse::launch(
        "filesrc name=src ! decodebin ! videoconvert ! video/x-raw,format=RGB \
         ! appsink name=frame sync=false",
    )
    .map_err(|e| e.to_string())?
    .downcast::<gst::Pipeline>()
    .map_err(|_| "not a pipeline")?;
    pipeline
        .by_name("src")
        .ok_or("no filesrc")?
        .set_property("location", path);
    let sink = pipeline
        .by_name("frame")
        .and_then(|e| e.downcast::<AppSink>().ok())
        .ok_or("no appsink")?;
    let timeout = gst::ClockTime::from_seconds(10);
    let result = (|| {
        pipeline
            .set_state(gst::State::Paused)
            .map_err(|e| e.to_string())?;
        pipeline.state(timeout).0.map_err(|e| e.to_string())?;
        pipeline
            .seek_simple(
                gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                gst::ClockTime::from_nseconds((seconds * 1e9) as u64),
            )
            .map_err(|e| e.to_string())?;
        pipeline.state(timeout).0.map_err(|e| e.to_string())?;
        let sample = sink.pull_preroll().map_err(|e| e.to_string())?;
        to_image(&sample).ok_or_else(|| "unexpected frame format".to_owned())
    })();
    let _ = pipeline.set_state(gst::State::Null);
    result
}

/// The file of `files` still being written by the recording whose files are
/// `<prefix>_NNN.mkv`: its last one.
fn being_recorded<'a>(files: &'a [PathBuf], prefix: &Path) -> Option<&'a PathBuf> {
    let name = format!("{}_", prefix.file_name()?.to_string_lossy());
    files
        .iter()
        .filter(|f| {
            f.parent() == prefix.parent()
                && f.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with(&name))
        })
        .max()
}

/// Lets the game have the CPU first: this thread, and the threads it starts,
/// only run on cores nothing else wants.
fn run_when_idle() {
    let param = libc::sched_param { sched_priority: 0 };
    // SAFETY: sched_setscheduler(2) on the calling thread, with a valid param.
    if unsafe { libc::sched_setscheduler(0, libc::SCHED_IDLE, &param) } != 0 {
        tracing::warn!(
            "could not lower the priority of reading text: {}",
            std::io::Error::last_os_error()
        );
    }
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
    fn leaves_out_the_file_being_recorded() {
        let day = PathBuf::from("/r/game/2026-10-05");
        let files = [
            "10-00-00_000.mkv",
            "10-00-00_001.mkv",
            "11-30-00_000.mkv",
            "11-30-00_001.mkv",
        ]
        .map(|f| day.join(f));
        assert_eq!(
            being_recorded(&files, &day.join("11-30-00")),
            Some(&day.join("11-30-00_001.mkv"))
        );
        assert_eq!(
            being_recorded(&files[..2], &day.join("10-00-00")),
            Some(&day.join("10-00-00_001.mkv"))
        );
        // Not written yet, or in another day's directory.
        assert_eq!(being_recorded(&files, &day.join("12-00-00")), None);
        assert_eq!(
            being_recorded(&files, Path::new("/r/game/2026-10-06/11-30-00")),
            None
        );
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
