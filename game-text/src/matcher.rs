//! Finds which line of the game a line read by OCR comes from, despite
//! OCR errors, and returns the game's exact text for it.
//!
//! Texts are compared without accents, case or punctuation, since OCR
//! loses those most. Candidates share many three-letter sequences
//! (trigrams) with the OCR line; the best is the one containing a stretch
//! closest to it in edit distance. A screen line is often only part of a
//! game line (long subtitles wrap), so that stretch is what is returned.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use rusqlite::Connection;
use unicode_normalization::UnicodeNormalization;

/// Trigrams in more texts than this say little and cost a lot.
const COMMON_TRIGRAM: usize = 20_000;
/// Candidates measured in full, after counting shared trigrams.
const CANDIDATES: usize = 40;
/// OCR lines shorter than this (letters and digits, without spaces) only
/// match a game text equal to them as a whole: short ones match anywhere.
const MIN_PARTIAL: usize = 8;

/// A game line found for an OCR line.
#[derive(Debug, Clone)]
pub struct Found {
    /// The game's text for the stretch the OCR line read, accents and
    /// punctuation included.
    pub text: String,
    /// The whole game line.
    pub line: String,
    pub file: String,
    pub id: String,
    pub english: Option<String>,
    /// 1 when the OCR line matches exactly (accents aside), less with each
    /// edit it needs.
    pub similarity: f32,
}

/// A game text as kept in the index: only what finding candidates needs,
/// as the daemon keeps the index loaded while the game runs.
struct Text {
    /// Index in `Index::rows`.
    row: u32,
    original: Box<str>,
    /// Characters of its folded form.
    folded_len: u32,
}

/// A game text ready to be compared, made again for each candidate.
struct Prepared {
    original: Vec<char>,
    /// Without accents, lowercase, words separated by one space.
    folded: Vec<char>,
    /// Index in `original` of each char of `folded`.
    positions: Vec<usize>,
}

impl Prepared {
    fn new(text: &str) -> Self {
        let original: Vec<char> = text.chars().collect();
        let (folded, positions) = fold(&original);
        Self {
            original,
            folded,
            positions,
        }
    }
}

struct Row {
    file: String,
    id: String,
    english: Option<String>,
}

#[derive(Default)]
pub struct Index {
    rows: Vec<Row>,
    texts: Vec<Text>,
    trigrams: HashMap<u64, Vec<u32>>,
}

impl Index {
    /// Loads every line of the database made by `import`.
    pub fn load(db: &Path) -> Result<Self, String> {
        let conn = Connection::open(db).map_err(|e| e.to_string())?;
        let mut query = conn
            .prepare("SELECT file, id, czech, czech_alt, english FROM lines")
            .map_err(|e| e.to_string())?;
        let mut index = Index::default();
        let mut rows = query.query(()).map_err(|e| e.to_string())?;
        while let Some(r) = rows.next().map_err(|e| e.to_string())? {
            let get = |i| r.get::<_, String>(i).map_err(|e| e.to_string());
            let english = r.get(4).map_err(|e| e.to_string())?;
            index.push(get(0)?, get(1)?, &get(2)?, &get(3)?, english);
        }
        Ok(index)
    }

    /// Adds a game line: where it is, its two Czech texts, its English.
    pub fn push(
        &mut self,
        file: String,
        id: String,
        czech: &str,
        alt: &str,
        english: Option<String>,
    ) {
        let row = self.rows.len();
        self.rows.push(Row { file, id, english });
        self.add(row, czech);
        if alt != czech {
            self.add(row, alt);
        }
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    fn add(&mut self, row: usize, text: &str) {
        let original: Vec<char> = text.chars().collect();
        let (folded, _) = fold(&original);
        if folded.is_empty() {
            return;
        }
        let n = self.texts.len() as u32;
        for t in trigrams(&folded).into_iter().collect::<HashSet<_>>() {
            self.trigrams.entry(t).or_default().push(n);
        }
        self.texts.push(Text {
            row: row as u32,
            original: text.into(),
            folded_len: folded.len() as u32,
        });
    }

    /// The game line `ocr` most likely is, if one is close enough:
    /// `min_similarity` is the share of characters that must be right.
    pub fn find(&self, ocr: &str, min_similarity: f32) -> Option<Found> {
        let ocr_chars: Vec<char> = ocr.chars().collect();
        let (query, _) = fold(&ocr_chars);
        let letters = query.iter().filter(|c| **c != ' ').count();
        if letters < 3 {
            return None;
        }
        let mut shared: HashMap<u32, u32> = HashMap::new();
        for t in trigrams(&query).into_iter().collect::<HashSet<_>>() {
            if let Some(texts) = self.trigrams.get(&t).filter(|t| t.len() <= COMMON_TRIGRAM) {
                for &n in texts {
                    *shared.entry(n).or_default() += 1;
                }
            }
        }
        let mut candidates: Vec<(u32, u32)> = shared.into_iter().collect();
        // Most shared trigrams first; then shorter texts, closer to a
        // whole match; then a fixed order, so results are repeatable.
        candidates.sort_by_key(|&(n, count)| {
            (
                std::cmp::Reverse(count),
                self.texts[n as usize].folded_len,
                n,
            )
        });
        let mut best: Option<(Match, Prepared)> = None;
        for &(n, _) in candidates.iter().take(CANDIDATES) {
            let prepared = Prepared::new(&self.texts[n as usize].original);
            let Some(found) = compare(&query, letters, &prepared) else {
                continue;
            };
            let better = best.as_ref().is_none_or(|(b, p)| {
                (found.similarity, found.coverage) > (b.similarity, b.coverage)
                    || ((found.similarity, found.coverage) == (b.similarity, b.coverage)
                        && prepared.folded.len() < p.folded.len())
            });
            if better {
                best = Some((
                    Match {
                        text: n as usize,
                        ..found
                    },
                    prepared,
                ));
            }
        }
        let (best, prepared) = best?;
        if best.similarity < min_similarity {
            return None;
        }
        let text = &self.texts[best.text];
        let row = &self.rows[text.row as usize];
        Some(Found {
            text: stretch(
                &prepared,
                best.start,
                best.end,
                ends_with_punctuation(&ocr_chars),
            ),
            line: text.original.to_string(),
            file: row.file.clone(),
            id: row.id.clone(),
            english: row.english.clone(),
            similarity: best.similarity,
        })
    }
}

/// How an OCR line compares with a game text.
struct Match {
    /// Index in `Index::texts`.
    text: usize,
    similarity: f32,
    /// Share of the OCR line the game text accounts for: less than 1 when
    /// the screen showed more, like a save's name followed by its date.
    coverage: f32,
    /// Stretch of the game text that was read, in folded characters.
    start: usize,
    end: usize,
}

fn compare(query: &[char], letters: usize, text: &Prepared) -> Option<Match> {
    if letters < MIN_PARTIAL {
        // Short lines match anywhere: only a whole, equal text counts.
        return (text.folded == query).then(|| Match {
            text: 0,
            similarity: 1.0,
            coverage: 1.0,
            start: 0,
            end: text.folded.len(),
        });
    }
    // The OCR line as part of the game text, as when a subtitle wraps.
    let (distance, start, end) = substring_distance(query, &text.folded);
    let inside = Match {
        text: 0,
        similarity: 1.0 - distance as f32 / query.len() as f32,
        coverage: 1.0,
        start,
        end,
    };
    // The whole game text within a longer OCR line.
    let text_letters = text.folded.iter().filter(|c| **c != ' ').count();
    if text.folded.len() >= query.len() || text_letters < MIN_PARTIAL {
        return Some(inside);
    }
    let (distance, _, _) = substring_distance(&text.folded, query);
    let around = Match {
        text: 0,
        similarity: 1.0 - distance as f32 / text.folded.len() as f32,
        coverage: text.folded.len() as f32 / query.len() as f32,
        start: 0,
        end: text.folded.len(),
    };
    Some(if around.similarity > inside.similarity {
        around
    } else {
        inside
    })
}

fn ends_with_punctuation(text: &[char]) -> bool {
    text.iter()
        .rev()
        .find(|c| !c.is_whitespace())
        .is_some_and(|c| !c.is_alphanumeric())
}

/// Lowercase letters and digits without accents, other characters turned
/// into single spaces, with the original position of each.
fn fold(text: &[char]) -> (Vec<char>, Vec<usize>) {
    let mut folded = Vec::with_capacity(text.len());
    let mut positions = Vec::with_capacity(text.len());
    for (i, &c) in text.iter().enumerate() {
        let mut letter = false;
        for base in std::iter::once(c).nfd() {
            if base.is_alphanumeric() {
                folded.extend(base.to_lowercase());
                positions.resize(folded.len(), i);
                letter = true;
            }
        }
        if !letter && folded.last().is_some_and(|&l| l != ' ') {
            folded.push(' ');
            positions.push(i);
        }
    }
    if folded.last() == Some(&' ') {
        folded.pop();
        positions.pop();
    }
    (folded, positions)
}

fn trigrams(text: &[char]) -> Vec<u64> {
    text.windows(3)
        .map(|w| (w[0] as u64) << 42 | (w[1] as u64) << 21 | w[2] as u64)
        .collect()
}

/// The fewest edits turning `query` into some stretch of `text`, with that
/// stretch's bounds (Sellers' algorithm: the stretch may start anywhere).
fn substring_distance(query: &[char], text: &[char]) -> (usize, usize, usize) {
    let m = query.len();
    // For each length of query prefix: its cost, and where its stretch starts.
    let mut cost: Vec<usize> = (0..=m).collect();
    let mut start: Vec<usize> = vec![0; m + 1];
    let mut best = (cost[m], 0, 0);
    for (j, &t) in text.iter().enumerate() {
        let (mut diagonal, mut diagonal_start) = (0, j);
        cost[0] = 0;
        start[0] = j + 1;
        for i in 1..=m {
            let (up, up_start) = (cost[i], start[i]);
            let substitute = diagonal + usize::from(query[i - 1] != t);
            let (mut value, mut from) = (substitute, diagonal_start);
            if up + 1 < value {
                (value, from) = (up + 1, up_start);
            }
            if cost[i - 1] + 1 < value {
                (value, from) = (cost[i - 1] + 1, start[i - 1]);
            }
            (diagonal, diagonal_start) = (up, up_start);
            cost[i] = value;
            start[i] = from;
        }
        if cost[m] < best.0 {
            best = (cost[m], start[m], j + 1);
        }
    }
    best
}

/// The original text of `folded[start..end]`, widened to whole words, and
/// to the sentence punctuation after the last one when the OCR line ended
/// with some or the game text ends there. Brackets and quotes stay out.
fn stretch(text: &Prepared, start: usize, end: usize, punctuation: bool) -> String {
    if start >= end {
        return String::new();
    }
    let mut first = text.positions[start];
    let mut last = text.positions[end - 1];
    while first > 0 && text.original[first - 1].is_alphanumeric() {
        first -= 1;
    }
    while last + 1 < text.original.len() && text.original[last + 1].is_alphanumeric() {
        last += 1;
    }
    let sentence_end = |c: &char| ".,!?…:;".contains(*c);
    let tail = &text.original[last + 1..];
    let at_end = !tail.is_empty() && tail.iter().all(|c| !c.is_alphanumeric());
    if punctuation || at_end {
        while text.original.get(last + 1).is_some_and(sentence_end) {
            last += 1;
        }
    }
    text.original[first..=last].iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(lines: &[&str]) -> Index {
        let mut index = Index::default();
        for (i, line) in lines.iter().enumerate() {
            index.push("test".into(), i.to_string(), line, line, None);
        }
        index
    }

    #[test]
    fn folds_accents_case_and_punctuation() {
        let (folded, _) = fold(&"Bezstarostná jízda, oh!".chars().collect::<Vec<_>>());
        assert_eq!(folded.iter().collect::<String>(), "bezstarostna jizda oh");
    }

    #[test]
    fn finds_the_exact_text_despite_ocr_errors() {
        let index = index(&["Bezstarostná jízda", "Pomoz lazebnici Anně.", "Nová hra"]);
        let found = index.find("Dezstarostná jízdy", 0.8).unwrap();
        assert_eq!(found.text, "Bezstarostná jízda");
        assert!(found.similarity < 1.0);
        let found = index.find("pomoz lazebnici anne", 0.8).unwrap();
        assert_eq!(found.text, "Pomoz lazebnici Anně.");
    }

    #[test]
    fn returns_the_stretch_of_a_wrapped_line() {
        let index = index(&["Zkusím se zeptat Kuběnky. Třeba mezitím něco zjistil."]);
        let found = index.find("Třeba mezitim neco zjistil.", 0.8).unwrap();
        assert_eq!(found.text, "Třeba mezitím něco zjistil.");
    }

    #[test]
    fn finds_a_game_text_within_a_longer_line() {
        let index = index(&["Bezstarostná jízda", "Pokračovat", "(Zpět)"]);
        let found = index
            .find("Bezstarostná jízda, 0h, 06/10/2026 12:22", 0.8)
            .unwrap();
        assert_eq!(found.text, "Bezstarostná jízda");
        assert_eq!(index.find("Zpět", 0.8).unwrap().text, "Zpět");
    }

    #[test]
    fn keeps_brackets_out_and_final_punctuation_in() {
        let index = index(&[
            "(Pokračovat v cestě)",
            "Pouze hrstka věrných stojí při králi...",
        ]);
        assert_eq!(
            index.find("Pokracovat v ceste", 0.8).unwrap().text,
            "Pokračovat v cestě"
        );
        assert_eq!(
            index
                .find("Pouze hrstka věrných stoji při králi.", 0.8)
                .unwrap()
                .text,
            "Pouze hrstka věrných stojí při králi..."
        );
    }

    #[test]
    fn short_lines_match_only_whole_texts() {
        let index = index(&["Nová hra", "Hra byla uložená"]);
        assert_eq!(index.find("Nova hra", 0.8).unwrap().text, "Nová hra");
        assert!(index.find("hra", 0.8).is_none());
    }
}
