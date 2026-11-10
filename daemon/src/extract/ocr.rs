//! Parses text in Tesseract's TSV format, what the Jetson's reader sends:
//! confident, word-like tokens, each with the sentence it belongs to.

use serde::Serialize;

/// A word read from a frame.
#[derive(Debug, Clone, Serialize)]
pub struct Word {
    pub text: String,
    pub confidence: f32,
    /// `[left, top, width, height]` in pixels.
    #[serde(rename = "box")]
    pub bbox: [u32; 4],
    /// True when the word was seen for the first time in this frame.
    pub new: bool,
    /// The sentence the word was read in, for context.
    pub sentence: String,
}

/// What was read on a frame.
#[derive(Debug, Default)]
pub struct Screen {
    /// Words worth learning, with their sentence.
    pub words: Vec<Word>,
    /// All the text read, line by line, noise included: for Claude to find
    /// whole sentences in, which the daemon's own cannot always be.
    pub lines: Vec<String>,
}

impl Word {
    /// Key used to decide whether two words are the same.
    pub fn key(&self) -> String {
        self.text.to_lowercase()
    }
}

/// A token as read, before filtering.
struct Token<'a> {
    paragraph: (&'a str, &'a str),
    line: &'a str,
    text: &'a str,
    confidence: f32,
    bbox: Option<[u32; 4]>,
}

/// Parses Tesseract's TSV, keeping confident, word-like tokens, each with the
/// sentence it belongs to, and every line read.
///
/// Columns: level page block par line word left top width height conf text;
/// level 5 rows are words (the header line, when present, is skipped by that
/// check).
pub(super) fn parse_tsv(tsv: &str, min_confidence: f32) -> Screen {
    let tokens: Vec<Token> = tsv
        .lines()
        .filter_map(|line| {
            let cols: Vec<&str> = line.split('\t').collect();
            if cols.len() < 12 || cols[0] != "5" || cols[11].trim().is_empty() {
                return None;
            }
            let num = |i: usize| cols[i].parse::<u32>().ok();
            Some(Token {
                paragraph: (cols[2], cols[3]),
                line: cols[4],
                text: cols[11].trim(),
                confidence: cols[10].parse().ok()?,
                bbox: (|| Some([num(6)?, num(7)?, num(8)?, num(9)?]))(),
            })
        })
        .collect();

    let mut lines: Vec<String> = Vec::new();
    for (i, token) in tokens.iter().enumerate() {
        let same_line =
            i > 0 && tokens[i - 1].paragraph == token.paragraph && tokens[i - 1].line == token.line;
        match lines.last_mut() {
            Some(line) if same_line => {
                line.push(' ');
                line.push_str(token.text);
            }
            _ => lines.push(token.text.to_owned()),
        }
    }

    let mut words = Vec::new();
    for sentence in sentences(&tokens) {
        let text = sentence
            .iter()
            .map(|t| t.text)
            .collect::<Vec<_>>()
            .join(" ");
        for token in sentence {
            let (Some(word), Some(bbox)) = (clean_word(token.text), token.bbox) else {
                continue;
            };
            if token.confidence >= min_confidence {
                words.push(Word {
                    text: word,
                    confidence: token.confidence,
                    bbox,
                    new: false,
                    sentence: text.clone(),
                });
            }
        }
    }
    Screen { words, lines }
}

/// Confidence under which a token is OCR noise rather than text. Real text
/// in games reads at 90+, scenery misread as letters mostly under 60.
const JUNK_CONFIDENCE: f32 = 60.0;

/// Splits tokens into sentences.
///
/// A sentence ends at a paragraph change, or after a token ending with `.`,
/// `!`, `?` or `…` (ignoring closing quotes and brackets) unless the next
/// token starts in lowercase: Czech writes ordinals with a dot ("15.
/// století", "Václav IV. byl"). Junk tokens (noise from game scenery) are
/// left out and also break sentences, so scattered fragments do not get
/// attached to real text.
fn sentences<'a, 'b>(tokens: &'b [Token<'a>]) -> Vec<Vec<&'b Token<'a>>> {
    let is_junk =
        |t: &Token| t.confidence < JUNK_CONFIDENCE || !t.text.chars().any(char::is_alphanumeric);
    let ends_sentence = |text: &str| {
        text.trim_end_matches(['"', '\'', '“', '”', '„', ')', ']', '»'])
            .ends_with(['.', '!', '?', '…'])
    };
    let starts_lowercase = |text: &str| {
        text.chars()
            .find(|c| c.is_alphabetic())
            .is_some_and(char::is_lowercase)
    };

    let mut sentences = Vec::new();
    let mut current: Vec<&Token> = Vec::new();
    for (i, token) in tokens.iter().enumerate() {
        let next = tokens.get(i + 1);
        if is_junk(token) {
            if !current.is_empty() {
                sentences.push(std::mem::take(&mut current));
            }
            continue;
        }
        current.push(token);
        let paragraph_ends = next.is_none_or(|next| next.paragraph != token.paragraph);
        let sentence_ends =
            ends_sentence(token.text) && !next.is_some_and(|n| starts_lowercase(n.text));
        if paragraph_ends || sentence_ends {
            sentences.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        sentences.push(current);
    }
    sentences
}

/// Strips surrounding punctuation and rejects tokens that are not words:
/// OCR on game scenery produces many short or mixed-symbol fragments.
fn clean_word(raw: &str) -> Option<String> {
    let word = raw.trim_matches(|c: char| !c.is_alphanumeric());
    let letters = word.chars().count();
    (letters >= 2 && word.chars().all(char::is_alphabetic)).then(|| word.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_confident_words() {
        let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
                   1\t1\t0\t0\t0\t0\t0\t0\t2560\t1440\t-1\t\n\
                   5\t1\t1\t1\t1\t1\t100\t200\t80\t20\t95.5\tJindřich,\n\
                   5\t1\t1\t1\t1\t2\t190\t200\t40\t20\t40.0\tmeč\n\
                   5\t1\t1\t1\t1\t3\t240\t200\t10\t20\t96.0\ta\n\
                   5\t1\t1\t1\t1\t4\t260\t200\t30\t20\t96.0\tx7z\n";
        let screen = parse_tsv(tsv, 70.0);
        assert_eq!(screen.lines, ["Jindřich, meč a x7z"]);
        let words = screen.words;
        assert_eq!(words.len(), 1);
        assert_eq!(words[0].text, "Jindřich");
        assert_eq!(words[0].bbox, [100, 200, 80, 20]);
        assert_eq!(words[0].key(), "jindřich");
        // "meč" (confidence 40) is noise: left out of the sentence too.
        assert_eq!(words[0].sentence, "Jindřich,");
    }

    #[test]
    fn splits_sentences_across_lines_and_paragraphs() {
        let row = |block: u32, line: u32, text: &str| {
            format!("5\t1\t{block}\t1\t{line}\t1\t0\t0\t10\t10\t95\t{text}\n")
        };
        let tsv = [
            row(1, 1, "Ponořte"),
            row(1, 1, "se"),
            row(1, 2, "do"),
            row(1, 2, "příběhu."),
            row(1, 2, "Tato"),
            row(1, 3, "aktualizace!"),
            row(2, 1, "Nová"),
            row(2, 1, "hra"),
            row(3, 1, "Na"),
            row(3, 1, "počátku"),
            row(3, 1, "15."),
            row(3, 1, "století"),
            row(3, 1, "válka."),
            row(4, 1, "Deš"),
            row(4, 1, "nám?"),
        ]
        .concat();
        // Scenery noise before a subtitle: low confidence, or symbols only.
        let noise =
            "5\t1\t4\t1\t1\t1\t0\t0\t10\t10\t14\ts\n5\t1\t4\t1\t1\t1\t0\t0\t10\t10\t91\t$\n";
        let tsv = tsv.replace(
            "5\t1\t4\t1\t1\t1\t0\t0\t10\t10\t95\tDeš",
            &format!("{noise}5\t1\t4\t1\t1\t1\t0\t0\t10\t10\t95\tDeš"),
        );
        let sentence_of = |word: &str| {
            parse_tsv(&tsv, 70.0)
                .words
                .into_iter()
                .find(|w| w.text == word)
                .unwrap()
                .sentence
        };
        assert_eq!(sentence_of("se"), "Ponořte se do příběhu.");
        assert_eq!(sentence_of("příběhu"), "Ponořte se do příběhu.");
        assert_eq!(sentence_of("Tato"), "Tato aktualizace!");
        assert_eq!(sentence_of("hra"), "Nová hra");
        assert_eq!(sentence_of("století"), "Na počátku 15. století válka.");
        assert_eq!(sentence_of("nám"), "Deš nám?");
    }

    #[test]
    fn reads_the_jetsons_output() {
        // What champollion-gpu-ocr --frames prints: a wrapped subtitle as
        // two lines of one block, and a menu item in another.
        let tsv = "5\t1\t1\t1\t1\t1\t100\t200\t160\t30\t99.90\tNastavení\n\
                   5\t1\t2\t1\t1\t1\t1000\t1000\t120\t30\t99.90\tBohuto!\n\
                   5\t1\t2\t1\t1\t2\t1130\t1000\t90\t30\t99.50\tVytas\n\
                   5\t1\t2\t1\t2\t1\t1000\t1036\t60\t30\t98.00\tmeč\n";
        let screen = parse_tsv(tsv, 70.0);
        let words: Vec<(&str, &str)> = screen
            .words
            .iter()
            .map(|w| (w.text.as_str(), w.sentence.as_str()))
            .collect();
        assert_eq!(
            words,
            [
                ("Nastavení", "Nastavení"),
                ("Bohuto", "Bohuto!"),
                ("Vytas", "Vytas meč"),
                ("meč", "Vytas meč"),
            ]
        );
        assert_eq!(screen.lines, ["Nastavení", "Bohuto! Vytas", "meč"]);
        assert_eq!(screen.words[2].bbox, [1130, 1000, 90, 30]);
    }
}
