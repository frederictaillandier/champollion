//! Turns the new words of each game's `vocabulary.tsv` into flashcards with
//! Claude (`cards.jsonl`), then sends them to the backend.

mod generate;
mod upload;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

pub use generate::Claude;

/// Time between two looks for new words.
const INTERVAL: Duration = Duration::from_secs(60);

pub struct Config {
    pub text_dir: PathBuf,
    /// Language of the words, as given to Tesseract.
    pub lang: String,
    /// Makes the cards; `None` to only send the cards already made.
    pub claude: Option<Claude>,
    /// e.g. `http://10.0.0.1:8090`
    pub backend_url: String,
}

/// Starts the thread making and sending cards every minute until `stop`.
pub fn spawn(config: Config, stop: Arc<AtomicBool>) -> thread::JoinHandle<()> {
    thread::Builder::new()
        .name("cards".into())
        .spawn(move || run(config, &stop))
        .expect("spawn cards thread")
}

fn run(config: Config, stop: &AtomicBool) {
    match &config.claude {
        Some(claude) => tracing::info!(
            "making cards with {} ({}) and sending them to {}",
            claude.bin.display(),
            claude.model,
            config.backend_url
        ),
        None => tracing::info!("sending cards to {}", config.backend_url),
    }
    let mut uploader = upload::Uploader::new(&config.backend_url);
    let mut generating = Failure::new("make cards");
    let mut uploading = Failure::new("send cards");
    while !stop.load(Ordering::Relaxed) {
        let games = fs::read_dir(&config.text_dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir());
        for dir in games {
            if let Some(claude) = &config.claude {
                generating.report(claude.make_cards(&dir, &config.lang, stop));
            }
            uploading.report(uploader.upload(&dir));
        }
        let mut slept = Duration::ZERO;
        while slept < INTERVAL && !stop.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(500));
            slept += Duration::from_millis(500);
        }
    }
}

/// Logs a step's failure once, not every minute while it keeps failing.
struct Failure {
    step: &'static str,
    failing: bool,
}

impl Failure {
    fn new(step: &'static str) -> Self {
        Self {
            step,
            failing: false,
        }
    }

    fn report(&mut self, result: Result<(), String>) {
        match result {
            Err(e) if !self.failing => {
                tracing::warn!("could not {}, will retry every minute: {e}", self.step);
                self.failing = true;
            }
            Err(_) => {}
            Ok(()) if self.failing => {
                tracing::info!("{} works again", self.step);
                self.failing = false;
            }
            Ok(()) => {}
        }
    }
}

/// The whole lines of `file` after the first `done` bytes, as recorded in
/// the `progress` file next to it. Lines are only appended to `file`; the
/// last one may still be being written, so it is left out until complete.
struct Pending {
    data: Vec<u8>,
    done: usize,
    end: usize,
    progress: PathBuf,
}

impl Pending {
    fn read(dir: &Path, file: &str, progress: &str) -> Option<Self> {
        let data = fs::read(dir.join(file)).ok()?;
        let progress = dir.join(progress);
        let mut done: usize = fs::read_to_string(&progress)
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0);
        if done > data.len() {
            // The file was deleted or rewritten: start over.
            done = 0;
        }
        let end = data.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
        (end > done).then_some(Self {
            data,
            done,
            end,
            progress,
        })
    }

    fn lines(&self) -> Vec<Vec<u8>> {
        self.data[self.done..self.end]
            .split_inclusive(|&b| b == b'\n')
            .map(<[u8]>::to_vec)
            .collect()
    }

    /// Records that `lines`, the next ones, are processed.
    fn advance(&mut self, lines: &[Vec<u8>]) -> Result<(), String> {
        self.done += lines.iter().map(|l| l.len()).sum::<usize>();
        fs::write(&self.progress, format!("{}\n", self.done)).map_err(|e| e.to_string())
    }
}

/// Appends lines to a file, creating it.
fn append(path: &Path, text: &str) -> Result<(), String> {
    use std::io::Write;
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut f| f.write_all(text.as_bytes()))
        .map_err(|e| format!("{}: {e}", path.display()))
}
