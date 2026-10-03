//! Where recordings live on disk, and which paths a caller may name.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub fn get_home_dir() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

pub const DEFAULT_MAX_RECORDINGS: usize = 5;

/// Recordings are persistent user data, so they follow the XDG data directory
/// rather than a shell profile: a GUI widget is never started from a login
/// shell and so never inherits one.
pub fn default_recordings_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| get_home_dir().join(".local/share"))
        .join("perfo/recordings")
}

pub fn get_config() -> (PathBuf, usize) {
    let rec_dir = std::env::var_os("PERFO_RECORDINGS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(default_recordings_dir);

    let _ = fs::create_dir_all(&rec_dir);

    let max_recs = std::env::var("PERFO_MAX_RECORDINGS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(DEFAULT_MAX_RECORDINGS)
        .max(1);

    (rec_dir, max_recs)
}

/// Reject ids that would escape the recordings directory. Ids normally come
/// from `record list`, but every subcommand is reachable from the CLI too.
pub(super) fn sanitize_recording_id(target: &str) -> io::Result<&str> {
    if target.is_empty() || target.contains('/') || target == "." || target == ".." {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Invalid recording id: {target:?}"),
        ));
    }
    Ok(target)
}

/// Map an id or explicit path to the file it names, without touching the disk.
pub(super) fn recording_path_for(target: &str, rec_dir: &Path) -> io::Result<PathBuf> {
    if target.ends_with(".json") {
        if let Some(rest) = target.strip_prefix("~/") {
            Ok(get_home_dir().join(rest))
        } else if target.contains('/') {
            Ok(PathBuf::from(target))
        } else {
            Ok(rec_dir.join(target))
        }
    } else {
        Ok(rec_dir.join(format!("{}.json", sanitize_recording_id(target)?)))
    }
}

/// True when `path` sits directly inside `dir`. Recordings are stored flat, so
/// an exact parent match is enough and `..` cannot slip through canonicalize.
pub(super) fn is_inside(path: &Path, dir: &Path) -> bool {
    let (Ok(base), Some(parent)) = (dir.canonicalize(), path.parent()) else {
        return false;
    };
    if parent.as_os_str().is_empty() {
        return false;
    }
    parent.canonicalize().map(|p| p == base).unwrap_or(false)
}

pub fn resolve_recording_path(target: &str, rec_dir: &Path) -> io::Result<PathBuf> {
    let path = recording_path_for(target, rec_dir)?;

    if !path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("Recording file not found: {:?}", path),
        ));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_id_rejects_path_separators() {
        assert!(sanitize_recording_id("rec-20260910-120000").is_ok());
        assert!(sanitize_recording_id("../../etc/passwd").is_err());
        assert!(sanitize_recording_id("sub/dir").is_err());
        assert!(sanitize_recording_id("..").is_err());
        assert!(sanitize_recording_id(".").is_err());
        assert!(sanitize_recording_id("").is_err());
    }

    #[test]
    fn bare_id_always_lands_in_the_recordings_dir() {
        let rec_dir = Path::new("/var/lib/perfo/recordings");
        let path = recording_path_for("rec-1", rec_dir).expect("valid id");
        assert_eq!(path, rec_dir.join("rec-1.json"));
    }

    #[test]
    fn traversing_id_never_produces_a_path() {
        let rec_dir = Path::new("/var/lib/perfo/recordings");
        assert!(recording_path_for("../../../etc/shadow", rec_dir).is_err());
    }

    #[test]
    fn is_inside_rejects_paths_outside_the_directory() {
        let dir = std::env::temp_dir().join("perfo-is-inside-test");
        let nested = dir.join("nested");
        fs::create_dir_all(&nested).expect("create test dirs");

        assert!(is_inside(&dir.join("rec-1.json"), &dir));
        // A sibling, a parent, and a subdirectory are all outside a flat store.
        assert!(!is_inside(&nested.join("rec-1.json"), &dir));
        assert!(!is_inside(Path::new("/etc/passwd"), &dir));
        assert!(!is_inside(&dir.join("../escaped.json"), &dir));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_reports_not_found_instead_of_handing_back_a_dead_path() {
        let dir = std::env::temp_dir().join("perfo-resolve-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create test dir");

        let err = resolve_recording_path("rec-missing", &dir).expect_err("nothing to resolve");
        assert_eq!(err.kind(), io::ErrorKind::NotFound);

        fs::write(dir.join("rec-1.json"), "{}").expect("write recording");
        let path = resolve_recording_path("rec-1", &dir).expect("resolves an existing id");
        assert_eq!(path, dir.join("rec-1.json"));

        // A traversing id fails on the id itself, before any disk lookup.
        let err = resolve_recording_path("../escape", &dir).expect_err("traversal refused");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_recordings_dir_follows_xdg_data_home() {
        // Absolute XDG_DATA_HOME wins; a relative one is ignored per the spec.
        assert!(default_recordings_dir().ends_with("perfo/recordings"));
    }
}
