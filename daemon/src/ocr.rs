use image::GrayImage;
use serde::Serialize;
use tesseract::Tesseract;

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

impl Word {
    /// Key used to decide whether two words are the same.
    pub fn key(&self) -> String {
        self.text.to_lowercase()
    }
}

#[derive(Clone)]
pub struct Ocr {
    pub lang: String,
    pub min_confidence: f32,
}

impl Ocr {
    /// Checks that libtesseract can load the language data.
    pub fn check(&self) -> Result<(), String> {
        self.engine().map(drop)
    }

    fn engine(&self) -> Result<Tesseract, String> {
        Tesseract::new(None, Some(&self.lang))
            .map_err(|e| {
                format!(
                    "tesseract could not load the '{}' language data: {e}",
                    self.lang
                )
            })?
            // Tesseract prints warnings like "Image too small to scale" on
            // stderr for every speck it tries to read in game scenery.
            .set_variable("debug_file", "/dev/null")
            .map_err(|e| e.to_string())
    }

    /// A reader owning its own Tesseract engine, for one thread.
    pub fn reader(&self) -> Reader {
        Reader {
            ocr: self.clone(),
            engine: None,
        }
    }
}

/// Reads frames with a Tesseract engine that is created once and reused, so
/// the language model is only loaded once per thread.
pub struct Reader {
    ocr: Ocr,
    engine: Option<Tesseract>,
}

impl Reader {
    pub fn read(&mut self, image: &GrayImage) -> Result<Vec<Word>, String> {
        let engine = match self.engine.take() {
            Some(engine) => engine,
            None => self.ocr.engine()?,
        };
        let (width, height) = (image.width() as i32, image.height() as i32);
        // The engine is consumed by each step and only handed back on
        // success; after an error it is dropped and recreated next frame.
        let mut engine = engine
            .set_frame(image.as_raw(), width, height, 1, width)
            .map_err(|e| e.to_string())?
            .recognize()
            .map_err(|e| e.to_string())?;
        let tsv = engine.get_tsv_text(0).map_err(|e| e.to_string())?;
        self.engine = Some(engine);
        Ok(parse_tsv(&tsv, self.ocr.min_confidence))
    }
}

/// A token as tesseract returned it, before filtering.
struct Token<'a> {
    paragraph: (&'a str, &'a str),
    text: &'a str,
    confidence: f32,
    bbox: Option<[u32; 4]>,
}

/// Parses tesseract's TSV output, keeping confident, word-like tokens, each
/// with the sentence it belongs to.
///
/// Columns: level page block par line word left top width height conf text;
/// level 5 rows are words (the header line, when present, is skipped by that
/// check).
fn parse_tsv(tsv: &str, min_confidence: f32) -> Vec<Word> {
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
                text: cols[11].trim(),
                confidence: cols[10].parse().ok()?,
                bbox: (|| Some([num(6)?, num(7)?, num(8)?, num(9)?]))(),
            })
        })
        .collect();

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
    words
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
        let words = parse_tsv(tsv, 70.0);
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
}
