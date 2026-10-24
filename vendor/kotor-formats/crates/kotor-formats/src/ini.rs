//! Instruction file reader and writer.
//!
//! Mods ship their instructions as INI files that TSLPatcher read through the
//! Windows profile API. That API has specific habits — it trims surrounding
//! whitespace, discards a matching pair of quotes, treats names
//! case-insensitively, and reports an empty section as absent. All of those are
//! reproduced here, along with the `<#LF#>` and `<#CR#>` tokens that let a value
//! span several lines.
//!
//! Reading alone is not enough for an editor, so this module also writes. The
//! document keeps every line it parsed, in order, including comments and blank
//! lines. A line nobody edited is written back exactly as it arrived, down to
//! its spacing and its line ending. Only the lines an edit touched are rebuilt.
//! Mod authors keep their files in version control; a save that reflowed the
//! whole file would bury one real change in a thousand cosmetic ones.

use crate::{latin1, text};

/// One line of the file, in the form it was read.
#[derive(Debug, Clone)]
enum Line {
    /// A `[name]` header.
    Section {
        /// The line as it was read, if untouched.
        raw: Option<String>,
        /// The name between the brackets.
        name: String,
    },
    /// A `key=value` pair.
    Entry {
        /// The line as it was read, if untouched.
        raw: Option<String>,
        /// The key, trimmed, in the spelling the file used.
        key: String,
        /// The value as stored in the file: quotes already dropped, newline
        /// tokens still in their escaped form.
        value: String,
    },
    /// A comment, a blank line, or anything without a separator. Never edited,
    /// always written back as it came.
    Verbatim(String),
}

/// A parsed instruction file.
#[derive(Debug, Clone)]
pub struct IniFile {
    lines: Vec<Line>,
    /// Line ending to give newly written lines.
    eol: String,
    /// Whether the file ended with a line break.
    trailing_newline: bool,
}

impl Default for IniFile {
    fn default() -> Self {
        Self {
            lines: Vec::new(),
            eol: default_eol().to_string(),
            trailing_newline: true,
        }
    }
}

/// Line ending for a file that does not yet have one to copy.
///
/// TSLPatcher is a Windows tool and its INI files are read on Windows, so new
/// files get CRLF whatever platform wrote them.
fn default_eol() -> &'static str {
    "\r\n"
}

impl IniFile {
    /// An empty document.
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse instruction text that has already been read into memory.
    pub fn parse(bytes: &[u8]) -> Self {
        // A byte-order mark would otherwise become part of the first section name.
        let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
        let content = latin1::decode(body);

        let eol = if content.contains("\r\n") {
            "\r\n".to_string()
        } else if content.contains('\n') {
            "\n".to_string()
        } else {
            default_eol().to_string()
        };
        let trailing_newline = content.is_empty() || content.ends_with('\n');

        let mut lines = Vec::new();
        for raw_line in split_lines(&content) {
            lines.push(classify(raw_line));
        }

        Self {
            lines,
            eol,
            trailing_newline,
        }
    }

    /// Read an instruction file from disk. A missing file parses as empty,
    /// which lets callers fall back to their defaults.
    pub fn load(path: &str) -> Self {
        match std::fs::read(path) {
            Ok(bytes) => Self::parse(&bytes),
            Err(_) => Self::default(),
        }
    }

    /// Render the document back to bytes.
    ///
    /// Lines nobody edited come back byte-for-byte; edited and inserted lines
    /// are built fresh using the file's own line ending.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = String::new();
        for (index, line) in self.lines.iter().enumerate() {
            out.push_str(&render(line));
            let last = index + 1 == self.lines.len();
            if !last || self.trailing_newline {
                out.push_str(&self.eol);
            }
        }
        latin1::encode(&out)
    }

    /// Write the document to disk.
    pub fn save(&self, path: &str) -> std::io::Result<()> {
        std::fs::write(path, self.to_bytes())
    }

    // -- reading ---------------------------------------------------------

    /// Index of the header line opening `name`, if the section is present.
    fn section_start(&self, name: &str) -> Option<usize> {
        self.lines.iter().position(|line| match line {
            Line::Section { name: found, .. } => found.eq_ignore_ascii_case(name),
            _ => false,
        })
    }

    /// Range of line indices belonging to a section, header excluded.
    fn section_body(&self, name: &str) -> Option<(usize, usize)> {
        let start = self.section_start(name)?;
        let mut end = self.lines.len();
        for (offset, line) in self.lines.iter().enumerate().skip(start + 1) {
            if matches!(line, Line::Section { .. }) {
                end = offset;
                break;
            }
        }
        Some((start + 1, end))
    }

    /// Every entry in a section, in file order.
    fn entries(&self, section: &str) -> Vec<(usize, &str, &str)> {
        let Some((start, end)) = self.section_body(section) else {
            return Vec::new();
        };
        self.lines[start..end]
            .iter()
            .enumerate()
            .filter_map(|(offset, line)| match line {
                Line::Entry { key, value, .. } => {
                    Some((start + offset, key.as_str(), value.as_str()))
                }
                _ => None,
            })
            .collect()
    }

    /// True when the section exists *and* holds at least one entry.
    ///
    /// An empty section counts as absent, matching the profile API. Several
    /// patch steps depend on this to decide whether a file has instructions.
    pub fn section_exists(&self, section: &str) -> bool {
        !self.entries(section).is_empty()
    }

    /// True when the section header is in the file, even with nothing under it.
    ///
    /// The editor needs this where [`section_exists`](Self::section_exists) is
    /// wrong: a section the author just created is real and must not be added
    /// twice, though a patcher reading the file would not yet see it.
    pub fn section_present(&self, section: &str) -> bool {
        self.section_start(section).is_some()
    }

    /// Key names in a section, in the order they appear in the file.
    ///
    /// Order matters: patch steps apply modifiers top to bottom, and some
    /// modifiers only take effect once an earlier key has run.
    pub fn read_section(&self, section: &str) -> Vec<String> {
        self.entries(section)
            .into_iter()
            .map(|(_, key, _)| key.to_string())
            .collect()
    }

    /// Every key and value in a section, in file order.
    pub fn read_section_values(&self, section: &str) -> Vec<(String, String)> {
        self.entries(section)
            .into_iter()
            .map(|(_, key, value)| (key.to_string(), expand_newline_tokens(value)))
            .collect()
    }

    /// Value for a key, or `default` when the key is absent.
    ///
    /// Duplicate keys resolve to the first one, and newline tokens are expanded.
    pub fn read_string(&self, section: &str, key: &str, default: &str) -> String {
        self.entries(section)
            .into_iter()
            .find(|(_, found, _)| found.eq_ignore_ascii_case(key))
            .map(|(_, _, value)| expand_newline_tokens(value))
            .unwrap_or_else(|| default.to_string())
    }

    /// True when the section lists this key, even if the value is empty.
    pub fn has_key(&self, section: &str, key: &str) -> bool {
        self.entries(section)
            .into_iter()
            .any(|(_, found, _)| found.eq_ignore_ascii_case(key))
    }

    /// Value for a key parsed as a whole number.
    ///
    /// A `0x` prefix is read as hexadecimal. Anything unparseable yields
    /// `default` rather than failing the operation.
    pub fn read_integer(&self, section: &str, key: &str, default: i32) -> i32 {
        let value = self.read_string(section, key, "");
        parse_integer(&value).unwrap_or(default)
    }

    /// Value for a key read as a flag. Any non-zero number is true.
    pub fn read_bool(&self, section: &str, key: &str, default: bool) -> bool {
        self.read_integer(section, key, i32::from(default)) != 0
    }

    /// Names of every section, in file order.
    pub fn section_names(&self) -> Vec<String> {
        self.lines
            .iter()
            .filter_map(|line| match line {
                Line::Section { name, .. } => Some(name.clone()),
                _ => None,
            })
            .collect()
    }

    // -- writing ---------------------------------------------------------

    /// Add a section if it is not already there, and return where its header sits.
    pub fn add_section(&mut self, section: &str) -> usize {
        if let Some(at) = self.section_start(section) {
            return at;
        }
        // A blank line before the header keeps the file readable, but not at the
        // very top and not where one already separates the sections.
        if !self.lines.is_empty()
            && !matches!(self.lines.last(), Some(Line::Verbatim(v)) if v.trim().is_empty())
        {
            self.lines.push(Line::Verbatim(String::new()));
        }
        self.lines.push(Line::Section {
            raw: None,
            name: section.to_string(),
        });
        self.lines.len() - 1
    }

    /// Set a key's value, adding the key or the whole section if needed.
    ///
    /// An existing key is updated where it sits, so the order of the modifiers
    /// around it does not shift. A new key goes at the end of its section,
    /// before the blank lines that trail it.
    pub fn write_string(&mut self, section: &str, key: &str, value: &str) {
        let stored = collapse_newline_tokens(value);

        if let Some((start, end)) = self.section_body(section) {
            for index in start..end {
                if let Line::Entry {
                    key: found,
                    value: slot,
                    raw,
                } = &mut self.lines[index]
                {
                    if found.eq_ignore_ascii_case(key) {
                        if *slot != stored {
                            *slot = stored;
                            *raw = None;
                        }
                        return;
                    }
                }
            }
            let at = self.insert_point(start, end);
            self.lines.insert(
                at,
                Line::Entry {
                    raw: None,
                    key: key.to_string(),
                    value: stored,
                },
            );
            return;
        }

        self.add_section(section);
        self.lines.push(Line::Entry {
            raw: None,
            key: key.to_string(),
            value: stored,
        });
    }

    /// Set a key to a whole number.
    pub fn write_integer(&mut self, section: &str, key: &str, value: i32) {
        self.write_string(section, key, &value.to_string());
    }

    /// Set a key to a flag, written as `1` or `0` the way TSLPatcher expects.
    pub fn write_bool(&mut self, section: &str, key: &str, value: bool) {
        self.write_integer(section, key, i32::from(value));
    }

    /// Where a new entry should go: after the last entry in the section, so it
    /// lands above any trailing blank lines rather than after them.
    fn insert_point(&self, start: usize, end: usize) -> usize {
        let mut at = start;
        for index in start..end {
            if matches!(self.lines[index], Line::Entry { .. }) {
                at = index + 1;
            }
        }
        if at == start {
            // No entries yet — go directly under the header.
            start
        } else {
            at
        }
    }

    /// Remove a key. Every occurrence goes, so a duplicate cannot resurface.
    pub fn delete_key(&mut self, section: &str, key: &str) {
        let Some((start, end)) = self.section_body(section) else {
            return;
        };
        let doomed: Vec<usize> = (start..end)
            .filter(|&index| match &self.lines[index] {
                Line::Entry { key: found, .. } => found.eq_ignore_ascii_case(key),
                _ => false,
            })
            .collect();
        for index in doomed.into_iter().rev() {
            self.lines.remove(index);
        }
    }

    /// Remove a section and everything under it.
    pub fn delete_section(&mut self, section: &str) {
        let Some(start) = self.section_start(section) else {
            return;
        };
        let (_, end) = self.section_body(section).unwrap_or((start + 1, start + 1));
        self.lines.drain(start..end);
        // Drop a blank line left stranded where the section used to be.
        if start > 0 {
            if let (Some(Line::Verbatim(before)), true) =
                (self.lines.get(start - 1), start < self.lines.len())
            {
                if before.trim().is_empty()
                    && matches!(self.lines.get(start), Some(Line::Verbatim(v)) if v.trim().is_empty())
                {
                    self.lines.remove(start);
                }
            }
        }
    }

    /// Rename a section, keeping its position and its entries.
    pub fn rename_section(&mut self, from: &str, to: &str) {
        if let Some(at) = self.section_start(from) {
            if let Line::Section { raw, name } = &mut self.lines[at] {
                *name = to.to_string();
                *raw = None;
            }
        }
    }

    /// Rename a key, keeping its position and its value.
    pub fn rename_key(&mut self, section: &str, from: &str, to: &str) {
        let Some((start, end)) = self.section_body(section) else {
            return;
        };
        for index in start..end {
            if let Line::Entry { key, raw, .. } = &mut self.lines[index] {
                if key.eq_ignore_ascii_case(from) {
                    *key = to.to_string();
                    *raw = None;
                    return;
                }
            }
        }
    }

    /// Replace a section's entries with exactly these pairs, in this order.
    ///
    /// Comments and blank lines inside the section stay where they are. This is
    /// how a panel writes back a list it has renumbered — `File0`, `File1` and
    /// so on have to close up after a delete, and rewriting them one key at a
    /// time would leave the old tail behind.
    pub fn replace_section_values(&mut self, section: &str, pairs: &[(String, String)]) {
        self.add_section(section);
        let Some((start, end)) = self.section_body(section) else {
            return;
        };

        let kept: Vec<Line> = self.lines[start..end]
            .iter()
            .filter(|line| matches!(line, Line::Verbatim(_)))
            .cloned()
            .collect();

        let fresh: Vec<Line> = pairs
            .iter()
            .map(|(key, value)| Line::Entry {
                raw: None,
                key: key.clone(),
                value: collapse_newline_tokens(value),
            })
            .collect();

        self.lines.splice(start..end, fresh.into_iter().chain(kept));
    }
}

/// Split text into lines, keeping empty ones and tolerating a missing final break.
fn split_lines(content: &str) -> Vec<&str> {
    if content.is_empty() {
        return Vec::new();
    }
    let body = content.strip_suffix('\n').unwrap_or(content);
    let body = body.strip_suffix('\r').unwrap_or(body);
    body.split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
        .collect()
}

/// Decide what a line is, keeping the original text for anything unedited.
fn classify(raw: &str) -> Line {
    let trimmed = raw.trim();

    if trimmed.is_empty() || trimmed.starts_with(';') {
        return Line::Verbatim(raw.to_string());
    }

    if let Some(name) = section_header(trimmed) {
        return Line::Section {
            raw: Some(raw.to_string()),
            name,
        };
    }

    match trimmed.split_once('=') {
        Some((key, value)) => Line::Entry {
            raw: Some(raw.to_string()),
            key: key.trim().to_string(),
            value: clean_value(value),
        },
        // Lines without a separator carry no value and are ignored on read.
        None => Line::Verbatim(raw.to_string()),
    }
}

/// Turn a line back into text, reusing the original where nothing changed.
fn render(line: &Line) -> String {
    match line {
        Line::Verbatim(raw) => raw.clone(),
        Line::Section { raw, name } => raw.clone().unwrap_or_else(|| format!("[{name}]")),
        Line::Entry { raw, key, value } => raw.clone().unwrap_or_else(|| format!("{key}={value}")),
    }
}

/// Extract a section name from a `[name]` line.
fn section_header(line: &str) -> Option<String> {
    let rest = line.strip_prefix('[')?;
    // Trailing text after the closing bracket is discarded, as the profile API does.
    let end = rest.rfind(']')?;
    Some(rest[..end].trim().to_string())
}

/// Trim a raw value and drop one enclosing pair of quotes.
fn clean_value(raw: &str) -> String {
    let trimmed = raw.trim();
    let chars: Vec<char> = trimmed.chars().collect();

    if chars.len() >= 2 {
        let first = chars[0];
        let last = chars[chars.len() - 1];
        if (first == '"' && last == '"') || (first == '\'' && last == '\'') {
            return chars[1..chars.len() - 1].iter().collect();
        }
    }

    trimmed.to_string()
}

/// Turn `<#LF#>` and `<#CR#>` into real line breaks.
fn expand_newline_tokens(value: &str) -> String {
    let with_lf = text::replace_in_string(value, "<#LF#>", "\n");
    text::replace_in_string(&with_lf, "<#CR#>", "\r")
}

/// Turn real line breaks back into the tokens an INI value can hold.
fn collapse_newline_tokens(value: &str) -> String {
    let with_lf = text::replace_in_string(value, "\n", "<#LF#>");
    text::replace_in_string(&with_lf, "\r", "<#CR#>")
}

/// Parse a decimal or `0x`-prefixed hexadecimal integer.
fn parse_integer(value: &str) -> Option<i32> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    let lower = trimmed.to_ascii_lowercase();
    if let Some(hex) = lower.strip_prefix("0x") {
        return u32::from_str_radix(hex, 16).ok().map(|v| v as i32);
    }
    if let Some(hex) = trimmed.strip_prefix('$') {
        return u32::from_str_radix(hex, 16).ok().map(|v| v as i32);
    }

    trimmed.parse::<i32>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
; a comment line
[Settings]
InstallerMode=1
LogLevel=4
WindowCaption=My Mod Installer
Quoted=\"spaced value\"
Hex=0x10

[2DAList]
Table0=spells.2da
Table1=feat.2da

[EmptySection]
";

    fn sample() -> IniFile {
        IniFile::parse(SAMPLE.as_bytes())
    }

    #[test]
    fn reads_values_and_defaults() {
        let ini = sample();
        assert_eq!(
            ini.read_string("Settings", "WindowCaption", ""),
            "My Mod Installer"
        );
        assert_eq!(
            ini.read_string("Settings", "Missing", "fallback"),
            "fallback"
        );
    }

    #[test]
    fn section_and_key_lookup_ignores_case() {
        let ini = sample();
        assert_eq!(ini.read_integer("settings", "loglevel", 3), 4);
        assert!(ini.section_exists("2DALIST"));
    }

    #[test]
    fn empty_sections_count_as_absent_but_are_present() {
        let ini = sample();
        assert!(!ini.section_exists("EmptySection"));
        assert!(ini.section_present("EmptySection"));
        assert!(!ini.section_exists("NeverDefined"));
        assert!(!ini.section_present("NeverDefined"));
    }

    #[test]
    fn keys_keep_file_order() {
        let ini = sample();
        assert_eq!(ini.read_section("2DAList"), vec!["Table0", "Table1"]);
    }

    #[test]
    fn flags_read_as_numbers() {
        let ini = sample();
        assert!(ini.read_bool("Settings", "InstallerMode", false));
        assert!(!ini.read_bool("Settings", "BackupFiles", false));
        assert!(ini.read_bool("Settings", "BackupFiles", true));
    }

    #[test]
    fn quotes_are_discarded_and_whitespace_trimmed() {
        let ini = IniFile::parse(b"[S]\nA=  padded  \nB=\"quoted\"\nC='single'\n");
        assert_eq!(ini.read_string("S", "A", ""), "padded");
        assert_eq!(ini.read_string("S", "B", ""), "quoted");
        assert_eq!(ini.read_string("S", "C", ""), "single");
    }

    #[test]
    fn hexadecimal_integers_are_understood() {
        let ini = sample();
        assert_eq!(ini.read_integer("Settings", "Hex", 0), 16);
    }

    #[test]
    fn duplicate_keys_resolve_to_the_first() {
        let ini = IniFile::parse(b"[S]\nKey=first\nKey=second\n");
        assert_eq!(ini.read_string("S", "Key", ""), "first");
        assert_eq!(ini.read_section("S").len(), 2);
    }

    #[test]
    fn newline_tokens_become_line_breaks() {
        let ini = IniFile::parse(b"[S]\nMsg=one<#LF#>two<#CR#>three\n");
        assert_eq!(ini.read_string("S", "Msg", ""), "one\ntwo\rthree");
    }

    #[test]
    fn values_keep_high_bytes_intact() {
        let ini = IniFile::parse(&[b'[', b'S', b']', b'\n', b'A', b'=', 0xE9, b'\n']);
        let value = ini.read_string("S", "A", "");
        assert_eq!(latin1::encode(&value), vec![0xE9]);
    }

    #[test]
    fn missing_files_parse_as_empty() {
        let ini = IniFile::load("/nonexistent/path/to/changes.ini");
        assert!(ini.section_names().is_empty());
        assert_eq!(ini.read_string("Settings", "Anything", "d"), "d");
    }

    // -- round trips -----------------------------------------------------

    #[test]
    fn untouched_files_come_back_byte_for_byte() {
        let raw = b"; header comment\r\n[Settings]\r\nWindowCaption = Spaced Out  \r\n\r\n; note\r\n[2DAList]\r\nTable0=spells.2da\r\n";
        let ini = IniFile::parse(raw);
        assert_eq!(ini.to_bytes(), raw.to_vec());
    }

    #[test]
    fn round_trip_keeps_quotes_and_odd_spacing() {
        // Reading strips the quotes; writing an untouched line must not.
        let raw = b"[S]\nA=\"quoted\"\nB=   spaced   \n";
        let ini = IniFile::parse(raw);
        assert_eq!(ini.read_string("S", "A", ""), "quoted");
        assert_eq!(ini.to_bytes(), raw.to_vec());
    }

    #[test]
    fn round_trip_survives_a_missing_final_newline() {
        let raw = b"[S]\nA=1";
        let ini = IniFile::parse(raw);
        assert_eq!(ini.to_bytes(), raw.to_vec());
    }

    #[test]
    fn round_trip_keeps_high_bytes() {
        let raw = &[b'[', b'S', b']', b'\n', b'A', b'=', 0xE9, 0xFF, b'\n'];
        let ini = IniFile::parse(raw);
        assert_eq!(ini.to_bytes(), raw.to_vec());
    }

    // -- writing ---------------------------------------------------------

    #[test]
    fn writing_an_existing_key_leaves_its_neighbours_alone() {
        let mut ini = IniFile::parse(b"[S]\r\nA=1\r\nB=2\r\nC=3\r\n");
        ini.write_string("S", "B", "changed");
        assert_eq!(
            ini.to_bytes(),
            b"[S]\r\nA=1\r\nB=changed\r\nC=3\r\n".to_vec()
        );
    }

    #[test]
    fn writing_the_same_value_changes_nothing() {
        let raw = b"[S]\nA =  1  \n";
        let mut ini = IniFile::parse(raw);
        ini.write_string("S", "A", "1");
        assert_eq!(ini.to_bytes(), raw.to_vec());
    }

    #[test]
    fn new_keys_land_above_the_trailing_blank_line() {
        let mut ini = IniFile::parse(b"[S]\nA=1\n\n[T]\nB=2\n");
        ini.write_string("S", "Z", "9");
        assert_eq!(ini.to_bytes(), b"[S]\nA=1\nZ=9\n\n[T]\nB=2\n".to_vec());
    }

    #[test]
    fn new_sections_append_with_a_separating_blank_line() {
        let mut ini = IniFile::parse(b"[S]\nA=1\n");
        ini.write_string("New", "K", "v");
        assert_eq!(ini.to_bytes(), b"[S]\nA=1\n\n[New]\nK=v\n".to_vec());
    }

    #[test]
    fn writing_into_an_empty_section_uses_it() {
        let mut ini = IniFile::parse(b"[S]\n\n[Empty]\n");
        ini.write_string("Empty", "K", "v");
        assert_eq!(ini.to_bytes(), b"[S]\n\n[Empty]\nK=v\n".to_vec());
    }

    #[test]
    fn line_breaks_are_stored_as_tokens() {
        let mut ini = IniFile::new();
        ini.write_string("Settings", "ConfirmMessage", "one\ntwo");
        let text = String::from_utf8(ini.to_bytes()).unwrap();
        assert!(text.contains("ConfirmMessage=one<#LF#>two"));
        assert_eq!(
            IniFile::parse(&ini.to_bytes()).read_string("Settings", "ConfirmMessage", ""),
            "one\ntwo"
        );
    }

    #[test]
    fn deleting_a_key_removes_every_copy() {
        let mut ini = IniFile::parse(b"[S]\nA=1\nB=2\nA=3\n");
        ini.delete_key("S", "a");
        assert_eq!(ini.to_bytes(), b"[S]\nB=2\n".to_vec());
    }

    #[test]
    fn deleting_a_section_takes_its_body() {
        let mut ini = IniFile::parse(b"[S]\nA=1\n\n[T]\nB=2\n\n[U]\nC=3\n");
        ini.delete_section("T");
        assert!(!ini.section_present("T"));
        assert!(ini.section_exists("S"));
        assert!(ini.section_exists("U"));
    }

    #[test]
    fn renaming_keeps_position_and_contents() {
        let mut ini = IniFile::parse(b"[Old]\nA=1\n\n[Other]\nB=2\n");
        ini.rename_section("Old", "New");
        ini.rename_key("New", "A", "Z");
        assert_eq!(ini.to_bytes(), b"[New]\nZ=1\n\n[Other]\nB=2\n".to_vec());
        assert_eq!(ini.section_names(), vec!["New", "Other"]);
    }

    #[test]
    fn replacing_values_renumbers_and_drops_the_tail() {
        let mut ini = IniFile::parse(b"[L]\nFile0=a\nFile1=b\nFile2=c\n\n[After]\nK=v\n");
        ini.replace_section_values(
            "L",
            &[
                ("File0".to_string(), "a".to_string()),
                ("File1".to_string(), "c".to_string()),
            ],
        );
        assert_eq!(ini.read_section("L"), vec!["File0", "File1"]);
        assert_eq!(ini.read_string("L", "File1", ""), "c");
        assert!(ini.section_exists("After"));
        // The blank line that separated the sections is still there.
        assert_eq!(
            ini.to_bytes(),
            b"[L]\nFile0=a\nFile1=c\n\n[After]\nK=v\n".to_vec()
        );
    }

    #[test]
    fn an_edit_leaves_the_rest_of_the_file_untouched() {
        let raw = b"; keep me\r\n[Settings]\r\nWindowCaption  =  Old Name  \r\n; and me\r\nLogLevel=4\r\n\r\n[2DAList]\r\nTable0=spells.2da\r\n";
        let mut ini = IniFile::parse(raw);
        ini.write_string("Settings", "WindowCaption", "New Name");
        let out = String::from_utf8(ini.to_bytes()).unwrap();
        assert!(out.contains("; keep me\r\n"));
        assert!(out.contains("; and me\r\n"));
        assert!(out.contains("WindowCaption=New Name\r\n"));
        assert!(out.contains("LogLevel=4\r\n"));
        assert!(out.ends_with("Table0=spells.2da\r\n"));
    }
}
