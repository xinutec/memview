//! Replace a file's contents in one step, or not at all.
//!
//! `std::fs::write` is not a replacement — it truncates and then writes. Between
//! those two, the file on disk is short, and every reader of these files parses
//! JSON, so a reader that arrives in the gap does not get old data or new data:
//! it gets a parse error. The window is small and it is not theoretical, because
//! two of the three callers are in a request path that runs on every read of a
//! shared page.
//!
//! The worse case is a crash or an eviction in the gap, which leaves the file
//! truncated permanently. `ShareStore::load` treats an unreadable state file as
//! "no share exists", so the outcome is a share link that silently stops
//! working, and `couse.json`/`agents.json` are mined artefacts that would simply
//! read as absent — a graph that quietly loses its usage weighting.
//!
//! Write-then-rename closes both. `rename(2)` within a directory is atomic: a
//! reader sees the whole old file or the whole new one, and a crash leaves the
//! old one intact plus a stray temp file. The temp lives in the SAME directory
//! on purpose — across filesystems `rename` fails with `EXDEV`, and on this
//! deployment the target is a PVC mount while `/tmp` is the container's own
//! filesystem.
//!
//! ⚠ This is atomicity, not mutual exclusion. Two processes each holding their
//! own copy of a whole-file document still lose one of the two updates, whoever
//! renames last — see `ShareStore`, and #744 for why memview's Deployment is
//! `Recreate` rather than rolling.

use std::path::Path;

use anyhow::{Context, Result};

/// Write `bytes` to `path`, replacing it atomically.
///
/// The temp name is derived from the target rather than random: there is one
/// writer per file here, so a fixed name cannot collide, and a leftover
/// `<name>.tmp` after a crash is recognisable rather than one of a growing pile
/// of unexplained files. `agents.rs` already treats `.tmp` as leftover litter.
pub fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    // ⚠ **The directory may not exist yet, and that must not be a failure.**
    // The caches moved under `memview/cache/` (#1240), so the first run on a
    // fresh checkout — or with `MEMVIEW_DIR` pointed at a temp dir, which is how
    // every ablation is run — writes into a directory nothing has created. A
    // miner that dies here after eight minutes of reading is a bad trade for one
    // `create_dir_all`.
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

/// The temp's name, beside the target. Private: which name it uses is not a
/// promise, but that it is a SIBLING is — see the module header on EXDEV — and
/// `tests/atomic.rs` pins that through what the directory holds afterwards.
fn tmp_beside(path: &Path) -> std::path::PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    path.with_file_name(name)
}
