//! Round-trip readers and writers for BioWare Aurora/Odyssey data formats.
//!
//! Loading a file and saving it back without edits produces the same bytes,
//! which is what a patcher needs to be trustworthy: it must reproduce
//! everything it does not touch, exactly.
//!
//! That holds for [`ini`] too, which is not a BioWare format but is the one
//! every tool here reads: the instruction file describing what to patch. It
//! keeps comments, spacing and line endings, so an editor can change one value
//! without rewriting the file around it.

pub mod error;
pub mod fsutil;
pub mod latin1;
pub mod text;

pub mod erf;
pub mod gff;
pub mod ini;
pub mod ssf;
pub mod strtok;
pub mod tlk;
pub mod twoda;
