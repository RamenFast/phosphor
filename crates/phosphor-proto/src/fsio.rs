// SPDX-License-Identifier: GPL-3.0-or-later
//! Atomic file writes for the formats this crate owns. One policy,
//! decided once: temp file in the SAME directory (rename is only
//! atomic within a filesystem), `sync_all` before the rename so a
//! crash never leaves a truncated settings/kit file where a good one
//! used to be.

use std::io::Write;
use std::path::Path;

/// Write `contents` to `path` atomically: unique temp sibling →
/// write → fsync → rename over the target. The temp file is cleaned
/// up on any failure.
pub fn write_atomic(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let directory = path.parent().filter(|p| !p.as_os_str().is_empty());
    let directory = directory.unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(directory)?;
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file");
    let temp_path = directory.join(format!(
        ".{file_name}.tmp.{}",
        std::process::id()
    ));
    let result = (|| {
        let mut file = std::fs::File::create(&temp_path)?;
        file.write_all(contents)?;
        file.sync_all()?;
        std::fs::rename(&temp_path, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_atomic_replaces_and_leaves_no_temp() {
        let directory = std::env::temp_dir().join("phosphor-proto-fsio");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("out.json");
        write_atomic(&path, b"first").unwrap();
        write_atomic(&path, b"second").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"second");
        let leftovers: Vec<_> = std::fs::read_dir(&directory)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp."))
            .collect();
        assert!(leftovers.is_empty(), "temp files must not survive");
    }

    #[test]
    fn write_atomic_creates_missing_directories() {
        let directory = std::env::temp_dir()
            .join("phosphor-proto-fsio-deep")
            .join("a/b");
        let _ = std::fs::remove_dir_all(&directory);
        let path = directory.join("out.json");
        write_atomic(&path, b"deep").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"deep");
    }
}
