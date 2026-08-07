//! Mine the session transcripts for which agent works where.
//!
//!     cargo run --release --bin agents
//!
//! Release build on purpose: this reads every byte of every transcript under
//! `~/.claude/projects`, which is gigabytes. Writes `agents.json` beside them —
//! never inside `memory/`, which `scripts/sync.sh` replaces wholesale, so
//! anything parked there is destroyed on the next sync.

use anyhow::{Context, Result};
use memview::agents;
use memview::couse::stamp;

fn main() -> Result<()> {
    let home = std::env::var("HOME").unwrap_or_default();
    let root = std::env::args()
        .nth(1)
        .unwrap_or_else(|| format!("{home}/.claude/projects"));
    let sessions = format!("{home}/.claude/sessions");
    let out = std::env::args()
        .nth(2)
        .unwrap_or_else(|| format!("{home}/.claude/agents.json"));
    // Overridable so the miner is not welded to one machine's layout, and so
    // nothing publishes a home directory from a public repo.
    let code_root = std::env::var("CODE_ROOT").unwrap_or_else(|_| format!("{home}/Code"));

    // Where the corpus lives, so opening a memory is attributed to that memory
    // instead of being discarded as "outside the code root". Same override as
    // scripts/sync.sh uses, so the two cannot point at different corpora.
    let memory_dir = std::env::var("MEMORY_DIR")
        .unwrap_or_else(|_| format!("{home}/.claude/projects/-Users-pippijn-Code/memory"));

    let generated = stamp(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    );

    let mut found = agents::scan(
        std::path::Path::new(&root),
        std::path::Path::new(&sessions),
        &code_root,
        &memory_dir,
        &home,
        &generated,
    )?;

    println!("{} agents", found.agents.len());
    // What the commit join could and could not do. An unattributed commit is
    // the ordinary case for anything predating the corpus — Claude Code prunes
    // its own old sessions — but the share has to be visible, or these line
    // counts read as a complete account of the history when they are not.
    if found.commits > 0 {
        let joined = found.commits - found.unattributed;
        println!(
            "{joined} of {} commits attributed ({:.0}%); {} have no session left to credit",
            found.commits,
            100.0 * joined as f64 / found.commits as f64,
            found.unattributed
        );
    }
    for agent in &found.agents {
        let reads: usize = agent.reads.values().sum();
        let writes: usize = agent.writes.values().sum();
        // Ordered the way the page orders it — by recent days present, not by
        // lifetime writes — so the console and the UI cannot disagree.
        let top: Vec<String> = {
            let mut v: Vec<(&String, &f64)> = agent.recent_writes.iter().collect();
            v.sort_by(|a, b| b.1.total_cmp(a.1).then_with(|| a.0.cmp(b.0)));
            v.into_iter()
                .take(3)
                .map(|(name, score)| {
                    let n = agent.writes.get(name).copied().unwrap_or(0);
                    format!("{name}({n}w, {score:.1})")
                })
                .collect()
        };
        let added: usize = agent.commit_lines.values().map(|d| d.added).sum();
        let deleted: usize = agent.commit_lines.values().map(|d| d.deleted).sum();
        println!(
            "  {:<18} {:>6} reads {:>6} writes {:>5} deleg  {:>4} commits +{}/-{}",
            agent.name, reads, writes, agent.delegated, agent.commits, added, deleted
        );
        println!("        recent: {}", top.join(" "));
        // What it consults, beside where it works — the two answer different
        // questions and routing a task wants both.
        let mut mem: Vec<(&String, &agents::MemoryUse)> = agent.memories.iter().collect();
        mem.sort_by(|a, b| {
            let (x, y) = (a.1.edits + a.1.reads, b.1.edits + b.1.reads);
            y.cmp(&x)
                .then_with(|| b.1.edits.cmp(&a.1.edits))
                .then_with(|| a.0.cmp(b.0))
        });
        for (name, use_) in mem.iter().take(5) {
            println!("        {:<58} {:>5}r {:>4}e", name, use_.reads, use_.edits);
        }
    }

    // The timeline goes to its own file: it is a hundred times the roster's
    // size and answers a different question, and `/api/agents` must not carry
    // it. `Agents` marks the field `#[serde(skip)]`, so this is the only way it
    // is ever written.
    let timeline = std::mem::take(&mut found.doing);
    let beside = std::path::Path::new(&out).with_file_name("doing.json");
    timeline.save(&beside)?;

    // The memory days go the same way and for the same reason — a different
    // question, and one no view draws. `memory-rank` reads this file.
    let days = std::mem::take(&mut found.memory_days);
    let days_file = std::path::Path::new(&out).with_file_name("memory-days.json");
    std::fs::write(&days_file, serde_json::to_string(&days)?)
        .with_context(|| format!("writing {}", days_file.display()))?;
    println!(
        "{} memories carry days → {}",
        days.len(),
        days_file.display()
    );
    let failed = timeline
        .rows
        .iter()
        .filter(|row| row.v == reader::doing::Verdict::Failed)
        .count();
    println!(
        "\n{} activities, {} of them failed ({:.1}%), {} kinds",
        timeline.rows.len(),
        failed,
        100.0 * failed as f64 / timeline.rows.len().max(1) as f64,
        timeline.kinds.len()
    );
    println!("wrote {}", beside.display());

    found.save(std::path::Path::new(&out))?;
    println!("wrote {out}");
    Ok(())
}
