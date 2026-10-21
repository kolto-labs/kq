//! String table files (`TLK V3.0`).
//!
//! A string table holds every line of localized text the game displays. The
//! file is a 20-byte header, a fixed-size record per entry describing flags,
//! sound, and where the text lives, then one contiguous block of text.
//!
//! Entries are addressed by position, so appending is safe but removing an
//! entry from the middle renumbers everything after it.

use crate::error::{PatchError, Result};
use crate::latin1;

const SIGNATURE: &[u8; 4] = b"TLK ";
const VERSION: &[u8; 4] = b"V3.0";
const HEADER_SIZE: usize = 20;
const RECORD_SIZE: usize = 40;

/// Text buffer size used when reading and writing entry text.
///
/// Text longer than this is handled in successive chunks; the chunking is
/// visible in behavior because each chunk stops at an embedded NUL.
const TEXT_CHUNK: usize = 4096;

/// Set when the entry carries text.
pub const FLAG_TEXT_PRESENT: u32 = 0x0001;
/// Set when the entry carries a sound reference.
pub const FLAG_SOUND_PRESENT: u32 = 0x0002;
/// Set when the entry carries a sound duration.
pub const FLAG_SOUND_LENGTH_PRESENT: u32 = 0x0004;

/// One entry in a string table.
#[derive(Debug, Clone, PartialEq)]
pub struct TlkEntry {
    /// Bit flags describing which fields carry data.
    pub flags: u32,
    /// Sound resource name, stored as a fixed 16-byte field.
    pub sound_resref: [u8; 16],
    /// Volume variance for the associated sound.
    pub volume_variance: u32,
    /// Pitch variance for the associated sound.
    pub pitch_variance: u32,
    /// Duration of the associated sound, in seconds.
    pub sound_length: f32,
    /// Byte length recorded in the entry's record.
    ///
    /// Kept separate from the text because the record is written back as read,
    /// and a file whose text contains an embedded NUL would otherwise change.
    pub size: u32,
    /// The entry's text.
    pub text: String,
    /// Position of this entry in the table.
    pub strref: u32,
}

impl Default for TlkEntry {
    fn default() -> Self {
        Self {
            flags: 0,
            sound_resref: [0u8; 16],
            volume_variance: 0,
            pitch_variance: 0,
            sound_length: 0.0,
            size: 0,
            text: String::new(),
            strref: 0,
        }
    }
}

impl TlkEntry {
    /// True when two entries carry the same content.
    ///
    /// Position is deliberately excluded: this answers "does an identical line
    /// already exist elsewhere in the table", which is how appending avoids
    /// creating duplicates.
    pub fn same_content(&self, other: &TlkEntry) -> bool {
        self.text == other.text
            && self.flags == other.flags
            && self.sound_resref == other.sound_resref
            && self.volume_variance == other.volume_variance
            && self.pitch_variance == other.pitch_variance
            && self.sound_length.to_bits() == other.sound_length.to_bits()
    }

    /// Sound resource name with padding removed.
    pub fn sound_name(&self) -> String {
        self.sound_resref
            .iter()
            .filter(|&&b| b != 0)
            .map(|&b| b as char)
            .collect()
    }
}

/// A loaded string table.
#[derive(Debug, Clone, Default)]
pub struct TlkFile {
    /// Language this table is written in.
    pub language_id: u32,
    entries: Vec<TlkEntry>,
    path: String,
    loaded: bool,
    modified: bool,
}

impl TlkFile {
    /// An empty, unloaded table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Start a blank table that can be appended to and saved.
    pub fn new_file() -> Self {
        Self {
            language_id: 0,
            entries: Vec::new(),
            path: "untitled.tlk".to_string(),
            loaded: true,
            modified: true,
        }
    }

    /// True once a table has been read successfully.
    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    /// True when entries have been added since loading.
    pub fn is_modified(&self) -> bool {
        self.modified
    }

    /// Path the table was read from.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Number of entries.
    pub fn count(&self) -> usize {
        self.entries.len()
    }

    /// All entries, in table order.
    pub fn entries(&self) -> &[TlkEntry] {
        &self.entries
    }

    /// One entry by position.
    pub fn entry(&self, strref: usize) -> Option<&TlkEntry> {
        self.entries.get(strref)
    }

    /// Append an entry and return the position it landed at.
    ///
    /// The recorded byte length is recalculated from the text, and the entry's
    /// position is set to the end of the table.
    pub fn add_entry(&mut self, mut entry: TlkEntry) -> Result<u32> {
        if !self.loaded {
            return Err(PatchError::tlk(
                "Unable to add new entry. No TLK file is open!",
            ));
        }

        let strref = self.entries.len() as u32;
        entry.size = latin1::byte_len(&entry.text) as u32;
        entry.strref = strref;

        self.entries.push(entry);
        self.modified = true;

        Ok(strref)
    }

    /// Replace an existing entry's content in place, keeping its position.
    ///
    /// Used by `[TLKList]` `Replace*=file` edits (HoloPatcher `Replace0=` /
    /// `ReplaceFile0=`), which overwrite an existing
    /// line in the game's table with one copied from a mod-supplied file. The
    /// recorded byte length is recalculated and the entry keeps its original
    /// position, so nothing after it is renumbered.
    pub fn replace_entry(&mut self, index: usize, mut entry: TlkEntry) -> Result<()> {
        if !self.loaded {
            return Err(PatchError::tlk(
                "Unable to replace entry. No TLK file is open!",
            ));
        }

        let slot = self
            .entries
            .get_mut(index)
            .ok_or_else(|| PatchError::tlk(format!("No entry at position {index} to replace.")))?;

        entry.size = latin1::byte_len(&entry.text) as u32;
        entry.strref = index as u32;
        *slot = entry;
        self.modified = true;

        Ok(())
    }

    /// Parse string table bytes.
    pub fn parse(bytes: &[u8], path: &str) -> Result<Self> {
        if bytes.len() < HEADER_SIZE {
            return Err(PatchError::tlk(
                "Type mismatch. Specified file is not a valid TLK file!",
            ));
        }

        if &bytes[0..4] != SIGNATURE {
            return Err(PatchError::tlk(
                "Type mismatch. Specified file is not a valid TLK file!",
            ));
        }
        if &bytes[4..8] != VERSION {
            return Err(PatchError::tlk(
                "Version mismatch. File is not a valid v3.0 TLK file!",
            ));
        }

        let language_id = read_u32(bytes, 8);
        let count = read_u32(bytes, 12) as usize;
        let text_base = read_u32(bytes, 16) as usize;

        if count < 1 {
            return Err(PatchError::tlk(
                "No entries found in the specified TLK file!",
            ));
        }

        let mut entries = Vec::with_capacity(count);

        for index in 0..count {
            let at = HEADER_SIZE + index * RECORD_SIZE;
            if at + RECORD_SIZE > bytes.len() || at >= text_base {
                return Err(PatchError::tlk(
                    "Error reading string data table. Overflow into entry data table!",
                ));
            }

            let mut sound_resref = [0u8; 16];
            sound_resref.copy_from_slice(&bytes[at + 4..at + 20]);

            let text_offset = read_u32(bytes, at + 28) as usize;
            let size = read_u32(bytes, at + 32);

            entries.push(TlkEntry {
                flags: read_u32(bytes, at),
                sound_resref,
                volume_variance: read_u32(bytes, at + 20),
                pitch_variance: read_u32(bytes, at + 24),
                sound_length: f32::from_bits(read_u32(bytes, at + 36)),
                size,
                text: read_entry_text(bytes, text_base + text_offset, size as usize),
                strref: index as u32,
            });
        }

        Ok(Self {
            language_id,
            entries,
            path: path.to_string(),
            loaded: true,
            modified: false,
        })
    }

    /// Read a string table from disk.
    pub fn load(path: &str) -> Result<Self> {
        if !crate::fsutil::file_exists(path) {
            return Err(PatchError::tlk(
                "Unable to load specified TLK file since it could not be found!",
            ));
        }

        let bytes = crate::fsutil::read_file(path)
            .map_err(|e| PatchError::tlk(format!("Unable to read TLK file {path}: {e}")))?;
        Self::parse(&bytes, path)
    }

    /// Serialize the string table back to bytes.
    ///
    /// Each record keeps the byte length it was loaded with, while the text
    /// block is written from the current text. Entries with no text record an
    /// offset of zero.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        if !self.loaded {
            return Err(PatchError::tlk("There is no open file to save!"));
        }
        if self.entries.is_empty() {
            return Err(PatchError::tlk(
                "There is no current data in the TLK file to write!",
            ));
        }

        let count = self.entries.len();
        let text_base = HEADER_SIZE + count * RECORD_SIZE;

        let mut out = Vec::with_capacity(text_base + count * 32);
        out.extend_from_slice(SIGNATURE);
        out.extend_from_slice(VERSION);
        out.extend_from_slice(&self.language_id.to_le_bytes());
        out.extend_from_slice(&(count as u32).to_le_bytes());
        out.extend_from_slice(&(text_base as u32).to_le_bytes());

        // Records first, with text offsets filled in as the text block is built.
        for entry in &self.entries {
            out.extend_from_slice(&entry.flags.to_le_bytes());
            out.extend_from_slice(&entry.sound_resref);
            out.extend_from_slice(&entry.volume_variance.to_le_bytes());
            out.extend_from_slice(&entry.pitch_variance.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes()); // text offset, set below
            out.extend_from_slice(&entry.size.to_le_bytes());
            out.extend_from_slice(&entry.sound_length.to_bits().to_le_bytes());
        }

        debug_assert_eq!(out.len(), text_base);

        for (index, entry) in self.entries.iter().enumerate() {
            let encoded = latin1::encode(&entry.text);
            let offset = if encoded.is_empty() {
                0
            } else {
                (out.len() - text_base) as u32
            };

            let offset_at = HEADER_SIZE + index * RECORD_SIZE + 28;
            out[offset_at..offset_at + 4].copy_from_slice(&offset.to_le_bytes());

            out.extend_from_slice(&encoded);
        }

        Ok(out)
    }

    /// Write the string table to disk, to its own path unless another is given.
    pub fn save(&mut self, path: Option<&str>) -> Result<()> {
        if let Some(target) = path {
            self.path = target.to_string();
        }

        let bytes = self.to_bytes()?;
        let target = self.path.clone();
        crate::fsutil::write_file(&target, &bytes)
            .map_err(|e| PatchError::tlk(format!("Unable to write TLK file {target}: {e}")))?;

        self.modified = false;
        Ok(())
    }
}

/// Read entry text of `size` bytes starting at `at`.
///
/// Text is consumed in fixed-size chunks and each chunk stops at an embedded
/// NUL, so a NUL inside one chunk truncates that chunk without discarding the
/// chunks that follow.
fn read_entry_text(bytes: &[u8], at: usize, size: usize) -> String {
    if size == 0 || at >= bytes.len() {
        return String::new();
    }

    let available = bytes.len() - at;
    let mut remaining = size.min(available);
    let mut cursor = at;
    let mut text = String::new();

    while remaining > 0 {
        let chunk = remaining.min(TEXT_CHUNK);
        let slice = &bytes[cursor..cursor + chunk];
        text.push_str(&latin1::decode_nul_terminated(slice));
        cursor += chunk;
        remaining -= chunk;
    }

    text
}

fn read_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(text: &str) -> TlkEntry {
        TlkEntry {
            flags: FLAG_TEXT_PRESENT,
            text: text.to_string(),
            size: latin1::byte_len(text) as u32,
            ..TlkEntry::default()
        }
    }

    fn table(texts: &[&str]) -> TlkFile {
        let mut file = TlkFile::new_file();
        for text in texts {
            file.add_entry(entry(text)).unwrap();
        }
        file
    }

    #[test]
    fn round_trips_through_bytes() {
        let original = table(&["Hello there.", "", "General Kenobi."]);
        let bytes = original.to_bytes().unwrap();
        let reloaded = TlkFile::parse(&bytes, "dialog.tlk").unwrap();

        assert_eq!(reloaded.count(), 3);
        assert_eq!(reloaded.entry(0).unwrap().text, "Hello there.");
        assert_eq!(reloaded.entry(1).unwrap().text, "");
        assert_eq!(reloaded.entry(2).unwrap().text, "General Kenobi.");
    }

    #[test]
    fn header_is_written_exactly() {
        let bytes = table(&["a"]).to_bytes().unwrap();
        assert_eq!(&bytes[0..4], b"TLK ");
        assert_eq!(&bytes[4..8], b"V3.0");
        assert_eq!(read_u32(&bytes, 12), 1);
        assert_eq!(read_u32(&bytes, 16), (HEADER_SIZE + RECORD_SIZE) as u32);
    }

    #[test]
    fn saving_twice_produces_identical_bytes() {
        let first = table(&["one", "two"]).to_bytes().unwrap();
        let reloaded = TlkFile::parse(&first, "dialog.tlk").unwrap();
        assert_eq!(first, reloaded.to_bytes().unwrap());
    }

    #[test]
    fn appending_assigns_the_next_position() {
        let mut file = table(&["first"]);
        let strref = file.add_entry(entry("second")).unwrap();
        assert_eq!(strref, 1);
        assert_eq!(file.entry(1).unwrap().strref, 1);
        assert_eq!(file.count(), 2);
        assert!(file.is_modified());
    }

    #[test]
    fn replacing_overwrites_content_but_keeps_position() {
        let mut file = table(&["first", "second", "third"]);
        file.replace_entry(1, entry("brand new line")).unwrap();

        // The replaced slot carries the new text but keeps its position, and
        // nothing after it is renumbered.
        assert_eq!(file.count(), 3);
        assert_eq!(file.entry(1).unwrap().text, "brand new line");
        assert_eq!(file.entry(1).unwrap().strref, 1);
        assert_eq!(
            file.entry(1).unwrap().size,
            latin1::byte_len("brand new line") as u32
        );
        assert_eq!(file.entry(0).unwrap().text, "first");
        assert_eq!(file.entry(2).unwrap().text, "third");
    }

    #[test]
    fn replacing_a_missing_position_fails() {
        let mut file = table(&["only"]);
        assert!(file.replace_entry(5, entry("nope")).is_err());
    }

    #[test]
    fn appending_recalculates_the_recorded_length() {
        let mut file = TlkFile::new_file();
        let mut sloppy = entry("exact text");
        sloppy.size = 999;
        file.add_entry(sloppy).unwrap();
        assert_eq!(file.entry(0).unwrap().size, 10);
    }

    #[test]
    fn empty_text_records_a_zero_offset() {
        let bytes = table(&["", "after"]).to_bytes().unwrap();
        assert_eq!(read_u32(&bytes, HEADER_SIZE + 28), 0);
    }

    #[test]
    fn content_comparison_ignores_position() {
        let mut a = entry("same");
        let mut b = entry("same");
        a.strref = 5;
        b.strref = 900;
        assert!(a.same_content(&b));

        b.text = "different".to_string();
        assert!(!a.same_content(&b));
    }

    #[test]
    fn content_comparison_checks_sound_fields() {
        let a = entry("same");
        let mut b = entry("same");
        b.volume_variance = 3;
        assert!(!a.same_content(&b));

        let mut c = entry("same");
        c.sound_resref[0] = b'x';
        assert!(!a.same_content(&c));
    }

    #[test]
    fn sound_names_drop_padding() {
        let mut e = entry("x");
        e.sound_resref[..5].copy_from_slice(b"vo_01");
        assert_eq!(e.sound_name(), "vo_01");
    }

    #[test]
    fn rejects_wrong_signature_and_version() {
        let mut wrong_type = b"XXX V3.0".to_vec();
        wrong_type.extend_from_slice(&[0u8; 12]);
        assert!(TlkFile::parse(&wrong_type, "x.tlk").is_err());

        let mut wrong_version = b"TLK V4.0".to_vec();
        wrong_version.extend_from_slice(&[0u8; 12]);
        assert!(TlkFile::parse(&wrong_version, "x.tlk").is_err());
    }

    #[test]
    fn rejects_empty_tables() {
        let mut bytes = b"TLK V3.0".to_vec();
        bytes.extend_from_slice(&0u32.to_le_bytes()); // language
        bytes.extend_from_slice(&0u32.to_le_bytes()); // count
        bytes.extend_from_slice(&20u32.to_le_bytes()); // text base
        assert!(TlkFile::parse(&bytes, "x.tlk").is_err());
    }

    #[test]
    fn text_longer_than_one_chunk_survives() {
        let long = "z".repeat(TEXT_CHUNK * 2 + 17);
        let bytes = table(&[long.as_str()]).to_bytes().unwrap();
        let reloaded = TlkFile::parse(&bytes, "x.tlk").unwrap();
        assert_eq!(reloaded.entry(0).unwrap().text.len(), long.len());
        assert_eq!(reloaded.entry(0).unwrap().text, long);
    }

    #[test]
    fn high_bytes_survive_a_round_trip() {
        let accented = latin1::decode(&[0x48, 0xE9, 0x21]);
        let bytes = table(&[accented.as_str()]).to_bytes().unwrap();
        let reloaded = TlkFile::parse(&bytes, "x.tlk").unwrap();
        assert_eq!(
            latin1::encode(&reloaded.entry(0).unwrap().text),
            vec![0x48, 0xE9, 0x21]
        );
    }

    #[test]
    fn unloaded_tables_refuse_to_save() {
        assert!(TlkFile::new().to_bytes().is_err());
    }
}
