//! A file replaced whole, so no reader ever finds half of one.

use std::io::Write as _;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// Tells apart two replacements under way at once.
static NEXT: AtomicU64 = AtomicU64::new(0);

/// Write `bytes` beside `path`, then rename it over `path`, which is atomic. In
/// place, a process replaced mid-write — an upgrade execs — leaves a cut file the
/// next start cannot read.
pub fn replace(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    replace_with(path, |file| file.write_all(bytes))
}

/// [`replace`], with the contents written by `write` — streamed, for a file too
/// large to hold.
pub fn replace_with(
    path: &Path,
    write: impl FnOnce(&mut std::fs::File) -> std::io::Result<()>,
) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let name = path
        .file_name()
        .map_or_else(Default::default, |it| it.to_string_lossy());
    let beside = path.with_file_name(format!(
        ".{name}.{}.{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let written = std::fs::File::create(&beside).and_then(|mut file| write(&mut file));
    written
        .and_then(|()| std::fs::rename(&beside, path))
        .inspect_err(|_| {
            let _ = std::fs::remove_file(&beside);
        })
}
