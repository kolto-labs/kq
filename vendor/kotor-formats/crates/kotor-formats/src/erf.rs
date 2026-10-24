//! Archive files (`ERF`, `MOD`, `HAK`, `SAV`, and `RIM`).
//!
//! Archives bundle many game resources into one file. All five share a
//! four-character type tag and version `V1.0`; ERF-family archives carry
//! localized description strings and split their directory into a key list and
//! a resource list, while RIM keeps a single combined directory.
//!
//! Saving rebuilds the whole archive. Existing resources keep their order and
//! newly added ones follow, so the result is reproducible on every platform.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::{PatchError, Result};
use crate::{fsutil, latin1, text};

const ERF_HEADER_SIZE: u32 = 160;
const ERF_KEY_ENTRY_SIZE: u32 = 24;
const ERF_RES_ENTRY_SIZE: u32 = 8;

const RIM_HEADER_SIZE: u32 = 120;
const RIM_KEY_ENTRY_SIZE: u32 = 32;

/// Which archive flavour a file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKind {
    /// Module archive.
    Mod,
    /// Resource pack.
    Hak,
    /// Generic archive.
    Erf,
    /// Saved game.
    Sav,
    /// Room resource archive, with a simpler directory.
    Rim,
}

impl ArchiveKind {
    /// The four-character tag written at the start of the file.
    pub fn tag(self) -> &'static [u8; 4] {
        match self {
            ArchiveKind::Mod => b"MOD ",
            ArchiveKind::Hak => b"HAK ",
            ArchiveKind::Erf => b"ERF ",
            ArchiveKind::Sav => b"SAV ",
            ArchiveKind::Rim => b"RIM ",
        }
    }

    /// Recognise a tag, or `None` when it is not an archive.
    pub fn from_tag(tag: &[u8]) -> Option<Self> {
        match tag {
            b"MOD " => Some(ArchiveKind::Mod),
            b"HAK " => Some(ArchiveKind::Hak),
            b"ERF " => Some(ArchiveKind::Erf),
            b"SAV " => Some(ArchiveKind::Sav),
            b"RIM " => Some(ArchiveKind::Rim),
            _ => None,
        }
    }

    /// True for the RIM layout, which has no description strings.
    pub fn is_rim(self) -> bool {
        matches!(self, ArchiveKind::Rim)
    }
}

/// One localized description string attached to an archive.
#[derive(Debug, Clone)]
pub struct LocString {
    /// Language identifier.
    pub language_id: u32,
    /// The description text.
    pub text: String,
}

/// One resource stored inside an archive.
#[derive(Debug, Clone)]
pub struct Resource {
    /// Resource name without extension.
    pub resref: String,
    /// Numeric type identifying the resource's extension.
    pub res_type: u16,
    /// Unused field preserved from the directory entry.
    pub reserved_erf: u16,
    /// Unused field preserved from RIM directory entries.
    pub reserved_rim: u16,
    /// The resource's bytes.
    pub data: Vec<u8>,
}

impl Resource {
    /// Filename this resource would have when extracted.
    pub fn file_name(&self) -> String {
        let ext = res_type_to_extension(self.res_type);
        if ext.is_empty() {
            self.resref.clone()
        } else {
            format!("{}.{}", self.resref, ext)
        }
    }
}

/// A file queued to be added when the archive is next saved.
#[derive(Debug, Clone)]
struct PendingAdd {
    source_path: String,
    save_as: String,
}

/// A loaded archive.
#[derive(Debug, Clone)]
pub struct ErfFile {
    /// Which archive flavour this is.
    pub kind: ArchiveKind,
    /// Localized description strings. Always empty for RIM archives.
    pub loc_strings: Vec<LocString>,
    /// String-table reference for the archive description.
    pub loc_string_strref: u32,
    /// Value of the RIM header's unused leading field.
    pub rim_unknown: u32,
    resources: Vec<Resource>,
    pending: Vec<PendingAdd>,
    path: String,
    loaded: bool,
    dirty: bool,
}

impl Default for ErfFile {
    fn default() -> Self {
        Self {
            kind: ArchiveKind::Erf,
            loc_strings: Vec::new(),
            loc_string_strref: 0,
            rim_unknown: 0,
            resources: Vec::new(),
            pending: Vec::new(),
            path: String::new(),
            loaded: false,
            dirty: false,
        }
    }
}

impl ErfFile {
    /// An empty, unloaded archive.
    pub fn new() -> Self {
        Self::default()
    }

    /// Start a blank archive of the given flavour.
    pub fn create(path: &str, kind: ArchiveKind) -> Self {
        Self {
            kind,
            path: path.to_string(),
            loaded: true,
            ..Self::default()
        }
    }

    /// True once an archive has been read or created.
    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    /// True when resources have been added or removed since loading.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Path the archive was read from.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Resources currently stored, not counting queued additions.
    pub fn resources(&self) -> &[Resource] {
        &self.resources
    }

    /// Number of stored resources.
    pub fn count(&self) -> usize {
        self.resources.len()
    }

    /// Number of queued additions.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Quick check of whether a file on disk is an archive.
    pub fn is_valid_archive(path: &str) -> bool {
        let Ok(bytes) = fsutil::read_file(path) else {
            return false;
        };
        if bytes.len() < 8 {
            return false;
        }
        &bytes[4..8] == b"V1.0" && ArchiveKind::from_tag(&bytes[0..4]).is_some()
    }

    /// Read an archive from disk.
    pub fn load(path: &str) -> Result<Self> {
        if !fsutil::file_exists(path) {
            return Err(PatchError::erf(
                5,
                format!("Unable to load file \"{path}\", file not found!"),
            ));
        }

        let bytes = fsutil::read_file(path)
            .map_err(|e| PatchError::erf(5, format!("Unable to read archive {path}: {e}")))?;
        Self::parse(&bytes, path)
    }

    /// Parse archive bytes.
    pub fn parse(bytes: &[u8], path: &str) -> Result<Self> {
        if bytes.len() < 8 {
            return Err(PatchError::erf(
                6,
                format!("Unable to load file \"{path}\", it does not appear to be a valid ERF file type!"),
            ));
        }

        let kind = ArchiveKind::from_tag(&bytes[0..4]);
        if &bytes[4..8] != b"V1.0" || kind.is_none() {
            return Err(PatchError::erf(
                6,
                format!("Unable to load file \"{path}\", it does not appear to be a valid ERF file type!"),
            ));
        }

        let kind = kind.expect("checked above");
        let mut archive = Self {
            kind,
            path: path.to_string(),
            loaded: true,
            ..Self::default()
        };

        if kind.is_rim() {
            archive.rim_unknown = read_u32(bytes, 8)?;
            let entry_count = read_u32(bytes, 12)?;
            let key_offset = read_u32(bytes, 16)?;

            for i in 0..entry_count {
                let at = key_offset + i * RIM_KEY_ENTRY_SIZE;
                let resref = text::string_to_resref(&latin1::decode_nul_terminated(read_slice(
                    bytes, at, 16,
                )?));
                let res_type = read_u16(bytes, at + 16)?;
                let reserved_erf = read_u16(bytes, at + 18)?;
                let reserved_rim = read_u16(bytes, at + 22)?;
                let data_offset = read_u32(bytes, at + 24)?;
                let data_size = read_u32(bytes, at + 28)?;

                archive.resources.push(Resource {
                    resref,
                    res_type,
                    reserved_erf,
                    reserved_rim,
                    data: read_slice(bytes, data_offset, data_size)?.to_vec(),
                });
            }
        } else {
            let loc_string_count = read_u32(bytes, 8)?;
            let entry_count = read_u32(bytes, 16)?;
            let loc_string_offset = read_u32(bytes, 20)?;
            let key_offset = read_u32(bytes, 24)?;
            let res_offset = read_u32(bytes, 28)?;
            archive.loc_string_strref = read_u32(bytes, 40)?;

            let mut cursor = loc_string_offset;
            for _ in 0..loc_string_count {
                let language_id = read_u32(bytes, cursor)?;
                let size = read_u32(bytes, cursor + 4)?;
                let raw = read_slice(bytes, cursor + 8, size)?;
                archive.loc_strings.push(LocString {
                    language_id,
                    text: latin1::decode(raw),
                });
                cursor += 8 + size;
            }

            for i in 0..entry_count {
                let key_at = key_offset + i * ERF_KEY_ENTRY_SIZE;
                let resref = text::string_to_resref(&latin1::decode_nul_terminated(read_slice(
                    bytes, key_at, 16,
                )?));
                let res_type = read_u16(bytes, key_at + 20)?;
                let reserved_erf = read_u16(bytes, key_at + 22)?;

                let res_at = res_offset + i * ERF_RES_ENTRY_SIZE;
                let data_offset = read_u32(bytes, res_at)?;
                let data_size = read_u32(bytes, res_at + 4)?;

                archive.resources.push(Resource {
                    resref,
                    res_type,
                    reserved_erf,
                    reserved_rim: 0,
                    data: read_slice(bytes, data_offset, data_size)?.to_vec(),
                });
            }
        }

        Ok(archive)
    }

    /// Position of a stored resource matching a name and type.
    fn position_of(&self, resref: &str, res_type: u16) -> Option<usize> {
        self.resources
            .iter()
            .position(|r| r.resref.eq_ignore_ascii_case(resref) && r.res_type == res_type)
    }

    /// True when a resource matching this filename is already stored.
    ///
    /// Queued additions are only considered when `include_pending` is set, so a
    /// caller that adds several files in one session sees each of them as new
    /// unless it asks otherwise.
    pub fn resource_exists(&self, file_name: &str, include_pending: bool) -> Result<bool> {
        if !self.loaded {
            return Err(PatchError::erf(
                3,
                "Unable to get resource from file, no ERF file is open!",
            ));
        }

        let resref = text::string_to_resref(&text::strip_extension(file_name).to_lowercase());
        let res_type = extension_to_res_type(&text::extract_file_ext(file_name));

        if self.position_of(&resref, res_type).is_some() {
            return Ok(true);
        }

        if include_pending {
            for entry in &self.pending {
                let queued_name = text::extract_file_name(&entry.save_as);
                if queued_name.eq_ignore_ascii_case(&text::extract_file_name(file_name)) {
                    return Ok(true);
                }

                let queued_resref =
                    text::string_to_resref(&text::strip_extension(&entry.save_as).to_lowercase());
                let queued_ext = text::extract_file_ext(&entry.save_as);
                if queued_resref == resref
                    && queued_ext.eq_ignore_ascii_case(&text::extract_file_ext(file_name))
                {
                    return Ok(true);
                }
            }
        }

        Ok(false)
    }

    /// Extract a resource's bytes.
    pub fn resource_data(&self, resref: &str, res_type: u16) -> Result<&[u8]> {
        if !self.loaded {
            return Err(PatchError::erf(
                3,
                "Unable to get resource from file, no ERF file is open!",
            ));
        }

        self.position_of(resref, res_type)
            .map(|i| self.resources[i].data.as_slice())
            .ok_or_else(|| {
                PatchError::erf(
                    14,
                    format!(
                        "Resource \"{}.{}\" could not be found in the ERF file. Unable to extract it!",
                        resref,
                        res_type_to_extension(res_type)
                    ),
                )
            })
    }

    /// Extract a resource to a file on disk.
    pub fn extract_to(&self, resref: &str, res_type: u16, destination: &str) -> Result<()> {
        let data = self.resource_data(resref, res_type)?.to_vec();
        fsutil::write_file(destination, &data).map_err(|e| {
            PatchError::erf(
                12,
                format!("Unable to write extracted resource to {destination}: {e}"),
            )
        })
    }

    /// Remove a resource, or cancel a queued addition of it.
    pub fn delete_resource(&mut self, resref: &str, res_type: u16) -> Result<()> {
        if !self.loaded {
            return Err(PatchError::erf(
                4,
                "Unable to delete resource from file, no ERF file is open!",
            ));
        }

        if let Some(index) = self.position_of(resref, res_type) {
            self.resources.remove(index);
            self.dirty = true;
            return Ok(());
        }

        let wanted = format!("{}.{}", resref, res_type_to_extension(res_type)).to_lowercase();
        if let Some(index) = self
            .pending
            .iter()
            .position(|p| text::extract_file_name(&p.source_path).to_lowercase() == wanted)
        {
            self.pending.remove(index);
            return Ok(());
        }

        Err(PatchError::erf(
            8,
            format!("Unable to delete resource from file, resource \"{resref}\" not found!"),
        ))
    }

    /// Queue a file to be added when the archive is saved.
    ///
    /// `save_as` renames the resource; leaving it empty keeps the source
    /// filename. An existing resource of the same name and type is replaced only
    /// when `replace` is set, otherwise this reports an error.
    pub fn add_resource(&mut self, source_path: &str, replace: bool, save_as: &str) -> Result<()> {
        if !self.loaded {
            return Err(PatchError::erf(
                2,
                "Unable to add resource to file, no ERF file is open!",
            ));
        }

        let target_name = if save_as.is_empty() {
            text::extract_file_name(source_path)
        } else {
            save_as.to_string()
        };

        let extension = text::extract_file_ext(&target_name);
        let stem = text::strip_extension(&text::extract_file_name(&target_name)).to_lowercase();
        let ext_no_dot = extension.trim_start_matches('.').to_lowercase();
        let res_type = extension_to_res_type(&extension);

        if res_type == 0xFFFF {
            return Err(PatchError::erf(
                9,
                format!("Cannot add resource \"{stem}\"! Unsupported file type \"{ext_no_dot}\" encountered!"),
            ));
        }

        if self.position_of(&stem, res_type).is_some() {
            if !replace {
                return Err(PatchError::erf(
                    10,
                    format!("Cannot add resource \"{stem}.{ext_no_dot}\"! A file with this name already exists in the ERF!"),
                ));
            }
            self.delete_resource(&stem, res_type)?;
        }

        let source_name = text::extract_file_name(source_path).to_lowercase();
        if let Some(index) = self
            .pending
            .iter()
            .position(|p| text::extract_file_name(&p.source_path).to_lowercase() == source_name)
        {
            if !replace {
                return Err(PatchError::erf(
                    11,
                    format!("Cannot add resource \"{stem}.{ext_no_dot}\"! A file with this name has already been added to the ERF!"),
                ));
            }
            self.pending.remove(index);
        }

        self.pending.push(PendingAdd {
            source_path: source_path.to_string(),
            save_as: target_name,
        });
        self.dirty = true;

        Ok(())
    }

    /// Merge queued additions into the resource list.
    ///
    /// A queued file whose name matches a stored resource replaces it, which is
    /// what happens when both land in the same rebuild.
    fn absorb_pending(&mut self) -> Result<()> {
        let queued = std::mem::take(&mut self.pending);

        for entry in queued {
            let data = fsutil::read_file(&entry.source_path).map_err(|e| {
                PatchError::erf(
                    13,
                    format!(
                        "Unable to read \"{}\" while saving the archive: {e}",
                        entry.source_path
                    ),
                )
            })?;

            let name = text::extract_file_name(&entry.save_as);
            let resref = text::string_to_resref(&text::strip_extension(&name));
            let res_type = extension_to_res_type(&text::extract_file_ext(&name));

            let resource = Resource {
                resref,
                res_type,
                reserved_erf: 0,
                reserved_rim: 0,
                data,
            };

            match self.position_of(&resource.resref, resource.res_type) {
                Some(index) => self.resources[index] = resource,
                None => self.resources.push(resource),
            }
        }

        Ok(())
    }

    /// Serialize the archive, including queued additions.
    pub fn to_bytes(&mut self) -> Result<Vec<u8>> {
        if !self.loaded {
            return Err(PatchError::erf(1, "Unable to save, no ERF file is open!"));
        }

        self.absorb_pending()?;

        // Directory order follows resource name, matching how the original
        // rebuilt archives from a working folder.
        let mut ordered: Vec<&Resource> = self.resources.iter().collect();
        ordered.sort_by_key(|r| r.file_name().to_lowercase());

        if self.kind.is_rim() {
            Ok(self.write_rim(&ordered))
        } else {
            Ok(self.write_erf(&ordered))
        }
    }

    fn write_rim(&self, ordered: &[&Resource]) -> Vec<u8> {
        let entry_count = ordered.len() as u32;
        let key_offset = RIM_HEADER_SIZE;
        let data_start = key_offset + RIM_KEY_ENTRY_SIZE * entry_count;

        let total: usize =
            data_start as usize + ordered.iter().map(|r| r.data.len()).sum::<usize>();
        let mut out = vec![0u8; total];

        out[0..4].copy_from_slice(self.kind.tag());
        out[4..8].copy_from_slice(b"V1.0");
        put_u32(&mut out, 8, self.rim_unknown);
        put_u32(&mut out, 12, entry_count);
        put_u32(&mut out, 16, key_offset);

        let mut data_at = data_start;
        for (i, resource) in ordered.iter().enumerate() {
            let key_at = (key_offset + RIM_KEY_ENTRY_SIZE * i as u32) as usize;

            out[key_at..key_at + 16].copy_from_slice(&latin1::encode_fixed::<16>(&resource.resref));
            put_u16(&mut out, key_at + 16, resource.res_type);
            put_u16(&mut out, key_at + 18, resource.reserved_erf);
            put_u16(&mut out, key_at + 20, i as u16);
            put_u16(&mut out, key_at + 22, resource.reserved_rim);
            put_u32(&mut out, key_at + 24, data_at);
            put_u32(&mut out, key_at + 28, resource.data.len() as u32);

            let start = data_at as usize;
            out[start..start + resource.data.len()].copy_from_slice(&resource.data);
            data_at += resource.data.len() as u32;
        }

        out
    }

    fn write_erf(&self, ordered: &[&Resource]) -> Vec<u8> {
        let entry_count = ordered.len() as u32;

        let loc_string_size: u32 = self
            .loc_strings
            .iter()
            .map(|s| latin1::byte_len(&s.text) as u32 + 8)
            .sum();

        let loc_string_offset = ERF_HEADER_SIZE;
        let key_offset = loc_string_offset + loc_string_size;
        let res_offset = key_offset + ERF_KEY_ENTRY_SIZE * entry_count;
        let data_start = res_offset + ERF_RES_ENTRY_SIZE * entry_count;

        let total: usize =
            data_start as usize + ordered.iter().map(|r| r.data.len()).sum::<usize>();
        let mut out = vec![0u8; total];

        let (year, day) = build_stamp();

        out[0..4].copy_from_slice(self.kind.tag());
        out[4..8].copy_from_slice(b"V1.0");
        put_u32(&mut out, 8, self.loc_strings.len() as u32);
        put_u32(&mut out, 12, loc_string_size);
        put_u32(&mut out, 16, entry_count);
        put_u32(&mut out, 20, loc_string_offset);
        put_u32(&mut out, 24, key_offset);
        put_u32(&mut out, 28, res_offset);
        put_u32(&mut out, 32, year);
        put_u32(&mut out, 36, day);
        put_u32(&mut out, 40, self.loc_string_strref);

        let mut cursor = loc_string_offset as usize;
        for entry in &self.loc_strings {
            let encoded = latin1::encode(&entry.text);
            put_u32(&mut out, cursor, entry.language_id);
            put_u32(&mut out, cursor + 4, encoded.len() as u32);
            cursor += 8;
            out[cursor..cursor + encoded.len()].copy_from_slice(&encoded);
            cursor += encoded.len();
        }

        let mut data_at = data_start;
        for (i, resource) in ordered.iter().enumerate() {
            let key_at = (key_offset + ERF_KEY_ENTRY_SIZE * i as u32) as usize;
            out[key_at..key_at + 16].copy_from_slice(&latin1::encode_fixed::<16>(&resource.resref));
            put_u32(&mut out, key_at + 16, i as u32);
            put_u16(&mut out, key_at + 20, resource.res_type);
            put_u16(&mut out, key_at + 22, resource.reserved_erf);

            let res_at = (res_offset + ERF_RES_ENTRY_SIZE * i as u32) as usize;
            put_u32(&mut out, res_at, data_at);
            put_u32(&mut out, res_at + 4, resource.data.len() as u32);

            let start = data_at as usize;
            out[start..start + resource.data.len()].copy_from_slice(&resource.data);
            data_at += resource.data.len() as u32;
        }

        out
    }

    /// Write the archive to disk.
    ///
    /// An unchanged archive saved to its own path is left alone, so opening an
    /// archive without editing it never rewrites the file.
    pub fn save(&mut self, path: Option<&str>) -> Result<()> {
        if !self.loaded {
            return Err(PatchError::erf(1, "Unable to save, no ERF file is open!"));
        }

        if path.is_none() && !self.dirty {
            return Ok(());
        }

        if let Some(target) = path {
            self.path = target.to_string();
        }

        if self.path.is_empty() {
            return Err(PatchError::erf(
                16,
                "Unable to save, no file name has been specified!",
            ));
        }

        let bytes = self.to_bytes()?;
        let target = self.path.clone();
        fsutil::write_file(&target, &bytes)
            .map_err(|e| PatchError::erf(15, format!("Unable to write archive {target}: {e}")))?;

        self.dirty = false;
        Ok(())
    }
}

/// Years since 1900 and days since January 1, as the header records them.
fn build_stamp() -> (u32, u32) {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let days = seconds.div_euclid(86_400);
    let (year, day_of_year) = civil_from_days(days);

    ((year - 1900).max(0) as u32, day_of_year)
}

/// Convert days since the Unix epoch into a year and zero-based day of year.
fn civil_from_days(days: i64) -> (i64, u32) {
    // Shift to an era starting 0000-03-01 so leap years fall at the end.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };

    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let cumulative: [i64; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    let mut day_of_year = cumulative[(m - 1) as usize] + d - 1;
    if leap && m > 2 {
        day_of_year += 1;
    }

    (year, day_of_year.max(0) as u32)
}

fn read_u32(bytes: &[u8], at: u32) -> Result<u32> {
    let at = at as usize;
    if at + 4 > bytes.len() {
        return Err(PatchError::erf(
            6,
            "Archive ends before its directory does.",
        ));
    }
    Ok(u32::from_le_bytes([
        bytes[at],
        bytes[at + 1],
        bytes[at + 2],
        bytes[at + 3],
    ]))
}

fn read_u16(bytes: &[u8], at: u32) -> Result<u16> {
    let at = at as usize;
    if at + 2 > bytes.len() {
        return Err(PatchError::erf(
            6,
            "Archive ends before its directory does.",
        ));
    }
    Ok(u16::from_le_bytes([bytes[at], bytes[at + 1]]))
}

fn read_slice(bytes: &[u8], at: u32, len: u32) -> Result<&[u8]> {
    let start = at as usize;
    let end = start + len as usize;
    if end > bytes.len() {
        return Err(PatchError::erf(6, "Archive ends before its data does."));
    }
    Ok(&bytes[start..end])
}

fn put_u32(out: &mut [u8], at: usize, value: u32) {
    out[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u16(out: &mut [u8], at: usize, value: u16) {
    out[at..at + 2].copy_from_slice(&value.to_le_bytes());
}

/// Mapping between resource type numbers and file extensions.
const RES_TYPES: &[(u16, &str)] = &[
    (0x0000, "res"),
    (0x0001, "bmp"),
    (0x0002, "mve"),
    (0x0003, "tga"),
    (0x0004, "wav"),
    (0x0006, "plt"),
    (0x0007, "ini"),
    (0x0008, "mp3"),
    (0x0009, "mpg"),
    (0x000A, "txt"),
    (0x000B, "wma"),
    (0x000C, "wmv"),
    (0x000D, "xmv"),
    (0x07D0, "plh"),
    (0x07D1, "tex"),
    (0x07D2, "mdl"),
    (0x07D3, "thg"),
    (0x07D5, "fnt"),
    (0x07D7, "lua"),
    (0x07D8, "slt"),
    (0x07D9, "nss"),
    (0x07DA, "ncs"),
    (0x07DB, "mod"),
    (0x07DC, "are"),
    (0x07DD, "set"),
    (0x07DE, "ifo"),
    (0x07DF, "bic"),
    (0x07E0, "wok"),
    (0x07E1, "2da"),
    (0x07E2, "tlk"),
    (0x07E6, "txi"),
    (0x07E7, "git"),
    (0x07E8, "bti"),
    (0x07E9, "uti"),
    (0x07EA, "btc"),
    (0x07EB, "utc"),
    (0x07ED, "dlg"),
    (0x07EE, "itp"),
    (0x07EF, "btt"),
    (0x07F0, "utt"),
    (0x07F1, "dds"),
    (0x07F2, "bts"),
    (0x07F3, "uts"),
    (0x07F4, "ltr"),
    (0x07F5, "gff"),
    (0x07F6, "fac"),
    (0x07F7, "bte"),
    (0x07F8, "ute"),
    (0x07F9, "btd"),
    (0x07FA, "utd"),
    (0x07FB, "btp"),
    (0x07FC, "utp"),
    (0x07FD, "dft"),
    (0x07FE, "gic"),
    (0x07FF, "gui"),
    (0x0800, "css"),
    (0x0801, "ccs"),
    (0x0802, "btm"),
    (0x0803, "utm"),
    (0x0804, "dwk"),
    (0x0805, "pwk"),
    (0x0806, "btg"),
    (0x0807, "utg"),
    (0x0808, "jrl"),
    (0x0809, "sav"),
    (0x080A, "utw"),
    (0x080B, "4pc"),
    (0x080C, "ssf"),
    (0x080D, "hak"),
    (0x080E, "nwm"),
    (0x080F, "bik"),
    (0x0BB8, "lyt"),
    (0x0BB9, "vis"),
    (0x0BBA, "rim"),
    (0x0BBB, "pth"),
    (0x0BBC, "lip"),
    (0x0BBD, "bwm"),
    (0x0BBE, "txb"),
    (0x0BBF, "tpc"),
    (0x0BC0, "mdx"),
    (0x0BC1, "rsv"),
    (0x0BC2, "sig"),
    (0x0BC3, "xbx"),
    (0x270D, "erf"),
    (0x270E, "bif"),
    (0x270F, "key"),
];

/// File extension for a resource type number, without the leading dot.
pub fn res_type_to_extension(res_type: u16) -> &'static str {
    RES_TYPES
        .iter()
        .find(|(code, _)| *code == res_type)
        .map(|(_, ext)| *ext)
        .unwrap_or("")
}

/// Resource type number for an extension, with or without the leading dot.
///
/// Returns `0xFFFF` for anything unrecognised.
pub fn extension_to_res_type(extension: &str) -> u16 {
    let cleaned = extension.trim_start_matches('.').to_lowercase();
    RES_TYPES
        .iter()
        .find(|(_, ext)| *ext == cleaned)
        .map(|(code, _)| *code)
        .unwrap_or(0xFFFF)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("odypatcher-erf-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn archive_with(kind: ArchiveKind) -> ErfFile {
        let mut archive = ErfFile::create("test.mod", kind);
        archive.resources.push(Resource {
            resref: "alpha".into(),
            res_type: extension_to_res_type("uti"),
            reserved_erf: 0,
            reserved_rim: 0,
            data: b"first payload".to_vec(),
        });
        archive.resources.push(Resource {
            resref: "beta".into(),
            res_type: extension_to_res_type("ncs"),
            reserved_erf: 0,
            reserved_rim: 0,
            data: b"second".to_vec(),
        });
        archive.dirty = true;
        archive
    }

    #[test]
    fn extension_mapping_is_reversible() {
        assert_eq!(extension_to_res_type(".uti"), 0x07E9);
        assert_eq!(extension_to_res_type("UTI"), 0x07E9);
        assert_eq!(res_type_to_extension(0x07E9), "uti");
        assert_eq!(extension_to_res_type(".unknown"), 0xFFFF);
        assert_eq!(res_type_to_extension(0xFFFF), "");
    }

    #[test]
    fn erf_round_trips_through_bytes() {
        let mut original = archive_with(ArchiveKind::Mod);
        let bytes = original.to_bytes().unwrap();
        let reloaded = ErfFile::parse(&bytes, "test.mod").unwrap();

        assert_eq!(reloaded.kind, ArchiveKind::Mod);
        assert_eq!(reloaded.count(), 2);
        assert_eq!(
            reloaded
                .resource_data("alpha", extension_to_res_type("uti"))
                .unwrap(),
            b"first payload"
        );
        assert_eq!(
            reloaded
                .resource_data("beta", extension_to_res_type("ncs"))
                .unwrap(),
            b"second"
        );
    }

    #[test]
    fn rim_round_trips_through_bytes() {
        let mut original = archive_with(ArchiveKind::Rim);
        let bytes = original.to_bytes().unwrap();
        let reloaded = ErfFile::parse(&bytes, "test.rim").unwrap();

        assert_eq!(reloaded.kind, ArchiveKind::Rim);
        assert_eq!(reloaded.count(), 2);
        assert_eq!(
            reloaded
                .resource_data("alpha", extension_to_res_type("uti"))
                .unwrap(),
            b"first payload"
        );
    }

    #[test]
    fn headers_use_the_documented_offsets() {
        let erf = archive_with(ArchiveKind::Erf).to_bytes().unwrap();
        assert_eq!(&erf[0..4], b"ERF ");
        assert_eq!(&erf[4..8], b"V1.0");
        assert_eq!(u32::from_le_bytes(erf[20..24].try_into().unwrap()), 160);

        let rim = archive_with(ArchiveKind::Rim).to_bytes().unwrap();
        assert_eq!(&rim[0..4], b"RIM ");
        assert_eq!(u32::from_le_bytes(rim[16..20].try_into().unwrap()), 120);
    }

    #[test]
    fn saving_twice_produces_the_same_directory() {
        let mut archive = archive_with(ArchiveKind::Mod);
        let first = archive.to_bytes().unwrap();

        let mut reloaded = ErfFile::parse(&first, "test.mod").unwrap();
        reloaded.dirty = true;
        let second = reloaded.to_bytes().unwrap();

        // Build stamps differ by design; the directory and payloads must not.
        assert_eq!(first.len(), second.len());
        assert_eq!(first[44..], second[44..]);
    }

    #[test]
    fn description_strings_survive() {
        let mut archive = archive_with(ArchiveKind::Mod);
        archive.loc_strings.push(LocString {
            language_id: 0,
            text: "A test module".into(),
        });

        let bytes = archive.to_bytes().unwrap();
        let reloaded = ErfFile::parse(&bytes, "test.mod").unwrap();

        assert_eq!(reloaded.loc_strings.len(), 1);
        assert_eq!(reloaded.loc_strings[0].text, "A test module");
        assert_eq!(reloaded.loc_strings[0].language_id, 0);
    }

    #[test]
    fn queued_files_are_added_on_save() {
        let dir = temp_dir("add");
        let source = format!("{}\\gamma.ncs", dir.to_string_lossy());
        fsutil::write_file(&source, b"compiled").unwrap();

        let mut archive = archive_with(ArchiveKind::Mod);
        archive.add_resource(&source, false, "").unwrap();
        assert_eq!(archive.pending_count(), 1);

        let bytes = archive.to_bytes().unwrap();
        let reloaded = ErfFile::parse(&bytes, "test.mod").unwrap();

        assert_eq!(reloaded.count(), 3);
        assert_eq!(
            reloaded
                .resource_data("gamma", extension_to_res_type("ncs"))
                .unwrap(),
            b"compiled"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_queued_file_can_be_renamed() {
        let dir = temp_dir("rename");
        let source = format!("{}\\source.ncs", dir.to_string_lossy());
        fsutil::write_file(&source, b"payload").unwrap();

        let mut archive = archive_with(ArchiveKind::Mod);
        archive.add_resource(&source, false, "renamed.ncs").unwrap();

        let bytes = archive.to_bytes().unwrap();
        let reloaded = ErfFile::parse(&bytes, "test.mod").unwrap();
        assert!(reloaded
            .resource_data("renamed", extension_to_res_type("ncs"))
            .is_ok());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn adding_an_existing_name_needs_replace() {
        let dir = temp_dir("replace");
        let source = format!("{}\\alpha.uti", dir.to_string_lossy());
        fsutil::write_file(&source, b"new content").unwrap();

        let mut archive = archive_with(ArchiveKind::Mod);
        assert_eq!(
            archive.add_resource(&source, false, "").unwrap_err().code(),
            10
        );

        archive.add_resource(&source, true, "").unwrap();
        let bytes = archive.to_bytes().unwrap();
        let reloaded = ErfFile::parse(&bytes, "test.mod").unwrap();

        assert_eq!(reloaded.count(), 2);
        assert_eq!(
            reloaded
                .resource_data("alpha", extension_to_res_type("uti"))
                .unwrap(),
            b"new content"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unsupported_extensions_are_refused() {
        let mut archive = archive_with(ArchiveKind::Mod);
        let err = archive.add_resource("thing.zzz", false, "").unwrap_err();
        assert_eq!(err.code(), 9);
    }

    #[test]
    fn existence_checks_can_ignore_queued_files() {
        let dir = temp_dir("exists");
        let source = format!("{}\\gamma.ncs", dir.to_string_lossy());
        fsutil::write_file(&source, b"x").unwrap();

        let mut archive = archive_with(ArchiveKind::Mod);
        archive.add_resource(&source, false, "").unwrap();

        assert!(!archive.resource_exists("gamma.ncs", false).unwrap());
        assert!(archive.resource_exists("gamma.ncs", true).unwrap());
        assert!(archive.resource_exists("alpha.uti", false).unwrap());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn deleting_removes_a_stored_resource() {
        let mut archive = archive_with(ArchiveKind::Mod);
        archive
            .delete_resource("alpha", extension_to_res_type("uti"))
            .unwrap();
        assert_eq!(archive.count(), 1);
        assert_eq!(
            archive
                .delete_resource("missing", extension_to_res_type("uti"))
                .unwrap_err()
                .code(),
            8
        );
    }

    #[test]
    fn unchanged_archives_are_not_rewritten() {
        let dir = temp_dir("clean");
        let path = format!("{}\\clean.mod", dir.to_string_lossy());

        let mut archive = archive_with(ArchiveKind::Mod);
        archive.save(Some(&path)).unwrap();

        let before = fs::metadata(fsutil::native_path(&path)).unwrap().len();
        let mut reopened = ErfFile::load(&path).unwrap();
        reopened.save(None).unwrap();
        let after = fs::metadata(fsutil::native_path(&path)).unwrap().len();

        assert_eq!(before, after);
        assert!(!reopened.is_dirty());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn archive_detection_checks_the_header() {
        let dir = temp_dir("detect");
        let good = format!("{}\\good.mod", dir.to_string_lossy());
        let bad = format!("{}\\bad.txt", dir.to_string_lossy());

        let bytes = archive_with(ArchiveKind::Mod).to_bytes().unwrap();
        fsutil::write_file(&good, &bytes).unwrap();
        fsutil::write_file(&bad, b"not an archive at all").unwrap();

        assert!(ErfFile::is_valid_archive(&good));
        assert!(!ErfFile::is_valid_archive(&bad));
        assert!(!ErfFile::is_valid_archive("/no/such/file.mod"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn resource_names_are_filtered_to_the_allowed_characters() {
        let dir = temp_dir("filter");
        let source = format!("{}\\odd.ncs", dir.to_string_lossy());
        fsutil::write_file(&source, b"x").unwrap();

        let mut archive = ErfFile::create("test.mod", ArchiveKind::Mod);
        archive
            .add_resource(&source, false, "my file!.ncs")
            .unwrap();

        let bytes = archive.to_bytes().unwrap();
        let reloaded = ErfFile::parse(&bytes, "test.mod").unwrap();
        // The space goes, the bang stays. Delphi's StringToResRef drops `!`,
        // but real script resrefs carry it (`k_pkor_!knexcav`) and stripping
        // it would leave the resource unfindable. See `text::string_to_resref`.
        assert!(reloaded
            .resource_data("myfile!", extension_to_res_type("ncs"))
            .is_ok());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn build_stamp_is_plausible() {
        let (year, day) = build_stamp();
        // Years since 1900, so anything current is well past 100.
        assert!(year > 100, "unexpected year offset {year}");
        assert!(day < 366, "unexpected day of year {day}");
    }

    #[test]
    fn day_of_year_handles_leap_years() {
        // 2024-03-01 is day 60 in a leap year (January 1 is day 0).
        let days = 19_783; // 2024-03-01
        let (year, doy) = civil_from_days(days);
        assert_eq!(year, 2024);
        assert_eq!(doy, 60);
    }
}
