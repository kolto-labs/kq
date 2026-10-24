//! Two-dimensional table files (`2DA V2.b`).
//!
//! Layout on disk: the `2DA V2.b` signature and a line feed, tab-separated
//! column labels ended by a NUL, a row count, tab-separated row labels, a grid
//! of 16-bit offsets into the data area, two padding bytes, then the data area
//! of NUL-terminated cell strings.
//!
//! Cells holding the default marker `****` are written as an empty string, and
//! empty strings read back as `****`, so a load/save cycle is stable.

use crate::error::{PatchError, Result};
use crate::latin1;

const SIGNATURE: &[u8; 8] = b"2DA V2.b";
const OLD_SIGNATURE: &[u8; 8] = b"2DA V2.0";
const DEFAULT_CELL: &str = "****";

/// Largest row count accepted, guarding against a corrupt header.
const MAX_ROWS: u32 = 9999;

/// A loaded table.
#[derive(Debug, Clone, Default)]
pub struct TwoDaFile {
    column_labels: Vec<String>,
    row_labels: Vec<String>,
    cells: Vec<Vec<String>>,
    /// Data-size / padding bytes from the last load. Save recomputes size.
    #[allow(dead_code)]
    padding: [u8; 2],
    path: String,
    loaded: bool,
}

impl TwoDaFile {
    /// An empty, unloaded table.
    pub fn new() -> Self {
        Self::default()
    }

    /// True once a table has been read successfully.
    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    /// Path the table was read from.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Number of data rows.
    pub fn row_count(&self) -> usize {
        self.row_labels.len()
    }

    /// Number of columns.
    pub fn column_count(&self) -> usize {
        self.column_labels.len()
    }

    /// Label of a column.
    pub fn column_label(&self, index: usize) -> Result<&str> {
        self.column_labels
            .get(index)
            .map(String::as_str)
            .ok_or_else(|| {
                PatchError::twoda(
                    14,
                    "Invalid column index specified, unable to look up column label.",
                )
            })
    }

    /// Label of a row.
    pub fn row_label(&self, index: usize) -> Result<&str> {
        self.row_labels
            .get(index)
            .map(String::as_str)
            .ok_or_else(|| {
                PatchError::twoda(
                    16,
                    "Invalid row index specified, unable to look up row label.",
                )
            })
    }

    /// Rename a column.
    pub fn set_column_label(&mut self, index: usize, label: &str) -> Result<()> {
        let slot = self.column_labels.get_mut(index).ok_or_else(|| {
            PatchError::twoda(
                21,
                "Invalid column index specified, unable to set column label.",
            )
        })?;
        *slot = label.to_string();
        Ok(())
    }

    /// Rename a row.
    pub fn set_row_label(&mut self, index: usize, label: &str) -> Result<()> {
        let slot = self.row_labels.get_mut(index).ok_or_else(|| {
            PatchError::twoda(23, "Invalid row index specified, unable to set row label.")
        })?;
        *slot = label.to_string();
        Ok(())
    }

    /// Value of one cell.
    pub fn cell(&self, row: usize, column: usize) -> Result<&str> {
        let row_cells = self.cells.get(row).ok_or_else(|| {
            PatchError::twoda(
                18,
                "Invalid row index specified, unable to look up cell value.",
            )
        })?;
        row_cells.get(column).map(String::as_str).ok_or_else(|| {
            PatchError::twoda(
                19,
                "Invalid column index specified, unable to look up cell value.",
            )
        })
    }

    /// Overwrite one cell.
    pub fn set_cell(&mut self, row: usize, column: usize, value: &str) -> Result<()> {
        let row_cells = self.cells.get_mut(row).ok_or_else(|| {
            PatchError::twoda(25, "Invalid row index specified, unable to set cell value.")
        })?;
        let slot = row_cells.get_mut(column).ok_or_else(|| {
            PatchError::twoda(
                26,
                "Invalid column index specified, unable to set cell value.",
            )
        })?;
        *slot = value.to_string();
        Ok(())
    }

    /// Index of the column with this label, ignoring case.
    pub fn column_by_label(&self, label: &str) -> Result<usize> {
        self.column_labels
            .iter()
            .position(|l| l.eq_ignore_ascii_case(label))
            .ok_or_else(|| {
                PatchError::twoda(
                    8,
                    format!(
                        "Unable to find a column matching the label {} in {}!",
                        label, self.path
                    ),
                )
            })
    }

    /// Index of the row with this label, ignoring case.
    pub fn row_by_label(&self, label: &str) -> Result<usize> {
        self.row_labels
            .iter()
            .position(|l| l.eq_ignore_ascii_case(label))
            .ok_or_else(|| {
                PatchError::twoda(
                    10,
                    format!("Unable to find a row matching the label {label}!"),
                )
            })
    }

    /// Append a blank row and return its index.
    ///
    /// The new row is labelled with its own index and every cell starts at the
    /// default marker.
    pub fn add_row(&mut self) -> Result<usize> {
        self.require_loaded(31)?;

        let index = self.row_labels.len();
        self.row_labels.push(index.to_string());
        self.cells
            .push(vec![DEFAULT_CELL.to_string(); self.column_labels.len()]);

        Ok(index)
    }

    /// Append a blank column and return its index.
    pub fn add_column(&mut self) -> Result<usize> {
        self.require_loaded(29)?;

        let index = self.column_labels.len();
        self.column_labels.push(format!("Column{}", index + 1));
        for row in &mut self.cells {
            row.push(DEFAULT_CELL.to_string());
        }

        Ok(index)
    }

    /// Copy an existing row to the end of the table.
    ///
    /// Returns `None` when the source row does not exist. An empty `new_label`
    /// means the copy is labelled with its own index.
    pub fn clone_row(&mut self, source: usize, new_label: &str) -> Result<Option<usize>> {
        self.require_loaded(30)?;

        if source >= self.row_labels.len() {
            return Ok(None);
        }

        let index = self.add_row()?;
        if !new_label.is_empty() {
            self.row_labels[index] = new_label.to_string();
        }

        self.cells[index] = self.cells[source].clone();
        Ok(Some(index))
    }

    fn require_loaded(&self, code: i32) -> Result<()> {
        if self.loaded {
            Ok(())
        } else {
            Err(PatchError::twoda(
                code,
                "No 2da file has been loaded. Unable to look up column labels.",
            ))
        }
    }

    /// Parse table bytes.
    pub fn parse(bytes: &[u8], path: &str) -> Result<Self> {
        let mut reader = Cursor::new(bytes);

        let signature = reader.take(8).ok_or_else(|| {
            PatchError::twoda(
                2,
                "Specified file is not a valid binary 2DA file. Unable to load.",
            )
        })?;

        if signature == OLD_SIGNATURE {
            return Err(PatchError::twoda(
                1,
                "Specified file was a 2DA file but not in binary format. That format is unhandled at this time. Unable to load.",
            ));
        }
        if signature != SIGNATURE {
            return Err(PatchError::twoda(
                2,
                "Specified file is not a valid binary 2DA file. Unable to load.",
            ));
        }

        // Line feed following the signature.
        reader.skip(1);

        let column_labels = read_labels_until_nul(&mut reader)?;
        let row_count = reader.read_u32().ok_or_else(|| {
            PatchError::twoda(
                3,
                "Malformatted data encountered while reading 2DA! Unable to continue!",
            )
        })?;

        if row_count > MAX_ROWS {
            return Err(PatchError::twoda(
                4,
                "Sanity check failed, number of 2DA rows reported as unrealistically large! Load aborted.",
            ));
        }

        let row_count = row_count as usize;
        let column_count = column_labels.len();
        let row_labels = read_row_labels(&mut reader, row_count)?;

        let mut cells = Vec::new();
        let mut padding = [0u8; 2];

        if row_count > 0 && column_count > 0 {
            let mut offsets = vec![vec![0u16; column_count]; row_count];
            for row in offsets.iter_mut() {
                for slot in row.iter_mut() {
                    *slot = reader.read_u16().ok_or_else(|| {
                        PatchError::twoda(
                            5,
                            "Attempted to read past end of file while reading 2DA cell entry. Aborting...",
                        )
                    })?;
                }
            }

            padding[0] = reader.read_u8().unwrap_or(0);
            padding[1] = reader.read_u8().unwrap_or(0);

            let data_start = reader.position();

            cells = vec![vec![String::new(); column_count]; row_count];
            for (r, row) in cells.iter_mut().enumerate() {
                for (c, cell) in row.iter_mut().enumerate() {
                    let at = data_start + offsets[r][c] as usize;
                    if at > bytes.len() {
                        return Err(PatchError::twoda(
                            5,
                            "Attempted to read past end of file while reading 2DA cell entry. Aborting...",
                        ));
                    }
                    let raw = read_nul_terminated(bytes, at);
                    *cell = if raw.is_empty() {
                        DEFAULT_CELL.to_string()
                    } else {
                        raw
                    };
                }
            }
        }

        Ok(Self {
            column_labels,
            row_labels,
            cells,
            padding,
            path: path.to_string(),
            loaded: true,
        })
    }

    /// Read a table from disk.
    pub fn load(path: &str) -> Result<Self> {
        let bytes = crate::fsutil::read_file(path)
            .map_err(|e| PatchError::twoda(2, format!("Unable to read 2DA file {path}: {e}")))?;
        Self::parse(&bytes, path)
    }

    /// Serialize the table back to bytes.
    ///
    /// Identical cell strings share one copy in the data area. Deduplication is
    /// global (first write wins), matching HoloPatcher / PyKotor
    /// `TwoDABinaryWriter` — not the narrower left/above rectangle used by some
    /// Delphi TSLPatcher builds. The two bytes after the offset grid are the
    /// data-area length (`uint16` LE), again matching PyKotor.
    // The offset grid is addressed by row and column together, which index
    // arithmetic expresses more directly than nested iterators.
    #[allow(clippy::needless_range_loop)]
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        if !self.loaded {
            return Err(PatchError::twoda(
                27,
                "No 2da file has been loaded that can be saved.",
            ));
        }
        if self.column_labels.is_empty() || self.row_labels.is_empty() {
            return Err(PatchError::twoda(
                28,
                "The open 2da file is empty. There is no point in saving it.",
            ));
        }

        let rows = self.row_labels.len();
        let columns = self.column_labels.len();
        let mut out: Vec<u8> = Vec::new();

        out.extend_from_slice(SIGNATURE);
        out.push(b'\n');

        for label in &self.column_labels {
            out.extend_from_slice(&latin1::encode(label));
            out.push(b'\t');
        }
        out.push(0);

        out.extend_from_slice(&(rows as u32).to_le_bytes());

        for label in &self.row_labels {
            out.extend_from_slice(&latin1::encode(label));
            out.push(b'\t');
        }

        // Reserve the offset grid; real values are filled in once known.
        let offset_table_at = out.len();
        out.extend(std::iter::repeat(0u8).take(rows * columns * 2));

        // Placeholder for data-area size (filled after the pool is written).
        let data_size_at = out.len();
        out.extend_from_slice(&[0u8, 0u8]);

        let data_start = out.len();
        let mut offsets = vec![vec![0u16; columns]; rows];
        // Global first-seen map: cell text (**** → empty) → pool offset.
        let mut seen: std::collections::HashMap<String, u16> = std::collections::HashMap::new();

        for r in 0..rows {
            for c in 0..columns {
                let key = if self.cells[r][c] == DEFAULT_CELL {
                    String::new()
                } else {
                    self.cells[r][c].clone()
                };
                if let Some(&existing) = seen.get(&key) {
                    offsets[r][c] = existing;
                    continue;
                }

                let here = out.len() - data_start;
                let here_u16 = u16::try_from(here).map_err(|_| {
                    PatchError::twoda(
                        27,
                        "2DA text data exceeds the 64 KB the format can address.",
                    )
                })?;
                offsets[r][c] = here_u16;
                seen.insert(key.clone(), here_u16);

                if !key.is_empty() {
                    out.extend_from_slice(&latin1::encode(&key));
                }
                out.push(0);
            }
        }

        let data_size = u16::try_from(out.len() - data_start).map_err(|_| {
            PatchError::twoda(
                27,
                "2DA text data exceeds the 64 KB the format can address.",
            )
        })?;
        out[data_size_at..data_size_at + 2].copy_from_slice(&data_size.to_le_bytes());

        for r in 0..rows {
            for c in 0..columns {
                let at = offset_table_at + (r * columns + c) * 2;
                out[at..at + 2].copy_from_slice(&offsets[r][c].to_le_bytes());
            }
        }

        Ok(out)
    }

    /// Write the table to disk.
    pub fn save(&self, path: &str) -> Result<()> {
        let bytes = self.to_bytes()?;
        crate::fsutil::write_file(path, &bytes)
            .map_err(|e| PatchError::twoda(27, format!("Unable to write 2DA file {path}: {e}")))
    }
}

/// Read tab-separated labels up to the NUL that ends the column list.
fn read_labels_until_nul(reader: &mut Cursor<'_>) -> Result<Vec<String>> {
    let mut labels = Vec::new();
    let mut current: Vec<u8> = Vec::new();
    let mut guard = 0usize;

    loop {
        let Some(byte) = reader.read_u8() else {
            return Err(PatchError::twoda(
                3,
                "Malformatted data encountered while reading 2DA! Unable to continue!",
            ));
        };

        if byte == b'\t' {
            labels.push(latin1::decode(&current));
            current.clear();
        } else if byte == 0 {
            break;
        } else {
            current.push(byte);
        }

        guard += 1;
        if guard >= 4095 {
            return Err(PatchError::twoda(
                3,
                "Malformatted data encountered while reading 2DA! Unable to continue!",
            ));
        }
    }

    Ok(labels)
}

/// Read exactly `count` tab-terminated row labels.
fn read_row_labels(reader: &mut Cursor<'_>, count: usize) -> Result<Vec<String>> {
    let mut labels = Vec::with_capacity(count);
    let mut current: Vec<u8> = Vec::new();

    while labels.len() < count {
        let Some(byte) = reader.read_u8() else {
            return Err(PatchError::twoda(
                3,
                "Malformatted data encountered while reading 2DA! Unable to continue!",
            ));
        };

        if byte == b'\t' {
            labels.push(latin1::decode(&current));
            current.clear();
        } else {
            current.push(byte);
        }
    }

    Ok(labels)
}

/// Read a NUL-terminated string starting at `at`.
fn read_nul_terminated(bytes: &[u8], at: usize) -> String {
    let end = bytes[at..]
        .iter()
        .position(|&b| b == 0)
        .map(|p| at + p)
        .unwrap_or(bytes.len());
    latin1::decode(&bytes[at..end])
}

/// Minimal forward reader over a byte slice.
struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }

    fn position(&self) -> usize {
        self.at
    }

    fn skip(&mut self, count: usize) {
        self.at = (self.at + count).min(self.bytes.len());
    }

    fn take(&mut self, count: usize) -> Option<&'a [u8]> {
        if self.at + count > self.bytes.len() {
            return None;
        }
        let slice = &self.bytes[self.at..self.at + count];
        self.at += count;
        Some(slice)
    }

    fn read_u8(&mut self) -> Option<u8> {
        self.take(1).map(|b| b[0])
    }

    fn read_u16(&mut self) -> Option<u16> {
        self.take(2).map(|b| u16::from_le_bytes([b[0], b[1]]))
    }

    fn read_u32(&mut self) -> Option<u32> {
        self.take(4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
}

/// Build a table from labels and rows. Intended for tests and new tables.
pub fn build(columns: &[&str], rows: &[(&str, Vec<&str>)]) -> TwoDaFile {
    TwoDaFile {
        column_labels: columns.iter().map(|s| s.to_string()).collect(),
        row_labels: rows.iter().map(|(label, _)| label.to_string()).collect(),
        cells: rows
            .iter()
            .map(|(_, values)| values.iter().map(|v| v.to_string()).collect())
            .collect(),
        padding: [0, 0],
        path: String::new(),
        loaded: true,
    }
}

/// Value used for cells with no content.
pub fn default_cell() -> &'static str {
    DEFAULT_CELL
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> TwoDaFile {
        build(
            &["label", "cost", "icon"],
            &[
                ("0", vec!["first", "10", "i_first"]),
                ("1", vec!["second", "10", "i_second"]),
                ("2", vec!["third", "****", "i_first"]),
            ],
        )
    }

    #[test]
    fn round_trips_through_bytes() {
        let original = sample();
        let bytes = original.to_bytes().unwrap();
        let reloaded = TwoDaFile::parse(&bytes, "sample.2da").unwrap();

        assert_eq!(reloaded.row_count(), 3);
        assert_eq!(reloaded.column_count(), 3);
        assert_eq!(reloaded.cell(0, 0).unwrap(), "first");
        assert_eq!(reloaded.cell(2, 1).unwrap(), "****");
        assert_eq!(reloaded.cell(2, 2).unwrap(), "i_first");
        assert_eq!(reloaded.row_label(1).unwrap(), "1");
        assert_eq!(reloaded.column_label(2).unwrap(), "icon");
    }

    #[test]
    fn saving_twice_produces_identical_bytes() {
        let table = sample();
        let first = table.to_bytes().unwrap();
        let reloaded = TwoDaFile::parse(&first, "sample.2da").unwrap();
        let second = reloaded.to_bytes().unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn header_is_written_exactly() {
        let bytes = sample().to_bytes().unwrap();
        assert_eq!(&bytes[..8], SIGNATURE);
        assert_eq!(bytes[8], b'\n');
    }

    #[test]
    fn repeated_strings_share_storage() {
        // "10" appears in rows 0 and 1 of the same column, so the second use
        // points at the first copy.
        let table = sample();
        let bytes = table.to_bytes().unwrap();
        let text_area = &bytes[bytes.len().saturating_sub(64)..];
        let occurrences = text_area.windows(3).filter(|w| *w == b"10\0").count();
        assert_eq!(occurrences, 1);
    }

    #[test]
    fn global_dedup_reuses_string_further_right_on_earlier_row() {
        // Rectangle-only search would miss "x" on row0/col1 when writing row1/col0.
        let table = build(&["a", "b"], &[("0", vec!["y", "x"]), ("1", vec!["x", "z"])]);
        let bytes = table.to_bytes().unwrap();
        let occurrences = bytes.windows(2).filter(|w| *w == b"x\0").count();
        assert_eq!(occurrences, 1);
    }

    #[test]
    fn data_size_field_matches_pool_length() {
        let bytes = sample().to_bytes().unwrap();
        // After row labels and offset grid: uint16 data size, then pool.
        let mut i = 9usize;
        while bytes[i] != 0 {
            i += 1;
        }
        i += 1; // NUL after columns
        let rows = u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
        i += 4;
        for _ in 0..rows {
            while bytes[i] != b'\t' {
                i += 1;
            }
            i += 1;
        }
        let cols = 3usize;
        i += rows * cols * 2;
        let data_size = u16::from_le_bytes(bytes[i..i + 2].try_into().unwrap()) as usize;
        i += 2;
        assert_eq!(data_size, bytes.len() - i);
    }

    #[test]
    fn rejects_the_non_binary_variant() {
        let mut bytes = OLD_SIGNATURE.to_vec();
        bytes.push(b'\n');
        let err = TwoDaFile::parse(&bytes, "x.2da").unwrap_err();
        assert_eq!(err.code(), 1);
    }

    #[test]
    fn rejects_unknown_signatures() {
        let err = TwoDaFile::parse(b"NOTA2DA\0garbage", "x.2da").unwrap_err();
        assert_eq!(err.code(), 2);
    }

    #[test]
    fn rejects_absurd_row_counts() {
        let mut bytes = SIGNATURE.to_vec();
        bytes.push(b'\n');
        bytes.extend_from_slice(b"col\t");
        bytes.push(0);
        bytes.extend_from_slice(&1_000_000u32.to_le_bytes());
        let err = TwoDaFile::parse(&bytes, "x.2da").unwrap_err();
        assert_eq!(err.code(), 4);
    }

    #[test]
    fn adds_rows_with_default_cells() {
        let mut table = sample();
        let index = table.add_row().unwrap();
        assert_eq!(index, 3);
        assert_eq!(table.row_label(3).unwrap(), "3");
        assert_eq!(table.cell(3, 0).unwrap(), "****");
    }

    #[test]
    fn adds_columns_with_generated_labels() {
        let mut table = sample();
        let index = table.add_column().unwrap();
        assert_eq!(index, 3);
        assert_eq!(table.column_label(3).unwrap(), "Column4");
        assert_eq!(table.cell(0, 3).unwrap(), "****");
    }

    #[test]
    fn clones_rows_and_honours_a_new_label() {
        let mut table = sample();
        let index = table.clone_row(1, "custom").unwrap().unwrap();
        assert_eq!(table.row_label(index).unwrap(), "custom");
        assert_eq!(table.cell(index, 0).unwrap(), "second");

        let plain = table.clone_row(0, "").unwrap().unwrap();
        assert_eq!(table.row_label(plain).unwrap(), plain.to_string());
    }

    #[test]
    fn cloning_a_missing_row_reports_no_index() {
        let mut table = sample();
        assert!(table.clone_row(99, "").unwrap().is_none());
    }

    #[test]
    fn label_lookups_ignore_case() {
        let table = sample();
        assert_eq!(table.column_by_label("COST").unwrap(), 1);
        assert_eq!(table.row_by_label("2").unwrap(), 2);
        assert_eq!(table.column_by_label("missing").unwrap_err().code(), 8);
        assert_eq!(table.row_by_label("missing").unwrap_err().code(), 10);
    }

    #[test]
    fn high_bytes_survive_a_round_trip() {
        let accented = latin1::decode(&[0x63, 0xE9, 0x6C]);
        let table = build(&["label"], &[("0", vec![accented.as_str()])]);
        let bytes = table.to_bytes().unwrap();
        let reloaded = TwoDaFile::parse(&bytes, "x.2da").unwrap();
        assert_eq!(
            latin1::encode(reloaded.cell(0, 0).unwrap()),
            vec![0x63, 0xE9, 0x6C]
        );
    }
}
