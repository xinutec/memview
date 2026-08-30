//! Whose lint error is blocking the corpus commit, and a task that says so.
//!
//!     cargo run --release --bin memory-blame          # who owns what
//!     cargo run --release --bin memory-blame -- --file   # and file it
//!
//! ⚠ **A check nobody is addressed by is a check nobody acts on.** `mem_check.py`
//! runs as a fleetwatch collector: `--json` reports and exits 0. So an ERROR
//! paints a panel, stops the nightly committing the corpus, and tells no session
//! anything. Measured 2026-08-28: four memories written as a byproduct of other
//! work left the corpus uncommittable for about four hours, and the session that
//! wrote them had no way to know. Every fix was mechanical — three stamps and a
//! paragraph that needed marking bold (memview#1235).
//!
//! ⚠ **"Unattributable" was true of the FRONTMATTER, not of the corpus.** An
//! unstamped memory carries no `originSessionId`, so `lint::passed_for_session`
//! matches it to nobody and it fails no session's gate. The transcripts still
//! record who wrote it — `blame::attribute` is the recovery `memory-stamp` has
//! used all along, and this asks it the same question.
//!
//! ⚠ **A report, and with `--file` a task — never an edit to the corpus.** The
//! fixes belong to whoever wrote the memory; this only makes sure they are asked.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result};
use memview::blame::{MARKER, attribute, open_task_in, subject};
use memview::lint::{self, Finding, Severity};
use memview::store::Corpus;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let file = args.iter().any(|a| a == "--file");

    let home = std::env::var("HOME").unwrap_or_default();
    let root = std::env::var("CLAUDE_DIR").unwrap_or_else(|_| format!("{home}/.claude"));
    let memory_dir = std::env::var("MEMORY_DIR")
        .unwrap_or_else(|_| format!("{root}/projects/-Users-pippijn-Code/memory"));
    let projects = std::env::var("PROJECTS_DIR").unwrap_or_else(|_| format!("{root}/projects"));

    // ⚠ **Ask whether it can file BEFORE doing the work, not after.** `task`
    // refuses without an identity — the nightly runs under launchd, where
    // neither `CLAUDE_CODE_SESSION_ID` nor a terminal exists, so it needs
    // `TASKS_SESSION` set. Discovering that per agent, after a transcript scan
    // that reads gigabytes, prints a row of "not filed" into a log nobody is
    // watching — which is this ticket's own failure, one layer down.
    if file {
        run(&["task", "list", "--json"]).context(
            "cannot file: `task` needs an identity. Set TASKS_SESSION=<name> \
             (the nightly files as itself) or pass --session",
        )?;
    }

    let corpus = Corpus::load(&memory_dir)?;
    let findings: Vec<Finding> = lint::check(&corpus, None)
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

    // ⚠ **Only for what the frontmatter could not answer.** This reads every
    // transcript under the projects root, which is gigabytes; asking it about a
    // memory that already declares its author is minutes spent to learn what one
    // file already said.
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

    // ⚠ Refreshed rather than read off disk: this names sessions, and a session
    // that started since the last mine would otherwise show as a bare uuid.
    // Costs about 0.3s — see `memview::fresh`.
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

    // ⚠ **Group by AGENT, not by session.** A name is reused across resumed
    // sessions, and a task addressed to a uuid reaches whoever happens to be
    // that uuid — which is nobody tomorrow.
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
        // ⚠ **Named, not guessed.** No transcript claims these — it predates the
        // archive, or something outside a session wrote the file. (Not pruning:
        // memview#1240 measured that nothing holding a conversation is deleted.)
        // Attaching
        // them to whoever ran this would put a stranger's work in a real queue.
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
    // ⚠ **Ask, then overrule — never overrule first.** This tool's idempotence is
    // PER AGENT and the service's duplicate check is global, so every agent's
    // lint task reads like every other agent's and the third one gets refused:
    // measured 2026-08-28, a real error belonging to `tasks` went unfiled
    // because `dev-lint` and `home` already had one, which is #1235's failure
    // reproduced by #1235's fix.
    //
    // ⚠ But `--no-duplicate-check` is only accepted AFTER a refusal — passing it
    // unconditionally is itself a 400 and files nothing, which is how the first
    // version of this fix broke filing altogether. The `open_task_in` check
    // above is what makes overruling safe here.
    let file = |flags: &[&str]| -> Result<String> {
        let mut argv = vec!["task", "add", &title, "--to", agent, "--priority", "p2"];
        argv.extend_from_slice(flags);
        argv.extend_from_slice(&["--body", &body]);
        run(&argv)
    };
    let out = match file(&[]) {
        Ok(out) => out,
        Err(refused) if format!("{refused:#}").contains("already filed") => {
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
