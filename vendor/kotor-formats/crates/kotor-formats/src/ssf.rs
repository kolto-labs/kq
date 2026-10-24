//! Soundset files (`SSF V1.1`).
//!
//! A soundset is a fixed table of 40 string references, one per character
//! vocalisation. The file is a 12-byte header — signature, version, and the
//! offset where the table starts — followed by 40 little-endian 32-bit values.
//! `0xFFFFFFFF` means "no sound assigned".

use crate::error::{PatchError, Result};

const SIGNATURE: &[u8; 4] = b"SSF ";
const VERSION: &[u8; 4] = b"V1.1";
const ENTRY_COUNT: usize = 40;
const TABLE_OFFSET: u32 = 12;

/// Entry names, in table order. Lookups by name ignore case.
pub const ENTRY_LABELS: [&str; ENTRY_COUNT] = [
    "Battlecry 1",
    "Battlecry 2",
    "Battlecry 3",
    "Battlecry 4",
    "Battlecry 5",
    "Battlecry 6",
    "Selected 1",
    "Selected 2",
    "Selected 3",
    "Attack 1",
    "Attack 2",
    "Attack 3",
    "Pain 1",
    "Pain 2",
    "Low health",
    "Death",
    "Critical hit",
    "Target immune",
    "Place mine",
    "Disarm mine",
    "Stealth on",
    "Search",
    "Pick lock start",
    "Pick lock fail",
    "Pick lock done",
    "Leave party",
    "Rejoin party",
    "Poisoned",
    "Unknown(29)",
    "Unknown(30)",
    "Unknown(31)",
    "Unknown(32)",
    "Unknown(33)",
    "Unknown(34)",
    "Unknown(35)",
    "Unknown(36)",
    "Unknown(37)",
    "Unknown(38)",
    "Unknown(39)",
    "Unknown(40)",
];

/// A loaded soundset.
#[derive(Debug, Clone)]
pub struct SsfFile {
    entries: [u32; ENTRY_COUNT],
    path: String,
    loaded: bool,
}

impl Default for SsfFile {
    fn default() -> Self {
        Self {
            entries: [u32::MAX; ENTRY_COUNT],
            path: String::new(),
            loaded: false,
        }
    }
}

impl SsfFile {
    /// An empty soundset with every entry unassigned.
    pub fn new() -> Self {
        Self::default()
    }

    /// True once a soundset has been read successfully.
    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    /// Path the soundset was read from.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// All 40 entries in table order.
    pub fn entries(&self) -> &[u32; ENTRY_COUNT] {
        &self.entries
    }

    /// Value of the entry with this label.
    pub fn entry(&self, label: &str) -> Result<u32> {
        let index = index_of(label).ok_or_else(|| {
            PatchError::ssf(
                3,
                format!("Unable to read value in SSF file, label \"{label}\" is not a valid entry label!"),
            )
        })?;
        Ok(self.entries[index])
    }

    /// Assign a string reference to the entry with this label.
    pub fn set_entry(&mut self, label: &str, value: u32) -> Result<()> {
        let index = index_of(label).ok_or_else(|| {
            PatchError::ssf(
                2,
                format!("Unable to change value in SSF file, label \"{label}\" is not a valid entry label!"),
            )
        })?;
        self.entries[index] = value;
        Ok(())
    }

    /// Parse soundset bytes.
    pub fn parse(bytes: &[u8], path: &str) -> Result<Self> {
        if bytes.len() < 12 {
            return Err(PatchError::ssf(
                1,
                format!("Selected file \"{path}\" is not a valid SSF v1.1 file!"),
            ));
        }

        if &bytes[0..4] != SIGNATURE || &bytes[4..8] != VERSION {
            return Err(PatchError::ssf(
                1,
                format!("Selected file \"{path}\" is not a valid SSF v1.1 file!"),
            ));
        }

        let table_at = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
        let mut entries = [u32::MAX; ENTRY_COUNT];

        for (i, slot) in entries.iter_mut().enumerate() {
            let at = table_at + i * 4;
            if at + 4 > bytes.len() {
                // Short files leave the remaining entries unassigned, which is
                // how a partially written soundset was tolerated before.
                break;
            }
            *slot = u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
        }

        Ok(Self {
            entries,
            path: path.to_string(),
            loaded: true,
        })
    }

    /// Read a soundset from disk.
    pub fn load(path: &str) -> Result<Self> {
        if !crate::fsutil::file_exists(path) {
            return Err(PatchError::ssf(
                5,
                format!("Selected file \"{path}\" does not exist! Unable to load it."),
            ));
        }

        let bytes = crate::fsutil::read_file(path)
            .map_err(|e| PatchError::ssf(5, format!("Unable to read SSF file {path}: {e}")))?;
        Self::parse(&bytes, path)
    }

    /// Serialize the soundset back to bytes.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        if !self.loaded {
            return Err(PatchError::ssf(
                6,
                "No file has been loaded! Unable to save.",
            ));
        }

        let mut out = Vec::with_capacity(12 + ENTRY_COUNT * 4);
        out.extend_from_slice(SIGNATURE);
        out.extend_from_slice(VERSION);
        out.extend_from_slice(&TABLE_OFFSET.to_le_bytes());

        for value in self.entries {
            out.extend_from_slice(&value.to_le_bytes());
        }

        Ok(out)
    }

    /// Write the soundset to disk, to its own path unless another is given.
    pub fn save(&mut self, path: Option<&str>) -> Result<()> {
        if let Some(target) = path {
            self.path = target.to_string();
        }

        let bytes = self.to_bytes()?;
        let target = self.path.clone();
        crate::fsutil::write_file(&target, &bytes)
            .map_err(|e| PatchError::ssf(6, format!("Unable to write SSF file {target}: {e}")))
    }
}

/// Table position of an entry label, ignoring case.
fn index_of(label: &str) -> Option<usize> {
    ENTRY_LABELS
        .iter()
        .position(|candidate| candidate.eq_ignore_ascii_case(label))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loaded() -> SsfFile {
        let mut file = SsfFile::new();
        file.loaded = true;
        file.path = "sound.ssf".to_string();
        file
    }

    #[test]
    fn new_soundsets_start_unassigned() {
        let file = SsfFile::new();
        assert!(file.entries().iter().all(|&v| v == u32::MAX));
    }

    #[test]
    fn round_trips_through_bytes() {
        let mut file = loaded();
        file.set_entry("Battlecry 1", 1000).unwrap();
        file.set_entry("Death", 2000).unwrap();

        let bytes = file.to_bytes().unwrap();
        assert_eq!(bytes.len(), 12 + 160);

        let reloaded = SsfFile::parse(&bytes, "sound.ssf").unwrap();
        assert_eq!(reloaded.entry("Battlecry 1").unwrap(), 1000);
        assert_eq!(reloaded.entry("Death").unwrap(), 2000);
        assert_eq!(reloaded.entry("Search").unwrap(), u32::MAX);
    }

    #[test]
    fn header_is_written_exactly() {
        let bytes = loaded().to_bytes().unwrap();
        assert_eq!(&bytes[0..4], b"SSF ");
        assert_eq!(&bytes[4..8], b"V1.1");
        assert_eq!(&bytes[8..12], &12u32.to_le_bytes());
    }

    #[test]
    fn entry_labels_ignore_case() {
        let mut file = loaded();
        file.set_entry("low health", 42).unwrap();
        assert_eq!(file.entry("LOW HEALTH").unwrap(), 42);
    }

    #[test]
    fn unknown_labels_are_rejected() {
        let mut file = loaded();
        assert_eq!(file.set_entry("Nope", 1).unwrap_err().code(), 2);
        assert_eq!(file.entry("Nope").unwrap_err().code(), 3);
    }

    #[test]
    fn rejects_wrong_signature_and_version() {
        let mut bytes = b"XXX V1.1".to_vec();
        bytes.extend_from_slice(&12u32.to_le_bytes());
        assert_eq!(SsfFile::parse(&bytes, "x.ssf").unwrap_err().code(), 1);

        let mut wrong_version = b"SSF V2.0".to_vec();
        wrong_version.extend_from_slice(&12u32.to_le_bytes());
        assert_eq!(
            SsfFile::parse(&wrong_version, "x.ssf").unwrap_err().code(),
            1
        );
    }

    #[test]
    fn honours_a_non_standard_table_offset() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"SSF V1.1");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&[0xAA; 4]); // filler before the table
        for i in 0..ENTRY_COUNT {
            bytes.extend_from_slice(&(i as u32).to_le_bytes());
        }

        let file = SsfFile::parse(&bytes, "x.ssf").unwrap();
        assert_eq!(file.entry("Battlecry 1").unwrap(), 0);
        assert_eq!(file.entry("Unknown(40)").unwrap(), 39);
    }

    #[test]
    fn unloaded_soundsets_refuse_to_save() {
        assert_eq!(SsfFile::new().to_bytes().unwrap_err().code(), 6);
    }
}
