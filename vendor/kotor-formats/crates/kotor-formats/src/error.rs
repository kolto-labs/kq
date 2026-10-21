//! Error types.
//!
//! Failures carry the subsystem that produced them and a numeric code. Log
//! lines and dialogs render these as `message (2DA-8)`, which is the form mod
//! authors have been reading in install logs for years.

use std::fmt;

/// Which subsystem raised the error. Determines the tag shown in logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subsystem {
    /// String table handling.
    Tlk,
    /// Two-dimensional table handling.
    TwoDa,
    /// General patch flow.
    General,
    /// Structured game file handling.
    Gff,
    /// Soundset handling.
    Ssf,
    /// Archive handling.
    Erf,
    /// External process or file access.
    External,
}

impl Subsystem {
    /// Short tag used in log messages and dialogs.
    pub fn tag(self) -> &'static str {
        match self {
            Subsystem::Tlk => "TLK",
            Subsystem::TwoDa => "2DA",
            Subsystem::General => "GEN",
            Subsystem::Gff => "GFF",
            Subsystem::Ssf => "SSF",
            Subsystem::Erf => "ERF",
            Subsystem::External => "EXT",
        }
    }

    /// True when the string table tag is shown without a code number.
    fn hides_code(self) -> bool {
        matches!(self, Subsystem::Tlk)
    }
}

/// An error raised while reading, writing, or patching game data.
#[derive(Debug, Clone)]
pub struct PatchError {
    subsystem: Subsystem,
    code: i32,
    message: String,
}

impl PatchError {
    /// Build an error for a subsystem with a specific code.
    pub fn new(subsystem: Subsystem, code: i32, message: impl Into<String>) -> Self {
        Self {
            subsystem,
            code,
            message: message.into(),
        }
    }

    /// String table error.
    pub fn tlk(message: impl Into<String>) -> Self {
        Self::new(Subsystem::Tlk, 0, message)
    }

    /// Table error with a code.
    pub fn twoda(code: i32, message: impl Into<String>) -> Self {
        Self::new(Subsystem::TwoDa, code, message)
    }

    /// General patch-flow error with a code.
    pub fn general(code: i32, message: impl Into<String>) -> Self {
        Self::new(Subsystem::General, code, message)
    }

    /// Structured game file error with a code.
    pub fn gff(code: i32, message: impl Into<String>) -> Self {
        Self::new(Subsystem::Gff, code, message)
    }

    /// Soundset error with a code.
    pub fn ssf(code: i32, message: impl Into<String>) -> Self {
        Self::new(Subsystem::Ssf, code, message)
    }

    /// Archive error with a code.
    pub fn erf(code: i32, message: impl Into<String>) -> Self {
        Self::new(Subsystem::Erf, code, message)
    }

    /// External process or file access error.
    pub fn external(code: i32, message: impl Into<String>) -> Self {
        Self::new(Subsystem::External, code, message)
    }

    /// The subsystem that raised this error.
    pub fn subsystem(&self) -> Subsystem {
        self.subsystem
    }

    /// The numeric code, used to distinguish recoverable cases.
    pub fn code(&self) -> i32 {
        self.code
    }

    /// The message without the subsystem tag.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The message with its subsystem tag, as written to the install log.
    pub fn tagged(&self) -> String {
        if self.subsystem.hides_code() {
            format!("{} ({})", self.message, self.subsystem.tag())
        } else {
            format!("{} ({}-{})", self.message, self.subsystem.tag(), self.code)
        }
    }
}

impl fmt::Display for PatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.tagged())
    }
}

impl std::error::Error for PatchError {}

impl From<std::io::Error> for PatchError {
    fn from(err: std::io::Error) -> Self {
        PatchError::external(1, err.to_string())
    }
}

/// Result type used throughout the patcher.
pub type Result<T> = std::result::Result<T, PatchError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_errors_show_tag_and_code() {
        let err = PatchError::twoda(8, "Unable to find a column matching the label x");
        assert_eq!(
            err.tagged(),
            "Unable to find a column matching the label x (2DA-8)"
        );
    }

    #[test]
    fn string_table_errors_omit_the_code() {
        let err = PatchError::tlk("No TLK file loaded. Unable to proceed.");
        assert_eq!(err.tagged(), "No TLK file loaded. Unable to proceed. (TLK)");
    }

    #[test]
    fn code_is_available_for_recovery_checks() {
        let err = PatchError::twoda(10, "row missing");
        assert_eq!(err.code(), 10);
        assert_eq!(err.subsystem(), Subsystem::TwoDa);
    }
}
