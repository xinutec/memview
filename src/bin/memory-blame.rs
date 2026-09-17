//! Whose lint error is blocking the corpus commit, and a task that says so.
//!
//!     cargo run --release --bin memory-blame          # who owns what
//!     cargo run --release --bin memory-blame -- --file   # and file it
//!
//! A check nobody is addressed by is a check nobody acts on: `mem_check.py`
//! reports to a panel and tells no session anything, and four unstamped
//! memories left the corpus uncommittable for four hours (memview#1235). The
//! transcripts still record who wrote them — `blame::attribute`. A report, and
//! with `--file` a task; never an edit to the corpus.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result};
use memview::blame::{MARKER, attribute, open_task_in, subject};
use memview::filing;
use memview::lint::{self, Finding, Severity};
use memview::store::Corpus;

fn main() -> Result<()> {
    // Refuse a flag this tool does not know (memview#1588).
    memview::flags::reject_unknown(&std::env::args().collect::<Vec<_>>(), &["--file"])?;
    let args: Vec<String> = std::env::args().collect();
    let file = args.iter().any(|a| a == "--file");

    let home = std::env::var("HOME").unwrap_or_default();
    let root = std::env::var("CLAUDE_DIR").unwrap_or_else(|_| format!("{home}/.claude"));
    let memory_dir = std::env::var("MEMORY_DIR")
        .unwrap_or_else(|_| format!("{root}/projects/-Users-pippijn-Code/memory"));
    let projects = std::env::var("PROJECTS_DIR").unwrap_or_else(|_| format!("{root}/projects"));

    // Ask whether it can file BEFORE doing the work: the nightly runs under launchd
    // and needs `TASKS_SESSION`, and discovering that after a gigabyte scan prints
    // "not filed" into a log nobody watches.
    if file {
        run(&["task", "list", "--json"]).context(
            "cannot file: `task` needs an identity. Set TASKS_SESSION=<name> \
             (the nightly files as itself) or pass --session",
        )?;
    }

    let corpus = Corpus::load(&memory_dir)?;
    let findings: Vec<Finding> = lint::check(&corpus, None, None)
        .into_iter()
        .filter(|f| f.severity == Severity::Error)
        .collect();
    if findings.is_empty() {
        println!("no error findings — the corpus is committable");
        return Ok(());
    }

    // The frontmatter answers for most of them without opening a transcript.
    let mut owner: BTreeMap<String, Option<String>> = BTreeMap::new();
    let mut ask_the_transcripts: Vec<String> = Vec::new();
    for finding in &findings {
        if owner.contains_key(&finding.memory) {
            continue;
        }
        let session = corpus
            .docs
            .get(&finding.memory)
            .and_then(|doc| doc.origin_session.clone());
        if session.is_none() {
            ask_the_transcripts.push(format!("{}.md", finding.memory));
        }
        owner.insert(finding.memory.clone(), session);
    }

    // Only for what the frontmatter could not answer: this reads gigabytes.
    if !ask_the_transcripts.is_empty() {
        println!(
            "{} unattributed in frontmatter — reading the transcripts",
            ask_the_transcripts.len()
        );
        let found = attribute(std::path::Path::new(&projects), &ask_the_transcripts, &home);
        for (name, author) in found {
            let stem = name.trim_end_matches(".md").to_string();
            owner.insert(stem, Some(author.session));
        }
    }

    // Refreshed rather than read off disk: a session started since the last mine
    // would show as a bare uuid. About 0.3s — see `memview::fresh`.
    let mined = memview::fresh::mined(
        &memview::fresh::Where::from_env(),
        memview::agents::Needs::MEMORIES,
    )
    .ok();
    let named = |session: &str| -> String {
        mined
            .as_ref()
            .and_then(|m| {
                m.agents
                    .iter()
                    .find(|a| a.sessions.contains(session))
                    .map(|a| a.name.clone())
            })
            .unwrap_or_else(|| session.to_string())
    };

    // Group by AGENT, not by session: a task addressed to a uuid reaches nobody tomorrow.
    let mut by_agent: BTreeMap<String, Vec<&Finding>> = BTreeMap::new();
    let mut unclaimed: Vec<&Finding> = Vec::new();
    for finding in &findings {
        match owner.get(&finding.memory).and_then(|o| o.as_deref()) {
            Some(session) => by_agent.entry(named(session)).or_default().push(finding),
            None => unclaimed.push(finding),
        }
    }

    for (agent, theirs) in &by_agent {
        println!("\n{agent} — {} error(s)", theirs.len());
        for finding in theirs {
            println!("  {:<20} {}", finding.rule, finding.memory);
        }
        if file {
            match file_for(agent, theirs) {
                Ok(what) => println!("  → {what}"),
                Err(why) => println!("  ⚠ not filed: {why:#}"),
            }
        }
    }

    if !unclaimed.is_empty() {
        // Named, not guessed: no transcript claims these, and attaching them to whoever
        // ran this would put a stranger's work in a real queue.
        println!("\nnobody claims these — no surviving transcript records the write");
        for finding in &unclaimed {
            println!("  {:<20} {}", finding.rule, finding.memory);
        }
    }

    if !file {
        println!("\n(nothing filed; pass --file to open one task per agent)");
    }
    Ok(())
}

/// Open or refresh this agent's one task, so a persistent error does not file a
/// new one every night.
fn file_for(agent: &str, findings: &[&Finding]) -> Result<String> {
    let reasons = lint::rule_reasons();
    let mut body = String::from(
        "The nightly cannot commit the corpus while these stand, and \
         `mem_check.py` reports them to a dashboard rather than to you.\n\n",
    );
    let mut rules: BTreeSet<&str> = BTreeSet::new();
    for finding in findings {
        body.push_str(&format!("- `{}` — {}\n", finding.memory, finding.detail));
        rules.insert(finding.rule);
    }
    body.push_str("\nWhat each rule wants:\n");
    for rule in &rules {
        if let Some((_, why)) = reasons.get(rule) {
            body.push_str(&format!("- `{rule}`: {why}\n"));
        }
    }
    body.push_str(&format!(
        "\nFiled by `memory-blame` {MARKER}; it refreshes this task rather than opening another."
    ));

    let memories: Vec<String> = findings.iter().map(|f| f.memory.clone()).collect();
    let title = subject(&memories, findings[0].rule);

    let listed = run(&["task", "list", "--to", agent, "--json"])?;
    if let Some(id) = open_task_in(&listed) {
        let id = id.to_string();
        run(&["task", "edit", &id, "--body", &body, "--subject", &title])?;
        return Ok(format!("refreshed #{id}"));
    }
    // Ask, then overrule — never overrule first. Idempotence here is PER AGENT and
    // the service's duplicate check is global, so the third agent's task got
    // refused; but `--no-duplicate-check` passed unconditionally is itself a 400.
    // The `open_task_in` check above is what makes overruling safe.
    let file = |flags: &[&str]| -> Result<String> {
        let mut argv = vec!["task", "add", &title, "--to", agent, "--priority", "p2"];
        argv.extend_from_slice(flags);
        argv.extend_from_slice(&["--body", &body]);
        run(&argv)
    };
    // The predicate is `filing::is_duplicate_refusal`, in the library so tests can
    // pin the service's live wordings.
    let out = match file(&[]) {
        Ok(out) => out,
        Err(refused) if filing::is_duplicate_refusal(&format!("{refused:#}")) => {
            file(&["--no-duplicate-check"])?
        }
        Err(other) => return Err(other),
    };
    Ok(out.lines().next().unwrap_or("filed").trim().to_string())
}

fn run(argv: &[&str]) -> Result<String> {
    let out = std::process::Command::new(argv[0])
        .args(&argv[1..])
        .output()
        .with_context(|| format!("running {}", argv.join(" ")))?;
    anyhow::ensure!(
        out.status.success(),
        "{} exited {}: {}",
        argv[0],
        out.status,
        String::from_utf8_lossy(&out.stderr).trim()
    );
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}
