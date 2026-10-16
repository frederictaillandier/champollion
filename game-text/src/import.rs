//! Copies the text of Kingdom Come: Deliverance II from its localization
//! archives into a SQLite database.
//!
//! `Localization/<Language>_xml.pak` is a zip of `text_ui_*.xml` files, one
//! row per line of text: `<Row><Cell>id</Cell><Cell>text</Cell><Cell>text</Cell></Row>`.
//! The two Czech texts differ in about 40% of the dialogue rows, the second
//! often with spoken forms ("nevim" for "nevím"), so both are kept.

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use rusqlite::Connection;

pub const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS lines (
    -- File it comes from, without .xml: text_ui_dialog, text_ui_quest...
    file TEXT NOT NULL,
    id TEXT NOT NULL,
    -- The row's first and second text, markup removed.
    czech TEXT NOT NULL,
    czech_alt TEXT NOT NULL,
    english TEXT,
    PRIMARY KEY (file, id)
)";

/// Reads `Czech_xml.pak` and `English_xml.pak` from `localization` into
/// `db`, replacing what it held. Returns the number of lines.
pub fn import(localization: &Path, db: &Path) -> Result<usize, String> {
    let czech = read_pak(&localization.join("Czech_xml.pak"))?;
    let english: HashMap<(String, String), String> =
        read_pak(&localization.join("English_xml.pak"))?
            .into_iter()
            .map(|row| ((row.file, row.id), row.second))
            .collect();

    let mut conn = Connection::open(db).map_err(|e| e.to_string())?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    tx.execute_batch(&format!("DROP TABLE IF EXISTS lines; {SCHEMA}"))
        .map_err(|e| e.to_string())?;
    {
        let mut insert = tx
            .prepare(
                "INSERT OR REPLACE INTO lines (file, id, czech, czech_alt, english)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .map_err(|e| e.to_string())?;
        for row in &czech {
            let english = english.get(&(row.file.clone(), row.id.clone()));
            insert
                .execute((&row.file, &row.id, &row.first, &row.second, english))
                .map_err(|e| e.to_string())?;
        }
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(czech.len())
}

struct Row {
    file: String,
    id: String,
    first: String,
    second: String,
}

fn read_pak(path: &Path) -> Result<Vec<Row>, String> {
    let file = File::open(path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut rows = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name().map_err(|e| e.to_string())?;
        let Some(name) = name.strip_suffix(".xml").map(str::to_owned) else {
            continue;
        };
        let mut xml = String::new();
        entry
            .read_to_string(&mut xml)
            .map_err(|e| format!("{name}: {e}"))?;
        rows.extend(parse_rows(&xml).into_iter().filter_map(|cells| {
            let [id, first, second] = <[String; 3]>::try_from(cells).ok()?;
            Some(Row {
                file: name.clone(),
                id,
                first: clean(&first),
                second: clean(&second),
            })
        }));
    }
    Ok(rows)
}

/// The cells of each `<Row>`, still escaped.
fn parse_rows(xml: &str) -> Vec<Vec<String>> {
    xml.split("<Row>")
        .skip(1)
        .map(|row| {
            let row = row.split("</Row>").next().unwrap_or_default();
            let mut cells = Vec::new();
            let mut rest = row;
            while let Some(start) = rest.find("<Cell") {
                rest = &rest[start + 5..];
                if let Some(after) = rest.strip_prefix("/>") {
                    cells.push(String::new());
                    rest = after;
                } else if let Some(open) = rest.find('>') {
                    let body = &rest[open + 1..];
                    let end = body.find("</Cell>").unwrap_or(body.len());
                    cells.push(body[..end].to_owned());
                    rest = &body[end..];
                } else {
                    break;
                }
            }
            cells
        })
        .collect()
}

/// The text as shown: entities decoded, markup (`<br/>`, `<paragraph>`...)
/// removed, spaces collapsed.
fn clean(cell: &str) -> String {
    let text = unescape(cell);
    let mut plain = String::with_capacity(text.len());
    let mut in_tag = false;
    for c in text.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                plain.push(' ');
            }
            _ if !in_tag => plain.push(c),
            _ => {}
        }
    }
    plain.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let Some(semi) = rest.find(';').filter(|&s| s <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let decoded = match &rest[1..semi] {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            code => code
                .strip_prefix("#x")
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| code.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rows_and_cleans_markup() {
        let xml = "<Table>\n<Row><Cell>a_1</Cell><Cell>&lt;paragraph&gt;Pomoz&lt;br/&gt;lazebnici\
                   &lt;/paragraph&gt;</Cell><Cell>Pomoz lazebnici Anně.</Cell></Row>\n\
                   <Row><Cell>b</Cell><Cell/><Cell>A &amp; B</Cell></Row>\n</Table>";
        let rows = parse_rows(xml);
        assert_eq!(rows.len(), 2);
        assert_eq!(clean(&rows[0][1]), "Pomoz lazebnici");
        assert_eq!(clean(&rows[0][2]), "Pomoz lazebnici Anně.");
        assert_eq!(rows[1], vec!["b", "", "A &amp; B"]);
        assert_eq!(clean(&rows[1][2]), "A & B");
    }
}
