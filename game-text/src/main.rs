//! The game's own text, to correct what OCR reads.
//!
//!   champollion-game-text import LOCALIZATION_DIR DB
//!   champollion-game-text match [--min-similarity 0.8] DB OCR.tsv...
//!
//! `import` copies the Czech and English text of Kingdom Come:
//! Deliverance II (its `Localization` folder) into a SQLite database.
//! `match` reads the words printed by champollion-gpu-ocr (source, x, y,
//! confidence, word), puts each screen line back together, and prints the
//! game line it comes from: how often it was read, the similarity, the OCR
//! text, the game's exact text for it, where it is (file:id), and the whole
//! game line in Czech and English.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use game_text::{import, matcher};

const USAGE: &str = "usage: champollion-game-text import LOCALIZATION_DIR DB\n       \
                     champollion-game-text match [--min-similarity 0.8] DB OCR.tsv...";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("champollion-game-text: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("import") => {
            let (Some(localization), Some(db), None) = (args.next(), args.next(), args.next())
            else {
                return Err(USAGE.into());
            };
            let count = import::import(&PathBuf::from(localization), &PathBuf::from(db))?;
            eprintln!("{count} lines imported");
            Ok(())
        }
        Some("match") => {
            let mut min_similarity = 0.8;
            let mut paths = Vec::new();
            while let Some(arg) = args.next() {
                if arg == "--min-similarity" {
                    min_similarity = args
                        .next()
                        .and_then(|v| v.parse().ok())
                        .ok_or("--min-similarity takes a number from 0 to 1")?;
                } else {
                    paths.push(PathBuf::from(arg));
                }
            }
            if paths.len() < 2 {
                return Err(USAGE.into());
            }
            let db = paths.remove(0);
            match_ocr(&db, &paths, min_similarity)
        }
        _ => Err(USAGE.into()),
    }
}

fn match_ocr(db: &PathBuf, ocr: &[PathBuf], min_similarity: f32) -> Result<(), String> {
    let start = Instant::now();
    let index = matcher::Index::load(db)?;
    eprintln!(
        "{} game lines loaded in {:.1} s",
        index.len(),
        start.elapsed().as_secs_f64()
    );

    // Screen lines in the order first read, with how many frames read them.
    let mut lines: Vec<(String, u32)> = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for path in ocr {
        let tsv = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        for line in screen_lines(&tsv) {
            match seen.get(&line) {
                Some(&i) => lines[i].1 += 1,
                None => {
                    seen.insert(line.clone(), lines.len());
                    lines.push((line, 1));
                }
            }
        }
    }

    let start = Instant::now();
    let mut matched = 0;
    for (line, count) in &lines {
        let Some(found) = index.find(line, min_similarity) else {
            continue;
        };
        matched += 1;
        println!(
            "{count}\t{:.2}\t{line}\t{}\t{}:{}\t{}\t{}",
            found.similarity,
            found.text,
            found.file,
            found.id,
            found.line,
            found.english.unwrap_or_default()
        );
    }
    eprintln!(
        "{matched} of {} different screen lines matched, in {:.1} s",
        lines.len(),
        start.elapsed().as_secs_f64()
    );
    Ok(())
}

/// The OCR words put back into screen lines: words of the same source and
/// position, in order.
fn screen_lines(tsv: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut current: Option<(&str, &str, &str)> = None;
    for row in tsv.lines() {
        let cols: Vec<&str> = row.split('\t').collect();
        let [source, x, y, _confidence, word] = cols[..] else {
            continue;
        };
        let key = (source, x, y);
        match lines.last_mut() {
            Some(line) if current == Some(key) => {
                line.push(' ');
                line.push_str(word);
            }
            _ => {
                lines.push(word.to_owned());
                current = Some(key);
            }
        }
    }
    lines
}
