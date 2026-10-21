//! Filesystem access that behaves the same on every supported platform.
//!
//! Instruction files were written on Windows, so they use `\` separators and
//! assume filename lookups ignore case. macOS and Linux need both of those
//! translated: separators are converted, and names are matched case-insensitively
//! against what is actually on disk. Without that, a mod asking for
//! `Override\Appearance.2da` would silently miss `override/appearance.2da`.

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

/// Convert a configuration-style path into one this platform can open.
///
/// Backslashes become native separators, then each component is matched against
/// existing directory entries so differences in letter case do not matter.
pub fn native_path(path: &str) -> PathBuf {
    let swapped: String = if std::path::MAIN_SEPARATOR == '\\' {
        path.to_string()
    } else {
        path.replace('\\', "/")
    };
    resolve_existing(Path::new(&swapped))
}

/// Resolve a path against what exists on disk, ignoring case differences.
///
/// Components that do not exist are kept exactly as written, so this is safe to
/// use for files about to be created.
pub fn resolve_existing(path: &Path) -> PathBuf {
    if cfg!(windows) || path.exists() {
        return path.to_path_buf();
    }

    let mut resolved = PathBuf::new();
    let mut matched_so_far = true;

    for component in path.components() {
        match component {
            Component::Normal(name) => {
                if matched_so_far {
                    match case_insensitive_child(&resolved, name.to_string_lossy().as_ref()) {
                        Some(actual) => resolved.push(actual),
                        None => {
                            // Nothing on disk matches; keep the requested spelling
                            // and stop probing for the remaining components.
                            matched_so_far = false;
                            resolved.push(name);
                        }
                    }
                } else {
                    resolved.push(name);
                }
            }
            other => resolved.push(other.as_os_str()),
        }
    }

    resolved
}

/// Find a directory entry whose name equals `wanted` ignoring case.
fn case_insensitive_child(parent: &Path, wanted: &str) -> Option<String> {
    let dir = if parent.as_os_str().is_empty() {
        Path::new(".")
    } else {
        parent
    };

    let entries = fs::read_dir(dir).ok()?;
    let mut fallback = None;

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == wanted {
            return Some(name);
        }
        if fallback.is_none() && name.eq_ignore_ascii_case(wanted) {
            fallback = Some(name);
        }
    }

    fallback
}

/// True when the path names an existing file.
pub fn file_exists(path: &str) -> bool {
    native_path(path).is_file()
}

/// True when the path names an existing directory.
pub fn dir_exists(path: &str) -> bool {
    native_path(path).is_dir()
}

/// Create a directory and any missing parents.
pub fn force_directories(path: &str) -> io::Result<()> {
    let target = native_path(path);
    if target.as_os_str().is_empty() {
        return Ok(());
    }
    fs::create_dir_all(target)
}

/// Clear the read-only attribute so a file can be overwritten.
pub fn make_writable(path: &str) {
    let target = native_path(path);
    if let Ok(metadata) = fs::metadata(&target) {
        let mut perms = metadata.permissions();
        if perms.readonly() {
            #[allow(clippy::permissions_set_readonly_false)]
            perms.set_readonly(false);
            let _ = fs::set_permissions(&target, perms);
        }
    }
}

/// True when the file exists and is marked read-only.
pub fn is_write_protected(path: &str) -> bool {
    let target = native_path(path);
    fs::metadata(&target)
        .map(|m| m.permissions().readonly())
        .unwrap_or(false)
}

/// Copy a file, creating the destination directory if needed.
pub fn copy_file(source: &str, destination: &str) -> io::Result<()> {
    let src = native_path(source);
    let dst = native_path(destination);

    if let Some(parent) = dst.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            fs::create_dir_all(parent)?;
        }
    }

    if dst.exists() {
        make_writable(destination);
    }

    replace_with(&dst, |scratch| {
        fs::copy(&src, scratch)?;
        Ok(())
    })
}

/// Put new content in place of a file by writing it beside the target and
/// moving it over, rather than opening the target and writing through it.
///
/// Writing through a file keeps the same underlying file, so anything else
/// pointing at it — a hard link, and therefore a snapshot taken with one — sees
/// the new content too, and a run that stops halfway leaves a half-written
/// game file behind. Moving a finished file into place gives the folder a new
/// file, leaves any other link on the old content, and is all-or-nothing.
///
/// The scratch file sits beside the target so the move stays within one
/// filesystem. If it cannot be moved into place, the write is done directly as
/// before rather than failing.
fn replace_with<F>(target: &Path, fill: F) -> io::Result<()>
where
    F: Fn(&Path) -> io::Result<()>,
{
    let scratch = match scratch_path(target) {
        Some(path) => path,
        None => return fill(target),
    };

    if let Err(err) = fill(&scratch) {
        let _ = fs::remove_file(&scratch);
        return Err(err);
    }

    if let Some(mode) = existing_permissions(target) {
        let _ = fs::set_permissions(&scratch, mode);
    }

    match fs::rename(&scratch, target) {
        Ok(()) => Ok(()),
        Err(_) => {
            let outcome = fill(target);
            let _ = fs::remove_file(&scratch);
            outcome
        }
    }
}

/// Name a scratch file beside the target, unused at the time of asking.
fn scratch_path(target: &Path) -> Option<PathBuf> {
    let parent = target.parent()?;
    let name = target.file_name()?.to_string_lossy().to_string();
    let stamp = std::process::id();

    for attempt in 0..100u32 {
        let candidate = parent.join(format!(".{name}.odypatcher-{stamp}-{attempt}"));
        if !candidate.exists() {
            return Some(candidate);
        }
    }

    None
}

/// The permissions of a file that is about to be replaced, if it has any.
fn existing_permissions(target: &Path) -> Option<fs::Permissions> {
    fs::metadata(target).ok().map(|m| m.permissions())
}

/// Delete a file. Missing files are not an error.
pub fn delete_file(path: &str) -> bool {
    let target = native_path(path);
    if !target.exists() {
        return false;
    }
    make_writable(path);
    fs::remove_file(target).is_ok()
}

/// Rename a file within the same directory tree.
pub fn rename_file(from: &str, to: &str) -> bool {
    let src = native_path(from);
    let dst = native_path(to);
    if !src.exists() || dst.exists() {
        return false;
    }
    fs::rename(src, dst).is_ok()
}

/// Remove a directory if it is empty.
pub fn remove_dir(path: &str) -> bool {
    fs::remove_dir(native_path(path)).is_ok()
}

/// Delete every file in a directory, then the directory itself.
///
/// Refuses to touch very short paths, so a malformed setting cannot aim this at
/// a drive root.
pub fn delete_folder(path: &str, recursive: bool) -> bool {
    let target = native_path(path);
    if path.len() < 5 || !target.is_dir() {
        return false;
    }

    let Ok(entries) = fs::read_dir(&target) else {
        return false;
    };

    for entry in entries.flatten() {
        let child = entry.path();
        if child.is_dir() {
            if recursive {
                delete_folder(&child.to_string_lossy(), true);
            }
        } else {
            let _ = fs::remove_file(child);
        }
    }

    fs::remove_dir(target).is_ok()
}

/// List the files (not directories) directly inside a folder, sorted by name.
///
/// Sorting keeps archive rebuilds reproducible across platforms, which raw
/// directory order would not.
pub fn files_in_folder(path: &str) -> Vec<PathBuf> {
    let target = native_path(path);
    let Ok(entries) = fs::read_dir(target) else {
        return Vec::new();
    };

    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();

    files.sort();
    files
}

/// Read a whole file into memory.
pub fn read_file(path: &str) -> io::Result<Vec<u8>> {
    fs::read(native_path(path))
}

/// Write a whole file, creating parent directories and clearing read-only.
pub fn write_file(path: &str, data: &[u8]) -> io::Result<()> {
    let target = native_path(path);
    if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            fs::create_dir_all(parent)?;
        }
    }
    if target.exists() {
        make_writable(path);
    }
    replace_with(&target, |scratch| fs::write(scratch, data))
}

/// Join a directory and a name using the platform separator.
pub fn join(dir: &str, name: &str) -> String {
    crate::text::join_path(dir, name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("odypatcher-fsutil-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn writing_gives_the_folder_a_new_file_and_leaves_other_links_alone() {
        let dir = temp_dir("replace");
        let target = dir.join("archive.mod");
        let link = dir.join("snapshot.mod");

        fs::write(&target, b"before").unwrap();
        fs::hard_link(&target, &link).unwrap();

        write_file(&target.to_string_lossy(), b"after").unwrap();

        // The folder holds the new content, and the other link still holds the
        // old — which is what a snapshot taken with hard links relies on.
        assert_eq!(fs::read(&target).unwrap(), b"after");
        assert_eq!(fs::read(&link).unwrap(), b"before");

        // Nothing is left beside it.
        let leftovers: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|name| name != "archive.mod" && name != "snapshot.mod")
            .collect();
        assert!(leftovers.is_empty(), "left behind {leftovers:?}");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn copying_over_a_file_also_leaves_other_links_alone() {
        let dir = temp_dir("replace-copy");
        let source = dir.join("new.mod");
        let target = dir.join("archive.mod");
        let link = dir.join("snapshot.mod");

        fs::write(&source, b"after").unwrap();
        fs::write(&target, b"before").unwrap();
        fs::hard_link(&target, &link).unwrap();

        copy_file(&source.to_string_lossy(), &target.to_string_lossy()).unwrap();

        assert_eq!(fs::read(&target).unwrap(), b"after");
        assert_eq!(fs::read(&link).unwrap(), b"before");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn joins_with_the_platform_separator() {
        use crate::text::SEPARATOR;
        assert_eq!(join("dir", "file.2da"), format!("dir{SEPARATOR}file.2da"));
        // A path that already ends with a separator keeps the one it has.
        assert_eq!(join("dir\\", "file.2da"), "dir\\file.2da");
    }

    #[test]
    fn finds_files_regardless_of_case() {
        let dir = temp_dir("case");
        fs::write(dir.join("appearance.2da"), b"x").unwrap();

        let requested = format!("{}\\Appearance.2DA", dir.to_string_lossy());
        assert!(file_exists(&requested));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn keeps_spelling_for_paths_that_do_not_exist_yet() {
        let dir = temp_dir("new");
        let requested = format!("{}\\NewFile.txt", dir.to_string_lossy());

        write_file(&requested, b"data").unwrap();
        assert!(dir.join("NewFile.txt").is_file());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn copies_and_deletes_files() {
        let dir = temp_dir("copy");
        let src = format!("{}\\src.bin", dir.to_string_lossy());
        let dst = format!("{}\\sub\\dst.bin", dir.to_string_lossy());

        write_file(&src, b"payload").unwrap();
        copy_file(&src, &dst).unwrap();
        assert_eq!(read_file(&dst).unwrap(), b"payload");

        assert!(delete_file(&dst));
        assert!(!file_exists(&dst));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn folder_listing_is_sorted() {
        let dir = temp_dir("list");
        for name in ["c.txt", "a.txt", "b.txt"] {
            fs::write(dir.join(name), b"x").unwrap();
        }
        fs::create_dir(dir.join("subdir")).unwrap();

        let listed: Vec<String> = files_in_folder(&dir.to_string_lossy())
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();

        assert_eq!(listed, vec!["a.txt", "b.txt", "c.txt"]);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn refuses_to_delete_suspiciously_short_paths() {
        assert!(!delete_folder("/", true));
        assert!(!delete_folder("C:\\", true));
    }
}
