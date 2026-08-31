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
    let (last, roster) = memview::fresh::last_writer(&memview::fresh::Where::from_env())?;

    // ⚠ **Who this session IS, from the roster — not from the directory name.**
    // The first version took the repository's basename, which is only ever
    // right where the agent and the repo share a name (`memview` in `memview`).
    // In `xinutec-infra` it made every file this session had written read as
    // another session's, which is noise that teaches people to ignore the one
    // warning that matters.
    let me = match std::env::var("CLAUDE_AGENT_NAME").ok().or_else(|| {
        std::env::var("CLAUDE_CODE_SESSION_ID")
            .ok()
            .and_then(|s| roster.name_of_session(&s).map(str::to_string))
    }) {
        Some(me) => me,
        // ⚠ **Skip loudly rather than guess.** With no name, every path is
        // "somebody else's" and the check would flag the whole index — which
        // reads as broken and gets muted, taking the real warnings with it.
        None => {
            eprintln!(
                "⚠ staged-check cannot tell which session it is (no CLAUDE_AGENT_NAME, \
                 and CLAUDE_CODE_SESSION_ID is absent or not in the roster) — \
                 whose-work-is-staged check SKIPPED, not passed."
            );
            return Ok(());
        }
    };
    // ⚠ **The record's spelling of this repo, which may not be the caller's.**
    // `git rev-parse --show-toplevel` resolves symlinks, and `~/Code` points at
    // an external volume here — so a hook that hands over its own cwd names a
    // path the record barely knows and the check silently examines nothing.
    //
    // ⚠ **Corrected HERE rather than in every hook.** The alternative was ~20
    // lines of path derivation copied into each repo's hook, which is the same
    // claim in two places waiting to disagree. The tool knows the record; the
    // hook should only have to name its own directory.
    let repo = match memview::staged::wrong_shape(&last, &repo) {
        None => repo,
        Some(shape) => {
            let same_dir =
                |a: &str, b: &str| match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
                    (Ok(a), Ok(b)) => a == b,
                    _ => false,
                };
            if same_dir(&shape.better, &repo) {
                // The same directory under the name the record uses. Nothing is
                // wrong; the caller simply had the resolved spelling.
                shape.better
            } else {
                // ⚠ A DIFFERENT directory — so this is a misconfiguration, not
                // a spelling. Saying nothing here would report all-clear from
                // no evidence, which is the failure this check exists for.
                eprintln!(
                    "⚠ staged-check was given {repo}, which the record knows {} path(s) \
                     under — but it knows {} under {}, and they are not the same \
                     directory. It checked almost nothing rather than finding nothing.",
                    shape.given, shape.better_count, shape.better
                );
                return Ok(());
            }
        }
    };
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
