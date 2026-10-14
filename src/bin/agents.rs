//! Mine the session transcripts for which agent works where.
//!
//!     cargo run --release --bin agents
//!
//! Release build: it reads gigabytes. Writes beside `memory/`, never inside it —
//! the sync replaces that directory.

use anyhow::{Context, Result};
use clap::Parser;
use memview::agents;
use memview::couse::stamp;

/// Mine the session transcripts for which agent works where.
#[derive(Parser)]
struct Cli {
    /// The projects root [default: ~/.claude/projects].
    root: Option<String>,
    /// Where to write [default: agents.json in memview's cache].
    out: Option<String>,
    /// Where `doing.json` and `effects.json` go, or `none` to skip them (and
    /// then no timeline is written either) [default: beside OUT].
    #[arg(long, value_name = "DIR|none")]
    exports: Option<String>,
    /// Resume from the recorded marks rather than reading every transcript
    /// whole. Opt-in until parity is shown on the real corpus.
    #[arg(long)]
    resume: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let home = std::env::var("HOME").unwrap_or_default();
    let root = cli
        .root
        .unwrap_or_else(|| format!("{home}/.claude/projects"));
    let sessions = format!("{home}/.claude/sessions");
    let out = cli.out.unwrap_or_else(|| {
        reader::home::cache("agents.json")
            .to_string_lossy()
            .into_owned()
    });
    // Overridable so the miner is not welded to one machine's layout, and so
    // nothing publishes a home directory from a public repo.
    let code_root = std::env::var("CODE_ROOT").unwrap_or_else(|_| format!("{home}/Code"));

    // Where the corpus lives — the same override `scripts/sync.sh` uses.
    let memory_dir = reader::home::memory_dir().to_string_lossy().into_owned();

    let generated = stamp(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    );

    // Opt-in until parity is shown on the real corpus: a wrong resume reports no error.
    let want_resume = cli.resume;
    // `doing.json` and `effects.json` are exports for the console: `--exports <dir>`
    // writes them elsewhere, `none` neither loads nor writes them. Only a full mine
    // writes a timeline worth reading.
    let exports: Option<std::path::PathBuf> = match cli.exports.as_deref() {
        None => Some(std::path::Path::new(&out).with_file_name(".").to_path_buf()),
        Some("none") => None,
        Some(dir) => Some(std::path::PathBuf::from(dir)),
    };
    let resume_file = reader::home::cache(memview::mine::FILE);
    let from = if want_resume {
        // An unparseable resume file is FATAL here, not "nothing to resume":
        // see `mine::Carried::load`.
        match memview::mine::Carried::load(&resume_file)? {
            None => {
                println!(
                    "no resume state at {} — reading whole",
                    resume_file.display()
                );
                None
            }
            Some(carried) => {
                println!(
                    "resuming from {} transcript mark(s) recorded {}, roster of {} agent(s)",
                    carried.marks.len(),
                    if carried.generated.is_empty() {
                        "(unstamped)"
                    } else {
                        &carried.generated
                    },
                    carried.agents.len()
                );
                Some(agents::Resumed {
                    carried,
                    // Carried only when this run writes a timeline.
                    doing: match &exports {
                        Some(dir) => {
                            reader::doing::Doing::load(&dir.join("doing.json")).unwrap_or_default()
                        }
                        None => reader::doing::Doing::default(),
                    },
                    effects: match &exports {
                        Some(dir) => reader::effects::Effects::load(&dir.join("effects.json"))
                            .unwrap_or_default(),
                        None => reader::effects::Effects::default(),
                    },
                })
            }
        }
    } else {
        None
    };

    // How much timeline this run STARTED with, so the export guard below can tell
    // "resumed and extended it" from "resumed and never had it".
    let carried_timeline_rows = from.as_ref().map(|r| r.doing.rows.len()).unwrap_or(0);

    let (mut found, resume_state) = agents::scan_resumed(
        agents::Roots {
            projects: std::path::Path::new(&root),
            sessions: std::path::Path::new(&sessions),
            code_root: &code_root,
            memory_root: &memory_dir,
            home: &home,
        },
        &generated,
        from,
        agents::Needs::EVERYTHING,
    )?;

    println!("{} agents", found.agents.len());
    // What the commit join could not attribute, so the counts are not taken as complete.
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

    // The timeline goes to its own file; `/api/agents` must not carry it.
    let timeline = std::mem::take(&mut found.doing);
    // Exported only from a run holding the whole timeline, or the push would
    // replace a real one with a tail. Refused, not warned: the push reads the disk.
    let partial = want_resume && carried_timeline_rows == 0 && !timeline.rows.is_empty();
    anyhow::ensure!(
        !(exports.is_some() && partial),
        "refusing to write exports from a resumed run that carried no timeline — \
         they would hold only the tail. Run a full mine, or pass --exports none."
    );
    if let Some(dir) = &exports {
        // Explicit: the directory usually exists, and if it cannot be made
        // the `save` below fails with a message naming the path.
        let _ = std::fs::create_dir_all(dir);
        timeline.save(&dir.join("doing.json"))?;
    } else {
        // Said out loud. A run that silently stopped producing the console's
        // data would look identical to one that produced it.
        println!("exports skipped (--exports none): no timeline or effects written");
    }

    // Who last wrote each path, kept locally. A full mine builds from empty; a
    // resumed one absorbs onto what it carried, and must not create it — a tail
    // alone looks complete and answers "nobody" for the rest.
    {
        let file = reader::home::cache(memview::last_writer::FILE);
        let carried = memview::last_writer::LastWriter::load(&file)?;
        match (want_resume, carried) {
            (true, None) => println!(
                "no {} to extend — a full mine seeds it (memview#1258)",
                file.display()
            ),
            (resumed, carried) => {
                let mut last = if resumed {
                    carried.unwrap_or_default()
                } else {
                    memview::last_writer::LastWriter::default()
                };
                last.absorb(&found.effects);
                last.save(&file)?;
                println!(
                    "{} path(s) carry a last writer → {}",
                    last.len(),
                    file.display()
                );
            }
        }
    }

    // The evidence under the timeline, to its own file again: it is larger than
    // both and answers the question a reader asks standing on a timeline row.
    let effects = std::mem::take(&mut found.effects);
    let effects_file = exports
        .as_ref()
        .map(|dir| dir.join("effects.json"))
        .unwrap_or_default();
    if exports.is_some() {
        effects.save(&effects_file)?;
    }
    let size = std::fs::metadata(&effects_file)
        .map(|m| m.len())
        .unwrap_or(0);
    // The measured size, printed rather than estimated.
    println!(
        "{} effects over {} paths, {} commands, {} patterns → {} ({:.1} MB)",
        effects.rows.len(),
        effects.paths.len(),
        effects.commands.len(),
        effects.patterns.len(),
        effects_file.display(),
        size as f64 / 1e6,
    );

    // The memory days, for `memory-rank`, written atomically like the rest.
    let mut days = std::mem::take(&mut found.memory_days);
    let days_file = std::path::Path::new(&out).with_file_name("memory-days.json");
    // Union with what earlier runs saw — [`memview::agents::carry_forward`].
    let carried = memview::agents::carry_forward(&days_file, &mut days)?;
    if carried > 0 {
        // Said out loud: a silent carry reads like a complete re-mine. It states
        // the observation, not a cause.
        println!("carried {carried} memory-day(s) from sessions with no surviving transcript");
    }
    memview::atomic::write(&days_file, serde_json::to_string(&days)?.as_bytes())
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
    if let Some(dir) = &exports {
        println!("wrote {}", dir.join("doing.json").display());
    }

    found.save(std::path::Path::new(&out))?;
    println!("wrote {out}");

    // Every run writes it — a full mine bootstraps it — and last, after the
    // artefacts it vouches for, or a crash would leave it promising work.
    resume_state.save(&resume_file)?;
    println!(
        "wrote {} ({} transcript mark(s))",
        resume_file.display(),
        resume_state.marks.len()
    );
    Ok(())
}
