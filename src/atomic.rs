//! Replace a file's contents in one step, or not at all.
//!
//! `fs::write` truncates first, so a reader can meet half a file. Write-then-rename
//! in the same directory (across filesystems `rename` fails) is atomic. Atomic, not
//! exclusive: two writers still lose one update.

use std::path::Path;

use anyhow::{Context, Result};

/// Write `bytes` to `path`, replacing it atomically, via a fixed `<name>.tmp`.
pub fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    // The directory may not exist yet.
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let tmp = tmp_beside(path);
    std::fs::write(&tmp, bytes)
        .with_context(|| format!("writing {} (temp for {})", tmp.display(), path.display()))?;
    std::fs::rename(&tmp, path).with_context(|| {
        // Cleaning up is best-effort: the rename failing is the news, and a
        // second error about the temp file would bury it.
        let _ = std::fs::remove_file(&tmp);
        format!("replacing {} with {}", path.display(), tmp.display())
    })
}

/// The temp's name, beside the target — a sibling, for `rename`.
fn tmp_beside(path: &Path) -> std::path::PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    path.with_file_name(name)
}
