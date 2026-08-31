//! Warn when the index holds work another session wrote.
//!
//!     git diff --cached --name-only | cargo run --release --bin staged-check -- <repo>
//!
//! ⚠ **Paths on stdin, and no git here.** A pre-commit hook exports `GIT_DIR` to
//! everything it spawns, so a checker that ran `git` would ask the committing
//! repository about paths it was already handed — see
//! `reference_a_git_hook_exports_its_repo_to_every_child`.
//!
//! ⚠ **Exits 0 whatever it finds.** Two sessions legitimately edit one file and a
//! refusal would wedge a shared repo. Naming it is the entire fix.

use std::io::Read;

use anyhow::Result;

fn main() -> Result<()> {
    // ⚠ The ABSOLUTE repository path: the artefact keys paths that way.
    let repo = std::env::args().nth(1).unwrap_or_else(|| {
        std::env::current_dir()
            .map(|d| d.display().to_string())
            .unwrap_or_default()
    });
    let me = std::env::var("CLAUDE_AGENT_NAME").unwrap_or_else(|_| {
        std::path::Path::new(&repo)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    });

    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let staged: Vec<String> = input
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    if staged.is_empty() {
        return Ok(());
    }

    // ⚠ **Refreshed, not read off disk.** The night-stale artefact caught 1 of
    // the 6 files in the incident that prompted this: a collision happens in a
    // window of HOURS and the mine ran at 00:36 (memview#1258).
    let last = memview::fresh::last_writer(&memview::fresh::Where::from_env())?;
    let foreign = memview::staged::foreign(&last, &repo, &staged, &me);
    if foreign.is_empty() {
        return Ok(());
    }
    eprintln!(
        "⚠ {} of {} staged path(s) were last written by another session:",
        foreign.len(),
        staged.len()
    );
    for f in &foreign {
        eprintln!("    {:<50} last written by {}", f.path, f.who);
    }
    eprintln!(
        "`git commit` takes the whole INDEX. Unstage what is not yours, or commit deliberately."
    );
    Ok(())
}
