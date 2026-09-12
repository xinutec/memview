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

    // ⚠ The diff's paths are repo-relative and the hook runs at the repo root,
    // so this reads the WORKING TREE, not the index. For the one question asked
    // of it — does this file carry a `modified:` at all — the two cannot
    // disagree in a way that matters: becoming a memory is not something a
    // commit does halfway. Still no `git` call, for the reason the module header
    // gives (a hook exports `GIT_DIR` to every child).
    let found = memview::stamped::unstamped(&diff, |path| std::fs::read_to_string(path).ok());

    if !found.unread.is_empty() {
        eprintln!(
            "\n{} file(s) changed their body, but could not be read to see whether\n\
             they carry a `modified:` at all:\n",
            found.unread.len()
        );
        for name in &found.unread {
            eprintln!("    {name}");
        }
        eprintln!(
            "\nRun this from the root of the repo the diff came from. Reported rather than\n\
             skipped on purpose: an unreadable file and a file with no stamp look identical\n\
             from here, and treating them alike would silence this check everywhere at once.\n"
        );
        std::process::exit(1);
    }

    if found.stale.is_empty() {
        return Ok(());
    }

    eprintln!(
        "\n{} memory/memories changed without moving `modified:`:\n",
        found.stale.len()
    );
    for name in &found.stale {
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
