//! Mine the session transcripts for which agent works where.
//!
//!     cargo run --release --bin agents
//!
//! Release build on purpose: this reads every byte of every transcript under
//! `~/.claude/projects`, which is gigabytes. Writes `agents.json` beside them —
//! never inside `memory/`, which `scripts/sync.sh` replaces wholesale, so
//! anything parked there is destroyed on the next sync.
use std::collections::BTreeSet;

use anyhow::Result;
use memview::agents;
use memview::couse::stamp;
use memview::store::Corpus;

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

    // The memory names to attribute mentions to. Filtered against the live
    // corpus for the same reason the co-use miner is: this repo's own test
    // fixtures match every pattern a memory does. Absent corpus, absent
    // profile — the rest of the mine is unaffected.
    let memory_dir = std::env::var("MEMORY_DIR")
        .unwrap_or_else(|_| format!("{home}/.claude/projects/-Users-pippijn-Code/memory"));
    let corpus: BTreeSet<String> = Corpus::load(&memory_dir)
        .map(|c| c.docs.keys().cloned().collect())
        .unwrap_or_default();
    if corpus.is_empty() {
        println!("no corpus at {memory_dir} — mining without the memory profile");
    }

    let generated = stamp(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    );

    let found = agents::scan(
        std::path::Path::new(&root),
        std::path::Path::new(&sessions),
        &code_root,
        &corpus,
        &generated,
    )?;

    println!("{} agents", found.agents.len());
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
        println!(
            "  {:<18} {:>6} reads {:>6} writes {:>5} deleg   recent: {}",
            agent.name,
            reads,
            writes,
            agent.delegated,
            top.join(" ")
        );
        // What it consults, beside where it works — the two answer different
        // questions and routing a task wants both.
        let mut mem: Vec<(&String, &agents::MemoryUse)> = agent.memories.iter().collect();
        mem.sort_by(|a, b| b.1.mentions.cmp(&a.1.mentions).then_with(|| a.0.cmp(b.0)));
        for (name, use_) in mem.iter().take(5) {
            println!(
                "        {:<58} {:>5} mentions {:>4}r {:>3}e",
                name, use_.mentions, use_.reads, use_.edits
            );
        }
    }

    found.save(std::path::Path::new(&out))?;
    println!("\nwrote {out}");
    Ok(())
}
