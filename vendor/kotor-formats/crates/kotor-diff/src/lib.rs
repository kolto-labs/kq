//! Compares BioWare Aurora/Odyssey data files and reports what changed.
//!
//! Two tools needed this and each had grown its own answer. A query tool
//! compares resources structurally, as JSON, and can patch and merge them. An
//! instruction-file editor needs the opposite direction: given the file a mod
//! started from and the file it produced, work out the instructions that turn
//! one into the other.
//!
//! Both are the same question asked of the same formats, so both live here
//! rather than in one tool where the other cannot reach them.
//!
//! # The two answers
//!
//! [`changes`] compares a pair of files and writes the TSLPatcher instructions
//! that reproduce the difference. It works on the types `kotor-formats` parses,
//! because an instruction has to name a field's exact on-disk type — `Byte` and
//! `DWORD` are different keywords — and only the parsed type still knows it.
//!
//! [`json`] is the structural diff, patch and three-way merge over JSON values,
//! behind the `json` feature. It knows nothing about game formats; it works on
//! whatever a caller has already turned into JSON.
//!
//! # Where the line falls
//!
//! [`changes`] produces a document as ordered sections and keys, not as text,
//! until [`changes::ChangesIni::render`] is called. An editor that already has a
//! file open can therefore merge the generated sections into it through its own
//! writer, instead of pasting in a block of text and losing whatever formatting
//! the file already had.

pub mod changes;

#[cfg(feature = "json")]
pub mod json;
