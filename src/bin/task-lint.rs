//! Two text claims about tickets, checked against the service that holds them.
//!
//!     cargo run --release --bin task-lint
//!
//! **`dangling-citation`** — a memory cites `#N` and no such task exists. A
//! dangling id appears whenever a task is deleted, and nothing notices.
//!
//! **`closed-but-still-asking`** — a CLOSED ticket whose subject is still phrased
//! as the open question it was filed with. The subject is what `task list`
//! prints, so a false one is read far more often than the body that corrects it.
//!
//! ⚠ **The nightly, never the pre-commit gate.** Both need the task service, and
//! the gate runs offline and fast. `claude-sync.sh` already runs on the Mac,
//! reaches the service, and has a reporting channel — which is the argument
//! memview#1179 settled, and where `memory-blame` went for the same reason.
//!
//! ⚠ **Report, never rewrite.** A subject is somebody's sentence about their own
//! work; a tool can say the service disagrees with it, and choosing the new
//! words is not a tool's job (memview#1227).
//!
//! ⚠ **The yield is low and that is the honest case for it.** Around 1% of
//! citations, measured by hand over 207 of them. Its value is catching DRIFT: it
//! costs nothing to run nightly, and the corpus cannot regress on it.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result};
use memview::cites::{citations, cited_paths, is_ours, repo_of, still_asks};
use memview::store::Corpus;

fn main() -> Result<()> {
    let home = std::env::var("HOME").unwrap_or_default();
    let memory_dir = std::env::var("MEMORY_DIR")
        .unwrap_or_else(|_| format!("{home}/.claude/projects/-Users-pippijn-Code/memory"));

    // One call, not one per id: the service answers the whole question at once,
    // and 571 lookups over the network would make this the slow step in a
    // nightly that has none.
    let listed = std::process::Command::new("task")
        .args(["list", "--all", "--done", "--json"])
        .output()
        .context("running task list — is `task` on PATH and is TASKS_SESSION set?")?;
    anyhow::ensure!(
        listed.status.success(),
        "task list exited {}: {}",
        listed.status,
        String::from_utf8_lossy(&listed.stderr).trim()
    );
    let tasks: Vec<serde_json::Value> = serde_json::from_slice(&listed.stdout)?;
    let known: BTreeSet<u64> = tasks.iter().filter_map(|t| t["id"].as_u64()).collect();
    // The names the service itself uses, so a qualified citation can be told
    // from another project's tracker without a list this repo has to maintain.
    let ours: BTreeSet<String> = tasks
        .iter()
        .filter_map(|t| t["assignee"]["name"].as_str().map(str::to_string))
        .collect();
    anyhow::ensure!(!known.is_empty(), "the service returned no tasks at all");

    let corpus = Corpus::load(&memory_dir)?;
    let mut dangling: BTreeMap<u64, Vec<String>> = BTreeMap::new();
    let mut cited = 0usize;
    let mut foreign = 0usize;
    for (name, doc) in &corpus.docs {
        for one in citations(&doc.raw) {
            if !is_ours(&one, &ours) {
                foreign += 1;
                continue;
            }
            cited += 1;
            if !known.contains(&one.id) {
                dangling.entry(one.id).or_default().push(name.clone());
            }
        }
    }

    // dead-path: what an OPEN task claims still exists in its own repository.
    //
    // ⚠ **Open only.** A closed task describes work that finished, and the files
    // it names are supposed to be gone — flagging those reports history as damage.
    let code_root = std::path::PathBuf::from(
        std::env::var("CODE_ROOT").unwrap_or_else(|_| format!("{home}/Code")),
    );
    let mut dead: Vec<(u64, String, String)> = Vec::new();
    let mut paths_checked = 0usize;
    // ⚠ Reported, so "no findings" cannot mean "nothing was read".
    let mut unreadable = 0usize;
    for t in &tasks {
        if t["status"]
            .as_str()
            .is_some_and(|s| s == "done" || s == "closed")
        {
            continue;
        }
        let (Some(id), Some(who)) = (t["id"].as_u64(), t["assignee"]["name"].as_str()) else {
            continue;
        };
        // ⚠ A session with no checkout here says NOTHING about paths. Reporting
        // its citations as dead would accuse a repo this machine cannot see.
        let Some(repo) = repo_of(who, &code_root) else {
            continue;
        };
        let shown = std::process::Command::new("task")
            .args(["show", "--json", &id.to_string()])
            .output();
        let Ok(shown) = shown else { continue };
        // ⚠ **A body that will not parse is COUNTED, never treated as empty.**
        // `unwrap_or_default()` here would turn a broken service response into a
        // task with no citations — so a service returning nonsense would report
        // "every path still exists", which is the loudest possible lie this check
        // can tell. dev-lint's `rust-serde-swallow` caught exactly that.
        let body = match serde_json::from_slice::<serde_json::Value>(&shown.stdout) {
            Ok(v) => v["body"].as_str().unwrap_or_default().to_string(),
            Err(_) => {
                unreadable += 1;
                continue;
            }
        };
        for path in cited_paths(&body) {
            paths_checked += 1;
            if !repo.join(&path).exists() {
                dead.push((id, who.to_string(), path));
            }
        }
    }

    println!(
        "{cited} citations across {} memories, against {} tasks the service holds",
        corpus.docs.len(),
        known.len()
    );
    println!("  {foreign} more name another project's tracker and are not ours to check");
    if dangling.is_empty() {
        println!("  every cited id resolves");
    } else {
        println!("\ndangling-citation — cited, and no such task:");
        for (id, memories) in &dangling {
            println!("  #{id:<6} {}", memories.join(", "));
        }
    }
    // ⚠ **Reported last and separately**: it is the highest-yield rule here by an
    // order of magnitude, and burying it under the citation counts would read as
    // a footnote to a check that finds almost nothing.
    // ⚠ **Said before any verdict.** A run that could not read half the tasks and
    // found nothing is not a clean run, and the two must never look alike.
    if unreadable > 0 {
        println!("\n  ⚠ {unreadable} task(s) returned a body that would not parse — NOT checked");
    }
    if dead.is_empty() {
        println!("\n  every path an open task cites still exists ({paths_checked} checked)");
    } else {
        let mut repos: BTreeSet<&str> = BTreeSet::new();
        for (_, who, _) in &dead {
            repos.insert(who.as_str());
        }
        println!(
            "\ndead-path — an OPEN task cites a file that is gone ({} of {paths_checked} cited paths, {} repo(s)):",
            dead.len(),
            repos.len()
        );
        for (id, who, path) in &dead {
            println!("  #{id:<6} {who:<10} {path}");
        }
        println!(
            "  ⚠ about 1 in 30 of these is an illustrative path rather than a citation — see this file's head."
        );
    }

    // ⚠ **Closed only.** An open ticket SHOULD still ask its question; that is
    // what an open ticket is. The claim only becomes false when the status says
    // the work is finished.
    let mut asking: Vec<(u64, &str)> = tasks
        .iter()
        .filter(|t| matches!(t["status"].as_str(), Some("done" | "dropped")))
        .filter_map(|t| Some((t["id"].as_u64()?, t["subject"].as_str()?)))
        .filter(|(_, subject)| still_asks(subject))
        .collect();
    asking.sort();

    println!();
    if asking.is_empty() {
        println!("closed-but-still-asking — none");
    } else {
        println!(
            "closed-but-still-asking — {} closed ticket(s) whose subject still asks:",
            asking.len()
        );
        for (id, subject) in &asking {
            println!("  #{id:<6} {subject}");
        }
        println!("  → the subject is what `task list` prints; the body's correction is not.");
    }

    Ok(())
}
