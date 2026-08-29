//! Did every memory this commit changes also move its `modified:` stamp?
//!
//!     git diff --cached | cargo run --release --quiet --bin memory-stamped
//!
//! ⚠ **This is a rule that already existed and had no enforcement.**
//! `reference_modified_stamp_is_not_the_file_mtime` says: *"If you edit from a
//! script, set `modified:` in the same pass."* It was written after a python
//! script advanced 30 mtimes and left 26 stamps untouched, and it has been
//! broken repeatedly since by exactly that shape — a `sed`, a heredoc, a
//! `python3 - <<PY`, none of which stamp the way Edit and Write do.
//!
//! ⚠ **Why the stamp and not the mtime.** The stamp is what recall's age banner
//! reads. A memory keeping an old stamp over new content reads as OLDER than it
//! is, which buys it more scrutiny and is safe; the dangerous direction is the
//! other one, and it arrives when somebody "repairs" stamps in bulk from mtime.
//! That same memory forbids it. This check makes the safe direction cheap so
//! nobody reaches for the dangerous one.
//!
//! ⚠ **Reads a DIFF on stdin and runs no git.** A pre-commit hook exports
//! `GIT_DIR` to every child, so a binary that ran `git -C <the corpus>` would
//! write into whichever repo is committing
//! (`reference_a_git_hook_exports_its_repo_to_every_child`).

use std::io::Read;

use anyhow::Result;

fn main() -> Result<()> {
    let mut diff = String::new();
    std::io::stdin().read_to_string(&mut diff)?;

    let found = memview::stamped::unstamped(&diff);
    if found.is_empty() {
        return Ok(());
    }

    eprintln!(
        "\n{} memory/memories changed without moving `modified:`:\n",
        found.len()
    );
    for name in &found {
        eprintln!("    {name}");
    }
    eprintln!(
        "\nThe stamp is what recall's age banner reads, so leaving it puts new content\n\
         under an old date — the memory then claims to be fresher than it is.\n\n\
         Set `modified:` in the same pass. Edit and Write stamp on their own; a `sed`,\n\
         a heredoc or a python rewrite does not.\n"
    );
    std::process::exit(1);
}
