//! 2DA — the game's data tables: appearance, feats, items, spells.
//!
//! # Layout (V2.b)
//!
//! 1. Signature `2DA V2.b` and a newline.
//! 2. Column headers: each name followed by a separator, then a NUL.
//! 3. `u32` row count.
//! 4. Row labels: each followed by a separator.
//! 5. `u16` cell offsets (`row_count × column_count`), then `u16` data-block size.
//! 6. NUL-terminated cell strings in the data block.
//!
//! The separator is normally TAB. K1 also ships tables whose separator is
//! NUL: 13 of the 2DAs in `rims/global.rim` and `rims/miniglobal.rim`
//! (`appearance`, `baseitems`, `placeables`, …). Each of those is the
//! `data/2da.bif` table byte for byte, except that every TAB separator in the
//! header and the row labels is a NUL. So the header ends in two NULs and
//! every row label ends in one. [`read`] accepts both.
//!
//! Any other deviation, or any truncation, is an error. The reader does not
//! guess, so a table it cannot read fails loudly instead of returning rows
//! that look plausible and are wrong.

use std::path::Path;

use crate::error::Result;
use crate::reader::Reader;
use crate::shared::{cp1252_display, format_error};

#[derive(Clone, Debug)]
pub struct TwoDa {
    pub columns: Vec<String>,
    /// Row labels as written, usually but not always the row index.
    pub labels: Vec<String>,
    /// `rows[r][c]` — always `columns.len()` wide.
    pub rows: Vec<Vec<String>>,
}

impl TwoDa {
    pub fn column_index(&self, name: &str) -> Option<usize> {
        self.columns
            .iter()
            .position(|c| c.eq_ignore_ascii_case(name))
    }

    pub fn get(&self, row: usize, column: &str) -> Option<&str> {
        let c = self.column_index(column)?;
        self.rows.get(row)?.get(c).map(String::as_str)
    }
}

pub fn sniff(data: &[u8]) -> bool {
    data.starts_with(b"2DA ")
}

/// Read a V2.b table with TAB or NUL separators (see the module docs).
///
/// The byte layout lives in the shared `kotor-formats` crate, so this reader
/// and OdyPatcher's writer cannot drift on offsets or field widths. The result
/// is narrowed here into kq's flat, query-shaped [`TwoDa`].
pub fn read(data: &[u8], path: &Path) -> Result<TwoDa> {
    let tabbed = nul_separators_as_tabs(data);
    let data = tabbed.as_deref().unwrap_or(data);
    reject_unterminated_header(data, path)?;

    let table = kotor_formats::twoda::TwoDaFile::parse(data, &path.to_string_lossy())
        .map_err(|err| format_error(err, path))?;

    let cell = |text: &str| cp1252_display(text);
    let columns: Vec<String> = (0..table.column_count())
        .map(|c| {
            table
                .column_label(c)
                .map(cell)
                .map_err(|err| format_error(err, path))
        })
        .collect::<Result<_>>()?;

    let mut labels = Vec::with_capacity(table.row_count());
    let mut rows = Vec::with_capacity(table.row_count());
    for row in 0..table.row_count() {
        labels.push(
            table
                .row_label(row)
                .map(cell)
                .map_err(|err| format_error(err, path))?,
        );
        let mut cells = Vec::with_capacity(columns.len());
        for column in 0..table.column_count() {
            cells.push(
                table
                    .cell(row, column)
                    .map(cell)
                    .map_err(|err| format_error(err, path))?,
            );
        }
        rows.push(cells);
    }

    Ok(TwoDa {
        columns,
        labels,
        rows,
    })
}

/// Rewrite a NUL-separated table as the TAB-separated table it encodes.
///
/// Returns `None` when the table already uses TABs, or when the bytes do not
/// have the NUL-separated shape; the TAB reader then reports what is wrong.
/// Only separator bytes change, one for one, so every offset in the file
/// stays valid and the strict reader still checks the rest.
fn nul_separators_as_tabs(data: &[u8]) -> Option<Vec<u8>> {
    let start = header_start(data)?;
    let header_end = start + data.get(start..)?.iter().position(|&b| b == 0)?;
    if header_end == start || data[start..header_end].contains(&b'\t') {
        return None;
    }

    let mut out = data.to_vec();
    // Column names, each followed by a NUL, then the NUL that ends the block.
    let mut pos = start;
    loop {
        let end = pos + data.get(pos..)?.iter().position(|&b| b == 0)?;
        if end == pos {
            break;
        }
        if data[pos..end].iter().any(|&b| b < b' ' || b == 0x7f) {
            return None;
        }
        out[end] = b'\t';
        pos = end + 1;
    }
    let row_count_at = pos + 1;
    let raw = data.get(row_count_at..row_count_at + 4)?;
    let rows = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize;

    // Row labels, each followed by a NUL instead of a TAB.
    let mut pos = row_count_at + 4;
    for _ in 0..rows {
        let end = pos + data.get(pos..)?.iter().position(|&b| b == 0)?;
        out[end] = b'\t';
        pos = end + 1;
    }
    Some(out)
}

/// Where the column names start: after the signature and its line break.
fn header_start(data: &[u8]) -> Option<usize> {
    if !data.starts_with(b"2DA V2.b") {
        return None;
    }
    let mut pos = 8;
    while pos < data.len() && (data[pos] == b'\n' || data[pos] == b'\r') {
        pos += 1;
    }
    Some(pos)
}

/// Reject a column header whose last name is not TAB-terminated.
///
/// Every column name in a conformant V2.b table is followed by a TAB,
/// including the last one, and the shared reader emits a column only when it
/// sees that TAB. A header ending `…\tvalue\0` would therefore lose `value`
/// silently, so it is an error instead.
fn reject_unterminated_header(data: &[u8], path: &Path) -> Result<()> {
    let mut pos = 8;
    while pos < data.len() && (data[pos] == b'\n' || data[pos] == b'\r') {
        pos += 1;
    }
    let start = pos;
    while pos < data.len() && data[pos] != 0 {
        pos += 1;
    }
    if pos > start && data[pos - 1] != b'\t' {
        let r = Reader::new(data, path);
        return Err(r.malformed(
            "column header block is not TAB-terminated; the last column name would be lost",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_tab_2da(columns: &[&str], labels: &[&str], cells: &[&[&str]]) -> Vec<u8> {
        let mut out = b"2DA V2.b\n".to_vec();
        for c in columns {
            out.extend_from_slice(c.as_bytes());
            // Every column name is TAB-terminated, the last one included.
            out.push(b'\t');
        }
        out.push(0);
        out.extend_from_slice(&(labels.len() as u32).to_le_bytes());
        for l in labels {
            out.extend_from_slice(l.as_bytes());
            out.push(b'\t');
        }
        let mut pool = String::new();
        let mut offsets = Vec::new();
        for row in cells {
            for cell in *row {
                let entry = format!("{cell}\0");
                if let Some(off) = pool.find(&entry) {
                    offsets.push(off as u16);
                } else {
                    offsets.push(pool.len() as u16);
                    pool.push_str(&entry);
                }
            }
        }
        for off in &offsets {
            out.extend_from_slice(&off.to_le_bytes());
        }
        out.extend_from_slice(&(pool.len() as u16).to_le_bytes());
        out.extend_from_slice(pool.as_bytes());
        out
    }

    fn build_nul_header_2da(
        columns: &[&str],
        labels: &[&str],
        cells: &[&[&str]],
        padding_nul: bool,
    ) -> Vec<u8> {
        let mut out = b"2DA V2.b\n".to_vec();
        for c in columns {
            out.extend_from_slice(c.as_bytes());
            out.push(0);
        }
        if padding_nul {
            out.push(0);
        }
        out.extend_from_slice(&(labels.len() as u32).to_le_bytes());
        for l in labels {
            out.extend_from_slice(l.as_bytes());
            out.push(b'\t');
        }
        let mut pool = String::new();
        let mut offsets = Vec::new();
        for row in cells {
            for cell in *row {
                let entry = format!("{cell}\0");
                if let Some(off) = pool.find(&entry) {
                    offsets.push(off as u16);
                } else {
                    offsets.push(pool.len() as u16);
                    pool.push_str(&entry);
                }
            }
        }
        for off in &offsets {
            out.extend_from_slice(&off.to_le_bytes());
        }
        out.extend_from_slice(&(pool.len() as u16).to_le_bytes());
        out.extend_from_slice(pool.as_bytes());
        out
    }

    #[test]
    fn strict_read_tab_separated_headers() {
        let data = build_tab_2da(
            &["label", "value"],
            &["0", "1"],
            &[&["a", "1"], &["b", "2"]],
        );
        let t = read(&data, Path::new("test.2da")).unwrap();
        assert_eq!(t.columns, vec!["label", "value"]);
        assert_eq!(t.labels, vec!["0", "1"]);
        assert_eq!(t.rows[0][1], "1");
    }

    #[test]
    fn strict_read_rejects_a_header_missing_its_final_tab() {
        // Dropping the trailing TAB used to cost the last column silently;
        // it is now reported so read_or_salvage can take over.
        let mut data = b"2DA V2.b\n".to_vec();
        data.extend_from_slice(b"label\tvalue\0");
        data.extend_from_slice(&0u32.to_le_bytes());
        assert!(read(&data, Path::new("test.2da")).is_err());
    }

    #[test]
    fn strict_read_rejects_nul_header_with_tab_labels() {
        let data = build_nul_header_2da(
            &["label", "value"],
            &["0", "1"],
            &[&["a", "1"], &["b", "2"]],
            true,
        );
        assert!(read(&data, Path::new("rims/global.rim/appearance.2da")).is_err());
    }
}
