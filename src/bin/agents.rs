//! Mine the session transcripts for which agent works where.
//!
//!     cargo run --release --bin agents
//!
//! Release build on purpose: this reads every byte of every transcript under
//! `~/.claude/projects`, which is gigabytes. Writes `agents.json` beside them —
//! never inside `memory/`, which `scripts/sync.sh` replaces wholesale, so
//! anything parked there is destroyed on the next sync.
use anyhow::Result;
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
        &generated,
    )?;

    println!("{} agents", found.agents.len());
    for agent in &found.agents {
        let reads: usize = agent.reads.values().sum();
        let writes: usize = agent.writes.values().sum();
        let top: Vec<String> = {
            let mut v: Vec<(&String, &usize)> = agent.writes.iter().collect();
            v.sort_by_key(|(name, n)| (std::cmp::Reverse(**n), name.as_str()));
            v.into_iter()
                .take(3)
                .map(|(name, n)| format!("{name}({n})"))
                .collect()
        };
        println!(
            "  {:<18} {:>6} reads {:>6} writes   writes: {}",
            agent.name,
            reads,
            writes,
            top.join(" ")
        );
    }

    found.save(std::path::Path::new(&out))?;
    println!("\nwrote {out}");
    Ok(())
}
