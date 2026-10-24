//! String and number handling that mirrors the original patcher's semantics.
//!
//! Mod configuration files were authored against a specific set of validation
//! and conversion rules. Reimplementing those rules exactly — including the
//! quirks — is what keeps existing mods installing the same way.

/// True when every character is an ASCII digit and the string is not empty.
pub fn is_number(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

/// True when the string is a digit run optionally led by `-`.
///
/// A lone `-` passes, matching the original check, which only validated the
/// first character against `0-9` or `-` and then looked at the remainder.
pub fn is_number_signed(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_digit() || first == '-') {
        return false;
    }
    chars.all(|c| c.is_ascii_digit())
}

/// True when the string looks like a decimal number.
///
/// The first character must be a digit or `-`, the last must be a digit, and
/// everything between may also include `.` and `,` as decimal separators.
pub fn is_float(s: &str) -> bool {
    let chars: Vec<char> = s.chars().collect();
    if chars.is_empty() {
        return false;
    }
    // Scientific notation (e.g. "-8.4006E-7"): accept when it parses as a real
    // number, matching TSLPatcher/HoloPatcher, which use StrToFloat/float().
    // Only strings carrying an exponent marker take this path, so plain values
    // like "1." or ".5" keep their existing meaning, and any value accepted
    // here is guaranteed to parse in `safe_str_to_double`.
    if s.contains('e') || s.contains('E') {
        return s.replacen(',', ".", 1).parse::<f64>().is_ok();
    }
    if !(chars[0].is_ascii_digit() || chars[0] == '-') {
        return false;
    }
    // Interior characters only; a single-character string skips this entirely.
    for &c in chars.iter().take(chars.len().saturating_sub(1)).skip(1) {
        if !(c.is_ascii_digit() || c == '.' || c == ',') {
            return false;
        }
    }
    chars[chars.len() - 1].is_ascii_digit()
}

/// Parse a 32-bit integer, accepting the unsigned maximum as `-1`.
///
/// `4294967295` does not fit a signed 32-bit integer, but configuration files
/// use it to mean "all bits set". It is translated rather than rejected.
pub fn safe_str_to_int(s: &str) -> Option<i32> {
    if s == "4294967295" {
        return Some(-1);
    }
    s.parse::<i32>().ok()
}

/// Parse a 64-bit integer.
pub fn str_to_int64(s: &str) -> Option<i64> {
    s.parse::<i64>().ok()
}

/// Parse a decimal number, accepting either `.` or `,` as the separator.
///
/// Text that does not look like a number at all yields `0.0`. Text that looks
/// like one but still will not convert — `1.2.3`, say — is an error, because
/// only one separator character is exchanged before conversion and the rest
/// makes the number unreadable.
pub fn safe_str_to_double(s: &str) -> Result<f64, String> {
    if !is_float(s) {
        return Ok(0.0);
    }

    // Only the first separator is exchanged, matching the original conversion.
    let normalized: String = s.replacen(',', ".", 1);

    normalized
        .parse::<f64>()
        .map_err(|_| format!("'{s}' is not a valid floating point value"))
}

/// Parse a single-precision decimal number. See [`safe_str_to_double`].
pub fn safe_str_to_float(s: &str) -> Result<f32, String> {
    safe_str_to_double(s).map(|value| value as f32)
}

/// Reduce text to resource-reference form: at most 16 characters from the
/// set used by retail KotOR / HoloPatcher (PyKotor) resrefs.
///
/// Allowed: ASCII alphanumeric, `_`, `-`, `+`, `!`. Hyphens appear in names
/// like `k_pdan_hist1-4`; plus and bang appear in K1CP script resrefs
/// (`k_pdan_state1+`, `k_pkor_!knexcav`). Everything else is dropped.
pub fn string_to_resref(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(*c, '_' | '-' | '+' | '!'))
        .take(16)
        .collect()
}

/// Locate `needle` in `haystack` starting the search *after* `start_1based`.
///
/// Positions are 1-based and the search begins at `start_1based + 1`, so an
/// occurrence at the very first position is never reported. Returns 0 for
/// "not found". This is the search primitive [`replace_in_string`] is built on.
fn search_after(needle: &str, haystack: &str, start_1based: usize) -> usize {
    let start = start_1based + 1;
    let chars: Vec<char> = haystack.chars().collect();
    if chars.len() <= start {
        return 0;
    }
    let tail: String = chars[start - 1..].iter().collect();
    match find_char_index(&tail, needle) {
        Some(rel) => start + rel,
        None => 0,
    }
}

/// Character-index (0-based) of `needle` within `haystack`, or `None`.
fn find_char_index(haystack: &str, needle: &str) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }
    let hay: Vec<char> = haystack.chars().collect();
    let ndl: Vec<char> = needle.chars().collect();
    if ndl.len() > hay.len() {
        return None;
    }
    (0..=hay.len() - ndl.len()).find(|&i| hay[i..i + ndl.len()] == ndl[..])
}

/// Replace occurrences of `find` with `replace` throughout `source`.
///
/// Scanning starts one character in, so a match sitting at the very start of
/// the string is left alone. Mods have been authored and tested against that
/// behavior — token lines in scripts are indented, and INI values that open
/// with a newline token keep it literal — so the scan window is preserved
/// rather than corrected.
pub fn replace_in_string(source: &str, find: &str, replace: &str) -> String {
    let mut current = source.to_string();
    let mut last_pos = 1usize;

    loop {
        let pos = search_after(find, &current, last_pos);
        last_pos = pos;
        if pos == 0 {
            break;
        }

        let chars: Vec<char> = current.chars().collect();
        let find_len = find.chars().count();
        let head: String = chars[..pos - 1].iter().collect();
        let tail: String = chars[(pos - 1 + find_len).min(chars.len())..]
            .iter()
            .collect();
        current = format!("{head}{replace}{tail}");
    }

    current
}

/// Extension of a path including the leading dot, or empty when there is none.
pub fn extract_file_ext(path: &str) -> String {
    let chars: Vec<char> = path.chars().collect();
    for i in (0..chars.len()).rev() {
        match chars[i] {
            '.' => return chars[i..].iter().collect(),
            '\\' | '/' | ':' => return String::new(),
            _ => {}
        }
    }
    String::new()
}

/// Final path component, with any directory prefix removed.
pub fn extract_file_name(path: &str) -> String {
    let chars: Vec<char> = path.chars().collect();
    for i in (0..chars.len()).rev() {
        if matches!(chars[i], '\\' | '/' | ':') {
            return chars[i + 1..].iter().collect();
        }
    }
    path.to_string()
}

/// Directory portion of a path, including the trailing separator.
pub fn extract_file_path(path: &str) -> String {
    let chars: Vec<char> = path.chars().collect();
    for i in (0..chars.len()).rev() {
        if matches!(chars[i], '\\' | '/' | ':') {
            return chars[..=i].iter().collect();
        }
    }
    String::new()
}

/// The separator this platform writes into paths.
///
/// Instruction files are written with `\`, and both forms are accepted when
/// reading a path apart. Paths built here use the native form so log lines and
/// messages read naturally on every platform.
pub const SEPARATOR: char = std::path::MAIN_SEPARATOR;

/// Append a trailing separator unless the path already ends with one.
pub fn include_trailing_delimiter(path: &str) -> String {
    if path.is_empty() {
        return SEPARATOR.to_string();
    }
    if path.ends_with('\\') || path.ends_with('/') {
        path.to_string()
    } else {
        format!("{path}{SEPARATOR}")
    }
}

/// Join a folder and a name with the native separator.
pub fn join_path(dir: &str, name: &str) -> String {
    format!("{}{name}", include_trailing_delimiter(dir))
}

/// Drop a file's extension by cutting at the *first* occurrence of the
/// extension text within the name.
///
/// Mirrors how the original built output names: it searched for the extension
/// string rather than the last dot. When there is no extension the result is
/// empty, because the cut length becomes negative.
pub fn strip_extension(path: &str) -> String {
    let ext = extract_file_ext(path);
    if ext.is_empty() {
        return String::new();
    }
    match find_char_index(path, &ext) {
        Some(idx) => path.chars().take(idx).collect(),
        None => String::new(),
    }
}

/// Replace a file's extension, keeping the original's name-cutting rule.
pub fn with_extension(path: &str, new_ext: &str) -> String {
    format!("{}{}", strip_extension(path), new_ext)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_checks_match_original_rules() {
        assert!(is_number("0"));
        assert!(is_number("12345"));
        assert!(!is_number(""));
        assert!(!is_number("-1"));
        assert!(!is_number("1a"));

        assert!(is_number_signed("-1"));
        assert!(is_number_signed("42"));
        // A lone minus passes the original's first-character check.
        assert!(is_number_signed("-"));
        assert!(!is_number_signed(""));
        assert!(!is_number_signed("1-2"));
    }

    #[test]
    fn float_checks_match_original_rules() {
        assert!(is_float("1"));
        assert!(is_float("-1"));
        assert!(is_float("1.5"));
        assert!(is_float("1,5"));
        assert!(is_float("-12.75"));
        assert!(!is_float("1."));
        assert!(!is_float(".5"));
        assert!(!is_float(""));
        assert!(!is_float("-"));
        assert!(!is_float("abc"));
        // Scientific notation is accepted (K1CP ships Float values like this).
        assert!(is_float("-8.40060238260776E-7"));
        assert!(is_float("1.5e10"));
        assert!(is_float("2E3"));
        assert!(!is_float("e"));
        assert!(!is_float("1.2.3e4"));
    }

    #[test]
    fn unsigned_max_maps_to_all_bits_set() {
        assert_eq!(safe_str_to_int("4294967295"), Some(-1));
        assert_eq!(safe_str_to_int("-1"), Some(-1));
        assert_eq!(safe_str_to_int("7"), Some(7));
        assert_eq!(safe_str_to_int("nope"), None);
    }

    #[test]
    fn float_parsing_accepts_both_separators() {
        assert_eq!(safe_str_to_double("1.5"), Ok(1.5));
        assert_eq!(safe_str_to_double("1,5"), Ok(1.5));
        // Text that is not a number at all converts to zero.
        assert_eq!(safe_str_to_double("bad"), Ok(0.0));
    }

    #[test]
    fn a_number_with_two_separators_will_not_convert() {
        // These pass the shape check but cannot be read as one number.
        assert!(is_float("1.234.567"));
        assert!(safe_str_to_double("1.234.567").is_err());
        assert!(safe_str_to_double("1,2,3").is_err());
    }

    #[test]
    fn resref_conversion_filters_and_truncates() {
        assert_eq!(string_to_resref("my item!.uti"), "myitem!uti");
        assert_eq!(
            string_to_resref("abcdefghijklmnopqrstuv"),
            "abcdefghijklmnop"
        );
        assert_eq!(string_to_resref("a_b-c"), "a_b-c");
        assert_eq!(string_to_resref("k_pdan_hist1-4"), "k_pdan_hist1-4");
        assert_eq!(string_to_resref("k_pdan_state1+"), "k_pdan_state1+");
        assert_eq!(string_to_resref("k_pkor_!knexcav"), "k_pkor_!knexcav");
        assert_eq!(string_to_resref("k_pkas_morph++"), "k_pkas_morph++");
    }

    #[test]
    fn replacement_skips_a_match_at_the_start() {
        // Preserved quirk: the scan begins at the second character.
        assert_eq!(replace_in_string("#TOK#tail", "#TOK#", "X"), "#TOK#tail");
        assert_eq!(replace_in_string(" #TOK#tail", "#TOK#", "X"), " Xtail");
    }

    #[test]
    fn replacement_handles_repeats_and_absence() {
        assert_eq!(replace_in_string("a b b c", "b", "Z"), "a Z Z c");
        assert_eq!(replace_in_string("nothing", "q", "Z"), "nothing");
        assert_eq!(replace_in_string("", "q", "Z"), "");
    }

    #[test]
    fn replacement_expands_newline_tokens() {
        assert_eq!(
            replace_in_string("line<#LF#>next", "<#LF#>", "\n"),
            "line\nnext"
        );
    }

    #[test]
    fn path_helpers_split_on_either_separator() {
        assert_eq!(extract_file_ext("dir\\file.2da"), ".2da");
        assert_eq!(extract_file_ext("dir.x/file"), "");
        assert_eq!(extract_file_ext("noext"), "");

        assert_eq!(extract_file_name("a\\b\\c.txt"), "c.txt");
        assert_eq!(extract_file_name("a/b/c.txt"), "c.txt");
        assert_eq!(extract_file_name("c.txt"), "c.txt");

        assert_eq!(extract_file_path("a\\b\\c.txt"), "a\\b\\");
        assert_eq!(extract_file_path("c.txt"), "");
    }

    #[test]
    fn trailing_delimiter_is_added_once() {
        assert_eq!(include_trailing_delimiter("dir"), format!("dir{SEPARATOR}"));
        // Either separator already present is left alone.
        assert_eq!(include_trailing_delimiter("dir\\"), "dir\\");
        assert_eq!(include_trailing_delimiter("dir/"), "dir/");
    }

    #[test]
    fn joining_uses_the_native_separator() {
        assert_eq!(
            join_path("dir", "file.2da"),
            format!("dir{SEPARATOR}file.2da")
        );
    }

    #[test]
    fn extension_stripping_cuts_at_first_match() {
        assert_eq!(strip_extension("script.nss"), "script");
        assert_eq!(with_extension("script.nss", ".ncs"), "script.ncs");
        // No extension means nothing survives the cut.
        assert_eq!(strip_extension("plain"), "");
    }
}
