//! Replaces each line read with the game's own text when it is found there
//! (`game-text`), so misread letters and lost accents come back before the
//! daemon looks for words. Lines not in the game, like Steam's overlay,
//! stay as read.

use std::collections::HashMap;
use std::path::Path;

use game_text::matcher::Index;

/// Share of a line's characters that must match a game text to replace it.
const MIN_SIMILARITY: f32 = 0.8;
/// Lines remembered: text stays on screen for many frames.
const CACHE: usize = 10_000;

pub struct Corrector {
    index: Index,
    /// Line read -> the game's text for it and how close it was.
    cache: HashMap<String, Option<(String, f32)>>,
}

impl Corrector {
    /// Loads the database made by `champollion-game-text import`.
    pub fn load(db: &Path) -> Result<Self, String> {
        Ok(Self {
            index: Index::load(db)?,
            cache: HashMap::new(),
        })
    }

    pub fn lines(&self) -> usize {
        self.index.len()
    }

    /// Tesseract's TSV with the words of each line found in the game's text
    /// replaced by that text's. A replaced word gets a box spread along its
    /// line's, and the match's similarity as its confidence.
    pub fn correct(&mut self, tsv: &str) -> String {
        let rows: Vec<Vec<&str>> = tsv.lines().map(|l| l.split('\t').collect()).collect();
        let mut out = String::with_capacity(tsv.len());
        let mut start = 0;
        while start < rows.len() {
            if !is_word(&rows[start]) {
                out.push_str(&rows[start].join("\t"));
                out.push('\n');
                start += 1;
                continue;
            }
            // The words of one line: same block, paragraph and line.
            let mut end = start + 1;
            while end < rows.len() && is_word(&rows[end]) && rows[end][2..5] == rows[start][2..5] {
                end += 1;
            }
            let line = &rows[start..end];
            let read: Vec<&str> = line.iter().map(|r| r[11]).collect();
            match self.find(&read.join(" ")) {
                Some((text, similarity)) => replace(&mut out, line, &text, similarity),
                None => {
                    for row in line {
                        out.push_str(&row.join("\t"));
                        out.push('\n');
                    }
                }
            }
            start = end;
        }
        out
    }

    fn find(&mut self, line: &str) -> Option<(String, f32)> {
        if let Some(found) = self.cache.get(line) {
            return found.clone();
        }
        if self.cache.len() >= CACHE {
            self.cache.clear();
        }
        let found = self
            .index
            .find(line, MIN_SIMILARITY)
            .map(|f| (f.text, f.similarity));
        self.cache.insert(line.to_owned(), found.clone());
        found
    }
}

fn is_word(row: &[&str]) -> bool {
    row.len() >= 12 && row[0] == "5"
}

/// Writes `text` as the words of `line`: its own boxes when it has as many
/// words, otherwise boxes spread along the line by character position.
fn replace(out: &mut String, line: &[Vec<&str>], text: &str, similarity: f32) {
    let num = |row: &[&str], i: usize| row[i].parse::<f32>().unwrap_or(0.0);
    let left = line.iter().map(|r| num(r, 6)).fold(f32::MAX, f32::min);
    let right = line
        .iter()
        .map(|r| num(r, 6) + num(r, 8))
        .fold(0.0, f32::max);
    let words: Vec<(usize, &str)> = text
        .split_whitespace()
        .scan(0, |offset, word| {
            let at = text[*offset..].find(word).map_or(*offset, |i| *offset + i);
            *offset = at + word.len();
            Some((text[..at].chars().count(), word))
        })
        .collect();
    let chars = text.chars().count().max(1) as f32;
    let confidence = format!("{:.2}", similarity * 100.0);
    let first = &line[0];
    for (n, (at, word)) in words.iter().enumerate() {
        let (x, width) = if words.len() == line.len() {
            (num(&line[n], 6), num(&line[n], 8))
        } else {
            let span = right - left;
            let x = left + span * *at as f32 / chars;
            let width = span * word.chars().count() as f32 / chars;
            (x, width)
        };
        let row = [
            "5",
            first[1],
            first[2],
            first[3],
            first[4],
            &(n + 1).to_string(),
            &(x as u32).to_string(),
            first[7],
            &(width.max(1.0) as u32).to_string(),
            first[9],
            &confidence,
            word,
        ];
        out.push_str(&row.join("\t"));
        out.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corrector(lines: &[&str]) -> Corrector {
        let mut index = Index::default();
        for (i, line) in lines.iter().enumerate() {
            index.push("test".into(), i.to_string(), line, line, None);
        }
        Corrector {
            index,
            cache: HashMap::new(),
        }
    }

    fn words(tsv: &str) -> Vec<(String, String)> {
        tsv.lines()
            .map(|l| {
                let c: Vec<&str> = l.split('\t').collect();
                (c[11].to_owned(), c[6].to_owned())
            })
            .collect()
    }

    #[test]
    fn puts_back_lost_accents_and_keeps_boxes() {
        let mut c = corrector(&["Je České království zmítáno válkou."]);
        let tsv = "5\t1\t3\t1\t1\t1\t100\t50\t30\t20\t99.00\tje\n\
                   5\t1\t3\t1\t1\t2\t140\t50\t60\t20\t98.00\tceské\n\
                   5\t1\t3\t1\t1\t3\t210\t50\t90\t20\t99.00\tkrálovství\n\
                   5\t1\t3\t1\t1\t4\t310\t50\t80\t20\t97.00\tzmitáno\n\
                   5\t1\t3\t1\t1\t5\t400\t50\t70\t20\t99.00\tválkou.\n";
        let words = words(&c.correct(tsv));
        let texts: Vec<&str> = words.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(texts, ["Je", "České", "království", "zmítáno", "válkou."]);
        assert_eq!(words[1].1, "140");
    }

    #[test]
    fn spreads_words_when_their_count_differs() {
        let mut c = corrector(&["Země bez pána upadá do chaosu."]);
        // OCR merged and split words: 5 words read, 6 in the game.
        let tsv = "5\t1\t1\t1\t1\t1\t0\t10\t60\t20\t95.00\tZemé\n\
                   5\t1\t1\t1\t1\t2\t70\t10\t90\t20\t95.00\tbez/pana\n\
                   5\t1\t1\t1\t1\t3\t170\t10\t110\t20\t95.00\tupadádo\n\
                   5\t1\t1\t1\t1\t4\t290\t10\t100\t20\t95.00\tchaosu.\n";
        let words = words(&c.correct(tsv));
        let texts: Vec<&str> = words.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(texts, ["Země", "bez", "pána", "upadá", "do", "chaosu."]);
        assert_eq!(words[0].1, "0");
    }

    #[test]
    fn leaves_text_not_in_the_game_as_read() {
        let mut c = corrector(&["Nová hra"]);
        let tsv = "5\t1\t1\t1\t1\t1\t0\t0\t50\t20\t99.00\tFRIENDS\n\
                   5\t1\t2\t1\t1\t1\t0\t40\t50\t20\t99.00\tNova\n\
                   5\t1\t2\t1\t1\t2\t60\t40\t40\t20\t99.00\thra\n";
        let texts: Vec<String> = words(&c.correct(tsv)).into_iter().map(|(t, _)| t).collect();
        assert_eq!(texts, ["FRIENDS", "Nová", "hra"]);
    }
}
