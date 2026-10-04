//! Sends the words of each game's `vocabulary.tsv` to the backend.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use champollion_api::{NewWord, WordBatch, WordBatchResult};

/// Time between two looks for new words.
const INTERVAL: Duration = Duration::from_secs(60);
/// Words per request.
const BATCH: usize = 500;
/// Next to `vocabulary.tsv`: how many of its bytes the backend has.
const UPLOADED_FILE: &str = "uploaded.txt";

pub struct Config {
    pub text_dir: PathBuf,
    /// e.g. `http://10.0.0.1:8090`
    pub backend_url: String,
    /// Language of the words, as given to Tesseract.
    pub lang: String,
}

/// Starts the thread uploading new words every minute until `stop`.
pub fn spawn(config: Config, stop: Arc<AtomicBool>) -> thread::JoinHandle<()> {
    thread::Builder::new()
        .name("word-uploader".into())
        .spawn(move || Uploader::new(config).run(&stop))
        .expect("spawn word uploader thread")
}

struct Uploader {
    config: Config,
    agent: ureq::Agent,
    /// Whether the last upload failed, to log a failure once and not every minute.
    failing: bool,
}

impl Uploader {
    fn new(config: Config) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .build()
            .into();
        Self {
            config,
            agent,
            failing: false,
        }
    }

    fn run(mut self, stop: &AtomicBool) {
        tracing::info!("sending new words to {}", self.config.backend_url);
        while !stop.load(Ordering::Relaxed) {
            let games = fs::read_dir(&self.config.text_dir)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir());
            for dir in games {
                if let Err(e) = self.upload_game(&dir) {
                    if !self.failing {
                        tracing::warn!("could not send words, will retry every minute: {e}");
                    }
                    self.failing = true;
                    break;
                }
            }
            let mut slept = Duration::ZERO;
            while slept < INTERVAL && !stop.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(500));
                slept += Duration::from_millis(500);
            }
        }
    }

    /// Sends the lines of the game's vocabulary added since the last upload.
    fn upload_game(&mut self, dir: &Path) -> Result<(), String> {
        let Ok(vocabulary) = fs::read(dir.join("vocabulary.tsv")) else {
            return Ok(());
        };
        let uploaded_path = dir.join(UPLOADED_FILE);
        let mut uploaded: usize = fs::read_to_string(&uploaded_path)
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0);
        if uploaded > vocabulary.len() {
            // The vocabulary was deleted or rewritten: send it all again.
            uploaded = 0;
        }
        // Only whole lines: the daemon may be writing the last one.
        let end = vocabulary
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(0, |i| i + 1);
        if end <= uploaded {
            return Ok(());
        }
        let game = dir.file_name().unwrap_or_default().to_string_lossy();

        let lines: Vec<&[u8]> = vocabulary[uploaded..end]
            .split_inclusive(|&b| b == b'\n')
            .collect();
        for chunk in lines.chunks(BATCH) {
            let words = chunk
                .iter()
                .filter_map(|line| {
                    parse_line(&String::from_utf8_lossy(line), &game, &self.config.lang)
                })
                .collect();
            let result: WordBatchResult = self
                .agent
                .post(format!("{}/words", self.config.backend_url))
                .send_json(WordBatch { words })
                .and_then(|mut r| r.body_mut().read_json())
                .map_err(|e| e.to_string())?;
            uploaded += chunk.iter().map(|l| l.len()).sum::<usize>();
            fs::write(&uploaded_path, format!("{uploaded}\n")).map_err(|e| e.to_string())?;
            if self.failing {
                tracing::info!("sending words works again");
                self.failing = false;
            }
            if result.added > 0 {
                tracing::info!("backend learned {} new words of {game}", result.added);
            }
        }
        Ok(())
    }
}

/// A `vocabulary.tsv` line: word, frame, video, seconds, sentence.
fn parse_line(line: &str, game: &str, lang: &str) -> Option<NewWord> {
    let mut fields = line.trim_end_matches(['\n', '\r']).split('\t');
    let text = fields.next().filter(|t| !t.is_empty())?;
    let frame = fields.next().unwrap_or_default();
    let video = fields.next().unwrap_or_default();
    let seconds = fields.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let sentence = fields.next().unwrap_or_default();
    Some(NewWord {
        lang: lang.to_owned(),
        text: text.to_owned(),
        sentence: sentence.to_owned(),
        game: game.to_owned(),
        video: video.to_owned(),
        seconds,
        frame: frame.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_vocabulary_line() {
        let line = "hra\t2026-10-03/22-49-34_000+00m01s.png\tkcd2/2026-10-03/22-49-34_000.mkv\t1.0\tNová hra\n";
        let word = parse_line(line, "kcd2", "ces").unwrap();
        assert_eq!(word.text, "hra");
        assert_eq!(word.frame, "2026-10-03/22-49-34_000+00m01s.png");
        assert_eq!(word.video, "kcd2/2026-10-03/22-49-34_000.mkv");
        assert_eq!(word.seconds, 1.0);
        assert_eq!(word.sentence, "Nová hra");
    }

    #[test]
    fn skips_empty_lines() {
        assert!(parse_line("\n", "kcd2", "ces").is_none());
    }
}
