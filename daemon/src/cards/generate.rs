//! Makes flashcards from a game's `vocabulary.tsv` by asking Claude, through
//! the Claude Code CLI in headless mode, about batches of words.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use champollion_api::{NewCard, Sighting};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{Pending, append};

/// Words per question: each one costs a few seconds.
const BATCH: usize = 40;
const TIMEOUT: Duration = Duration::from_secs(300);
/// Next to `vocabulary.tsv`: how many of its bytes were made into cards.
const PROGRESS_FILE: &str = "generated.txt";
/// Words Claude found not worth learning, with why.
const DROPPED_FILE: &str = "dropped.tsv";

const PROMPT: &str = "\
You make Czech flashcards for an English speaker. The words were read by OCR \
from the Czech subtitles and menus of the video game {game}, whose characters \
may speak colloquial or archaic Czech (e.g. \"potřebujem\" for \"potřebujeme\", \
\"deš\" for \"dáš\").

For each word, read the whole sentence first, then give:
- lemma: its standard dictionary form (\"králem\" → \"král\", \"namažem\" → \
\"namazat se\", \"deš\" → \"dát\").
- pos, and gender for nouns (m, f or n; empty otherwise).
- translation: the English translation of the lemma IN THIS SENTENCE, a few words.
- definition: one short English sentence explaining what the lemma means as \
used in this sentence, including slang, puns or idioms (e.g. \"namazat se\" \
is slang for getting drunk). The card shows the lemma, so never describe the \
form read: no case, number, person, tense or mood (not \"vocative of otec\" \
nor \"imperative plural\", but \"one's father, or a priest; here someone \
calls out to him\").
- sentence_translation: a natural English translation of the sentence.
- keep: false only for words not worth learning as Czech vocabulary: other \
languages (English UI text), OCR garbage, names of people and places, \
numbers. Colloquial and archaic Czech forms are real words: keep them.

Answer for every id, without explanations.";

pub struct Claude {
    /// The `claude` executable.
    pub bin: PathBuf,
    /// e.g. `sonnet`, `haiku`
    pub model: String,
}

/// A `vocabulary.tsv` line: word, frame, video, seconds, sentence.
struct Word {
    form: String,
    frame: String,
    video: String,
    seconds: f64,
    sentence: String,
}

impl Word {
    fn parse(line: &[u8]) -> Option<Self> {
        let line = String::from_utf8_lossy(line);
        let mut fields = line.trim_end_matches(['\n', '\r']).split('\t');
        let form = fields.next().filter(|t| !t.is_empty())?.to_owned();
        Some(Self {
            form,
            frame: fields.next().unwrap_or_default().to_owned(),
            video: fields.next().unwrap_or_default().to_owned(),
            seconds: fields.next().and_then(|s| s.parse().ok()).unwrap_or(0.0),
            sentence: fields.next().unwrap_or_default().to_owned(),
        })
    }
}

#[derive(Serialize)]
struct Question<'a> {
    id: usize,
    word: &'a str,
    sentence: &'a str,
}

#[derive(Deserialize)]
struct Answer {
    id: usize,
    keep: bool,
    lemma: String,
    pos: String,
    gender: String,
    translation: String,
    definition: String,
    sentence_translation: String,
}

#[derive(Deserialize)]
struct Answers {
    cards: Vec<Answer>,
}

/// What `claude -p --output-format json` prints.
#[derive(Deserialize)]
struct Output {
    #[serde(default)]
    is_error: bool,
    #[serde(default)]
    result: String,
    structured_output: Option<Answers>,
}

impl Claude {
    /// Makes cards from the words added to the game's vocabulary since the
    /// last time, appending them to `cards.jsonl`.
    pub fn make_cards(&self, dir: &Path, lang: &str, stop: &AtomicBool) -> Result<(), String> {
        let Some(mut pending) = Pending::read(dir, "vocabulary.tsv", PROGRESS_FILE) else {
            return Ok(());
        };
        let game = dir.file_name().unwrap_or_default().to_string_lossy();
        for chunk in pending.lines().chunks(BATCH) {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let words: Vec<Word> = chunk.iter().filter_map(|l| Word::parse(l)).collect();
            let started = Instant::now();
            let answers = self.ask(dir, &game_name(&game), &words)?;

            let (mut cards, mut dropped) = (String::new(), String::new());
            for answer in answers {
                let Some(word) = words.get(answer.id) else {
                    continue;
                };
                if !answer.keep {
                    dropped.push_str(&format!(
                        "{}\t{}\t{}\n",
                        word.form,
                        answer.lemma,
                        answer.definition.replace(['\t', '\n'], " ")
                    ));
                    continue;
                }
                let card = NewCard {
                    lang: lang.to_owned(),
                    lemma: answer.lemma,
                    pos: answer.pos,
                    gender: answer.gender,
                    translation: answer.translation,
                    sighting: Sighting {
                        form: word.form.clone(),
                        sentence: word.sentence.clone(),
                        sentence_translation: answer.sentence_translation,
                        definition: answer.definition,
                        game: game.to_string(),
                        video: word.video.clone(),
                        seconds: word.seconds,
                        frame: word.frame.clone(),
                    },
                };
                cards.push_str(&serde_json::to_string(&card).map_err(|e| e.to_string())?);
                cards.push('\n');
            }
            append(&dir.join("cards.jsonl"), &cards)?;
            append(&dir.join(DROPPED_FILE), &dropped)?;
            pending.advance(chunk)?;
            tracing::info!(
                "made {} cards from {} words of {game} in {:.0?}",
                cards.lines().count(),
                words.len(),
                started.elapsed()
            );
        }
        Ok(())
    }

    /// Asks Claude about the words, without tools, settings or memory.
    fn ask(&self, dir: &Path, game: &str, words: &[Word]) -> Result<Vec<Answer>, String> {
        let questions: Vec<Question> = words
            .iter()
            .enumerate()
            .map(|(id, w)| Question {
                id,
                word: &w.form,
                sentence: &w.sentence,
            })
            .collect();
        let input = serde_json::to_string(&questions).map_err(|e| e.to_string())?;
        let mut child = Command::new(&self.bin)
            .args(["-p", "--model", &self.model, "--output-format", "json"])
            .args(["--json-schema", &schema().to_string()])
            .args(["--system-prompt", &PROMPT.replace("{game}", game)])
            .args(["--tools", "", "--setting-sources", ""])
            .args(["--settings", r#"{"alwaysThinkingEnabled":false}"#])
            .args(["--strict-mcp-config", "--no-session-persistence"])
            // Thinking makes each batch ten times slower, for no better cards.
            .env("MAX_THINKING_TOKENS", "0")
            .current_dir(dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("{}: {e}", self.bin.display()))?;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .map_err(|e| e.to_string())?;

        let mut stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        let out_reader = thread::spawn(move || {
            let mut out = Vec::new();
            let _ = stdout.read_to_end(&mut out);
            out
        });
        let err_reader = thread::spawn(move || {
            let mut err = String::new();
            let _ = stderr.read_to_string(&mut err);
            err
        });
        let started = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                break status;
            }
            if started.elapsed() > TIMEOUT {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("claude did not answer in {TIMEOUT:?}"));
            }
            thread::sleep(Duration::from_millis(200));
        };
        let (out, err) = (out_reader.join().unwrap(), err_reader.join().unwrap());

        let output: Output = serde_json::from_slice(&out).map_err(|e| {
            format!(
                "unexpected answer from claude ({status}, {e}): {}{}",
                err.trim(),
                String::from_utf8_lossy(&out).trim()
            )
        })?;
        if output.is_error {
            return Err(format!("claude: {}", output.result));
        }
        let answers = output
            .structured_output
            .ok_or("claude gave no structured output")?
            .cards;
        if answers.len() != words.len() {
            tracing::warn!(
                "claude answered for {} of {} words",
                answers.len(),
                words.len()
            );
        }
        Ok(answers)
    }
}

/// `1771300-kingdom-come-deliverance-ii` → `kingdom come deliverance ii`
fn game_name(slug: &str) -> String {
    slug.trim_start_matches(|c: char| c.is_ascii_digit() || c == '-')
        .replace('-', " ")
}

fn schema() -> serde_json::Value {
    let string = json!({"type": "string"});
    json!({
        "type": "object",
        "properties": {"cards": {"type": "array", "items": {
            "type": "object",
            "properties": {
                "id": {"type": "integer"},
                "keep": {"type": "boolean"},
                "lemma": string,
                "pos": {"type": "string", "enum": [
                    "noun", "verb", "adjective", "adverb", "pronoun", "preposition",
                    "conjunction", "particle", "interjection", "numeral", "other"
                ]},
                "gender": {"type": "string", "enum": ["m", "f", "n", ""]},
                "translation": string,
                "definition": string,
                "sentence_translation": string,
            },
            "required": [
                "id", "keep", "lemma", "pos", "gender", "translation", "definition",
                "sentence_translation"
            ],
        }}},
        "required": ["cards"],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_vocabulary_line() {
        let line = b"hra\t2026-10-03/22-49-34_000+00m01s.png\tkcd2/2026-10-03/22-49-34_000.mkv\t1.0\tNov\xc3\xa1 hra\n";
        let word = Word::parse(line).unwrap();
        assert_eq!(word.form, "hra");
        assert_eq!(word.frame, "2026-10-03/22-49-34_000+00m01s.png");
        assert_eq!(word.video, "kcd2/2026-10-03/22-49-34_000.mkv");
        assert_eq!(word.seconds, 1.0);
        assert_eq!(word.sentence, "Nová hra");
    }

    #[test]
    fn skips_empty_lines() {
        assert!(Word::parse(b"\n").is_none());
    }

    #[test]
    fn names_the_game_from_its_slug() {
        assert_eq!(
            game_name("1771300-kingdom-come-deliverance-ii"),
            "kingdom come deliverance ii"
        );
    }
}
