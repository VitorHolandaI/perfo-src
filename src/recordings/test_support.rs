//! Fixtures the recording tests share: a throwaway store on disk, and a way
//! to age a file so the sidecar freshness checks can be exercised.

use std::fs;
use std::path::{Path, PathBuf};

/// A recording plus its sidecar, in a directory of its own so the tests
/// stay independent.
pub(super) fn recording_fixture(name: &str, body: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("perfo-timeline-{name}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create fixture dir");
    let rec = dir.join("rec.json");
    fs::write(&rec, body).expect("write recording");
    (rec, dir.join("rec.offsets.bin"))
}

/// Backdates a file's mtime. std has no setter, so go through utimensat.
pub(super) fn filetime_set(path: &Path, when: std::time::SystemTime) {
    let secs = when
        .duration_since(std::time::UNIX_EPOCH)
        .expect("after the epoch")
        .as_secs() as i64;
    let times = [
        libc::timespec {
            tv_sec: secs,
            tv_nsec: 0,
        },
        libc::timespec {
            tv_sec: secs,
            tv_nsec: 0,
        },
    ];
    let c_path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes())
        .expect("path has no interior nul");
    // SAFETY: c_path is a valid nul-terminated path and times is a 2-element
    // timespec array, which is exactly what utimensat documents.
    let rc = unsafe { libc::utimensat(libc::AT_FDCWD, c_path.as_ptr(), times.as_ptr(), 0) };
    assert_eq!(rc, 0, "utimensat failed on {path:?}");
}
