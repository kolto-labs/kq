//! TLK — `dialog.tlk`, the table every piece of displayed text points into.
//!
//! Layout (V3.0): signature, u32 language id, u32 string count,
//! u32 string-data offset, then a 40-byte entry per string.
//!
//! The text is stored in the Windows code page of the table's language, and
//! the header's language id says which one that is (see [`language`]).
//! Translations made outside BioWare do not always set the id: the Russian
//! K2 `dialog.tlk` says English (0) but is Windows-1251. When a table that
//! claims a Windows-1252 language is mostly Cyrillic, it is read as
//! Windows-1251 and [`Tlk::encoding_note`] says so.

use std::path::Path;

use encoding_rs::{
    Encoding, BIG5, EUC_KR, GBK, SHIFT_JIS, WINDOWS_1250, WINDOWS_1251, WINDOWS_1252,
};
use kotor_formats::tlk::TlkFile;

use crate::error::Result;
use crate::shared::format_error;

#[derive(Clone, Debug)]
pub struct Entry {
    pub text: String,
    /// Voice-over resource for this line, empty when there is none.
    pub sound: String,
}

#[derive(Clone, Debug)]
pub struct Tlk {
    pub language_id: u32,
    /// Code page the text was decoded with, e.g. `windows-1251`.
    pub encoding: &'static str,
    /// Why the code page is not the one the header's language id implies.
    /// `None` when the header was followed.
    pub encoding_note: Option<String>,
    pub entries: Vec<Entry>,
}

impl Tlk {
    /// Look up a StrRef. Out-of-range and the sentinel `-1` yield `None`.
    pub fn get(&self, strref: i64) -> Option<&Entry> {
        if strref < 0 {
            return None;
        }
        self.entries.get(strref as usize)
    }
}

/// BioWare's TLK language ids: the language name and the Windows code page
/// its text is stored in. `None` for an id outside BioWare's table.
pub fn language(id: u32) -> Option<(&'static str, &'static Encoding)> {
    Some(match id {
        0 => ("English", WINDOWS_1252),
        1 => ("French", WINDOWS_1252),
        2 => ("German", WINDOWS_1252),
        3 => ("Italian", WINDOWS_1252),
        4 => ("Spanish", WINDOWS_1252),
        5 => ("Polish", WINDOWS_1250),
        // encoding_rs names these by their WHATWG labels; each decoder is the
        // Windows code page: EUC-KR is cp949, Big5 cp950, GBK cp936 and
        // Shift_JIS cp932.
        128 => ("Korean", EUC_KR),
        129 => ("Chinese (Traditional)", BIG5),
        130 => ("Chinese (Simplified)", GBK),
        131 => ("Japanese", SHIFT_JIS),
        _ => return None,
    })
}

pub fn sniff(data: &[u8]) -> bool {
    data.starts_with(b"TLK ")
}

pub fn read(data: &[u8], path: &Path) -> Result<Tlk> {
    let file =
        TlkFile::parse(data, &path.to_string_lossy()).map_err(|err| format_error(err, path))?;

    // The shared crate keeps text losslessly, one char per byte, so the bytes
    // come back exactly before they are decoded in the right code page.
    let raw: Vec<Vec<u8>> = file
        .entries()
        .iter()
        .map(|entry| entry.text.chars().map(|c| c as u32 as u8).collect())
        .collect();
    let (encoding, encoding_note) = choose_encoding(file.language_id, &raw);

    let entries = file
        .entries()
        .iter()
        .zip(&raw)
        .map(|(entry, bytes)| Entry {
            text: encoding.decode_without_bom_handling(bytes).0.into_owned(),
            // Resource names are matched case-insensitively, so they are shown
            // folded, the way every other resref in kq's output is.
            sound: entry.sound_name().trim().to_ascii_lowercase(),
        })
        .collect();

    Ok(Tlk {
        language_id: file.language_id,
        encoding: encoding.name(),
        encoding_note,
        entries,
    })
}

/// Pick the code page from the language id, unless the text contradicts it.
fn choose_encoding(language_id: u32, texts: &[Vec<u8>]) -> (&'static Encoding, Option<String>) {
    let (name, encoding) = match language(language_id) {
        Some(lang) => lang,
        None => {
            let note = format!("language id {language_id} is not in BioWare's table");
            return if mostly_cyrillic(texts) {
                (
                    WINDOWS_1251,
                    Some(format!(
                        "{note}; the text is Cyrillic, read as windows-1251"
                    )),
                )
            } else {
                (WINDOWS_1252, Some(format!("{note}; read as windows-1252")))
            };
        }
    };
    if encoding == WINDOWS_1252 && mostly_cyrillic(texts) {
        let note = format!(
            "header says language {language_id} ({name}, windows-1252) but the text is \
             Cyrillic; read as windows-1251"
        );
        return (WINDOWS_1251, Some(note));
    }
    (encoding, None)
}

/// True when most letters are in the 0xC0–0xFF range.
///
/// That is where Windows-1251 keeps the Cyrillic alphabet. Western European
/// text in Windows-1252 uses the same range only for accented letters, which
/// stay a few percent of all letters even in French or German.
fn mostly_cyrillic(texts: &[Vec<u8>]) -> bool {
    let (mut letters, mut high) = (0usize, 0usize);
    for &b in texts.iter().flatten() {
        if b.is_ascii_alphabetic() {
            letters += 1;
        } else if b >= 0xC0 {
            letters += 1;
            high += 1;
        }
    }
    letters > 0 && high * 100 / letters >= 30
}
