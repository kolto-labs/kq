//! Delimiter splitting used by field paths and multi-part values.
//!
//! Field paths (`ClassList\0\Class`) and vector values (`1.0|2.0|3.0`) are split
//! with the same routine the original used. Its edge cases are load-bearing:
//! very short strings are never split, and empty segments are dropped, so
//! `A\\B` yields two parts rather than three.

/// Split `input` on `delimiter`, reproducing the original splitter's rules.
///
/// - Strings shorter than three characters are returned whole.
/// - A segment that is just the delimiter is skipped, which removes leading and
///   repeated delimiters.
/// - A trailing delimiter does not produce an empty final segment.
pub fn tokenize(input: &str, delimiter: char) -> Vec<String> {
    let chars: Vec<char> = input.chars().collect();

    if chars.len() < 3 {
        return vec![input.to_string()];
    }

    let mut parts = Vec::new();
    let mut last = 0usize; // 1-based index of the previous delimiter

    for i in 1..=chars.len() {
        let current = chars[i - 1];

        if current == delimiter {
            let segment = substring(&chars, last + 1, i);
            // A lone delimiter segment means two delimiters in a row.
            if !segment.starts_with(delimiter) || segment.chars().count() != 1 {
                parts.push(segment);
            }
            last = i;
        } else if i == chars.len() {
            parts.push(substring(&chars, last + 1, i + 1));
            last = i;
        }
    }

    parts
}

/// Characters from `start` up to but not including `end`, both 1-based.
///
/// When `start` and `end` are equal a single character is returned, which is
/// what makes back-to-back delimiters detectable.
fn substring(chars: &[char], start: usize, end: usize) -> String {
    use std::cmp::Ordering;

    match end.cmp(&start) {
        Ordering::Greater => chars[start - 1..(end - 1).min(chars.len())]
            .iter()
            .collect(),
        Ordering::Equal => chars
            .get(start - 1)
            .map(|c| c.to_string())
            .unwrap_or_default(),
        Ordering::Less => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_a_simple_path() {
        assert_eq!(tokenize("A\\B", '\\'), vec!["A", "B"]);
        assert_eq!(
            tokenize("ClassList\\0\\Class", '\\'),
            vec!["ClassList", "0", "Class"]
        );
    }

    #[test]
    fn short_strings_are_returned_whole() {
        assert_eq!(tokenize("A", '\\'), vec!["A"]);
        assert_eq!(tokenize("AB", '\\'), vec!["AB"]);
        // Two characters are below the split threshold, delimiter or not.
        assert_eq!(tokenize("A\\", '\\'), vec!["A\\"]);
    }

    #[test]
    fn a_path_without_delimiters_is_one_part() {
        assert_eq!(tokenize("ABC", '\\'), vec!["ABC"]);
    }

    #[test]
    fn leading_and_repeated_delimiters_are_dropped() {
        assert_eq!(tokenize("\\AB", '\\'), vec!["AB"]);
        assert_eq!(tokenize("A\\\\B", '\\'), vec!["A", "B"]);
    }

    #[test]
    fn trailing_delimiter_adds_no_empty_part() {
        assert_eq!(tokenize("ABC\\", '\\'), vec!["ABC"]);
    }

    #[test]
    fn splits_vector_values() {
        assert_eq!(tokenize("1.0|2.0|3.0", '|'), vec!["1.0", "2.0", "3.0"]);
        assert_eq!(
            tokenize("0.0|1.0|5.0|0.5", '|'),
            vec!["0.0", "1.0", "5.0", "0.5"]
        );
    }
}
