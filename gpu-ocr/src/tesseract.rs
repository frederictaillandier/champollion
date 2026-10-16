//! What a frame shows, written in Tesseract's TSV format, so the daemon reads
//! the Jetson's results with the same code as its own Tesseract's.

use crate::detect::TextBox;
use crate::recognize::Word;

/// Two lines belong to one paragraph when the second starts at most this
/// many line heights below the first (boxes may overlap a little)...
const PARAGRAPH_GAP: f32 = 0.8;
/// ...their heights differ by at most this factor, and they overlap
/// horizontally.
const PARAGRAPH_HEIGHTS: f32 = 1.4;

/// Word rows (level 5) of Tesseract's TSV: level, page, block, paragraph,
/// line, word, left, top, width, height, confidence, text. Lines are in
/// reading order, each paragraph a block of its own.
pub fn tsv(lines: &[TextBox], words: &[Vec<Word>]) -> String {
    let mut order: Vec<usize> = (0..lines.len()).collect();
    order.sort_by_key(|&i| (lines[i].y, lines[i].x));
    let mut out = String::new();
    let (mut block, mut line_number) = (0, 0);
    let mut previous: Option<&TextBox> = None;
    for &i in &order {
        let line = &lines[i];
        if previous.is_some_and(|p| same_paragraph(p, line)) {
            line_number += 1;
        } else {
            block += 1;
            line_number = 1;
        }
        previous = Some(line);
        for (n, word) in words[i].iter().enumerate() {
            let left = line.x + (word.start * line.width as f32) as u32;
            let width = (((word.end - word.start) * line.width as f32) as u32).max(1);
            out.push_str(&format!(
                "5\t1\t{block}\t1\t{line_number}\t{}\t{left}\t{}\t{width}\t{}\t{:.2}\t{}\n",
                n + 1,
                line.y,
                line.height,
                word.confidence,
                word.text.replace(['\t', '\n'], " "),
            ));
        }
    }
    out
}

fn same_paragraph(above: &TextBox, below: &TextBox) -> bool {
    let height = above.height as f32;
    let gap = below.y as f32 - (above.y + above.height) as f32;
    let ratio =
        above.height.max(below.height) as f32 / above.height.min(below.height).max(1) as f32;
    let overlap = below.x < above.x + above.width && above.x < below.x + below.width;
    gap >= -0.5 * height && gap <= PARAGRAPH_GAP * height && ratio <= PARAGRAPH_HEIGHTS && overlap
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(x: u32, y: u32, width: u32) -> TextBox {
        TextBox {
            x,
            y,
            width,
            height: 30,
        }
    }

    fn word(text: &str, start: f32, end: f32) -> Word {
        Word {
            text: text.into(),
            confidence: 99.0,
            start,
            end,
        }
    }

    #[test]
    fn groups_a_wrapped_subtitle_and_keeps_a_menu_apart() {
        let lines = [
            line(800, 1000, 600),
            line(820, 1036, 400),
            line(100, 200, 200),
        ];
        let words = vec![
            vec![word("Bohuto!", 0.0, 0.3), word("Vytas", 0.35, 0.6)],
            vec![word("pojď", 0.0, 0.5)],
            vec![word("Nastavení", 0.0, 1.0)],
        ];
        let rows: Vec<Vec<String>> = tsv(&lines, &words)
            .lines()
            .map(|r| r.split('\t').map(str::to_owned).collect())
            .collect();
        // The menu item first (higher up), alone in block 1.
        assert_eq!(rows[0][2..5], ["1", "1", "1"]);
        assert_eq!(rows[0][11], "Nastavení");
        // The subtitle's two lines share block 2.
        assert_eq!(rows[1][2..5], ["2", "1", "1"]);
        assert_eq!(rows[3][2..5], ["2", "1", "2"]);
        // "Vytas" starts 35% along its 600-pixel line.
        assert_eq!(rows[2][6], "1010");
        assert_eq!(rows[2][11], "Vytas");
    }
}
