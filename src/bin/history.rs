//! Mine the session transcripts into `history.json`.
//!
//!     cargo run --release --bin history
//!
//! Run deliberately: it reads every byte of ~3 GB. Writes the artefact BESIDE
//! the transcripts, never inside `memory/` — `scripts/sync.sh` replaces that
//! directory wholesale, so anything parked in it is destroyed on the next sync.
use anyhow::Result;
use memview::history;

fn main() -> Result<()> {
    let home = std::env::var("HOME").unwrap_or_default();
    let root = std::env::args()
        .nth(1)
        .unwrap_or_else(|| format!("{home}/.claude/projects"));
    let sessions = format!("{home}/.claude/sessions");
    let out = std::env::args()
        .nth(2)
        .unwrap_or_else(|| format!("{home}/.claude/history.json"));

    let generated = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let stamp = chrono_stamp(generated);

    // Where projects live. Overridable so the miner is not welded to one
    // machine's layout, and so nothing publishes a home directory.
    let code_root = std::env::var("CODE_ROOT").unwrap_or_else(|_| format!("{home}/Code"));

    let scanned = history::scan(
        std::path::Path::new(&root),
        std::path::Path::new(&sessions),
        &code_root,
        stamp,
    )?;

    println!(
        "{} sessions, {} projects, {} turns",
        scanned.sessions.len(),
        scanned.projects.len(),
        scanned.turns.len()
    );
    for project in scanned.projects.iter().take(10) {
        let who: Vec<String> = project
            .hands
            .iter()
            .take(3)
            .map(|h| format!("{}({})", scanned.sessions[h.session].name, h.turns))
            .collect();
        println!(
            "  {:<18} {:>5} turns  {}..{}  {}",
            project.name,
            project.turns,
            &project.first[..10.min(project.first.len())],
            &project.last[..10.min(project.last.len())],
            who.join(" ")
        );
    }

    scanned.save(std::path::Path::new(&out))?;
    let bytes = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
    println!("\nwrote {out} ({:.1} MB)", bytes as f64 / 1e6);
    Ok(())
}

/// ISO-8601 UTC from a unix timestamp, without pulling in a date crate for one
/// string. The artefact carries it purely so a reader can tell how stale it is.
fn chrono_stamp(secs: u64) -> String {
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (mut y, mut d) = (1970i64, days as i64);
    loop {
        let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
        let len = if leap { 366 } else { 365 };
        if d < len {
            break;
        }
        d -= len;
        y += 1;
    }
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let months = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut m = 0;
    while d >= months[m] {
        d -= months[m];
        m += 1;
    }
    format!(
        "{y:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        m + 1,
        d + 1,
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}
