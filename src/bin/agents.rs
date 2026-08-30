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
    // ⚠ **Flags taken out before the positionals are counted.** `root` and `out`
    // are read by position, so a bare `--resume` would otherwise become the
    // projects root and the mine would read an empty directory and report a
    // corpus of nothing — a wrong answer with no error.
    // ⚠ **A flag's VALUE is not a positional argument, and getting this wrong is
    // silent.** `--exports /tmp/x` left `/tmp/x` looking positional, so it became
    // the projects ROOT: the mine read an empty directory, found no transcripts,
    // and wrote a resume state with zero marks over a good one. Green, fast, and
    // entirely wrong — the exact failure the `--resume` note warned about, then
    // reintroduced by adding a flag that takes a value.
    let positional: Vec<String> = {
        const TAKES_VALUE: &[&str] = &["--exports"];
        let all: Vec<String> = std::env::args().skip(1).collect();
        let mut out = Vec::new();
        let mut i = 0;
        while i < all.len() {
            if TAKES_VALUE.contains(&all[i].as_str()) {
                i += 2; // the flag and the value it owns
            } else if all[i].starts_with("--") {
                i += 1;
            } else {
                out.push(all[i].clone());
                i += 1;
            }
        }
        out
    };
    let root = positional
        .first()
        .cloned()
        .unwrap_or_else(|| format!("{home}/.claude/projects"));
    let sessions = format!("{home}/.claude/sessions");
    let out = positional.get(1).cloned().unwrap_or_else(|| {
        reader::home::cache("agents.json")
            .to_string_lossy()
            .into_owned()
    });
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

    // ⚠ **Opt-in, and it stays that way until parity is shown on the REAL
    // corpus.** The fixtures prove a resumed scan equals a whole one; a fixture
    // is not 6.28 GB of transcripts, and a wrong resume reads no error — it
    // mines from an offset that means something else and the artefact simply
    // becomes untrue. `--resume` is how that comparison gets run at all.
    let want_resume = std::env::args().any(|a| a == "--resume");
    // ⚠ **`doing.json` and `effects.json` are EXPORTS, not local data.** 130 MB
    // that exists only to be pushed to the console — nothing on this Mac reads
    // them except this miner, to resume the timeline. `--exports <dir>` sends
    // them somewhere temporary so the push can carry them without leaving them
    // in `~/.claude`; `--exports none` skips them entirely, which is what the
    // hourly resumed mine wants.
    //
    // ⚠ **`none` means NOT LOADED either.** A resumed run that cannot carry the
    // previous timeline produces a wrong one — measured: 78 orphaned rows and
    // renumbered episodes (memview#1240). So it does not pretend to produce one.
    // Only a FULL mine writes a timeline anybody should read.
    let exports: Option<std::path::PathBuf> = {
        let args: Vec<String> = std::env::args().collect();
        match args.iter().position(|a| a == "--exports") {
            None => Some(std::path::Path::new(&out).with_file_name(".").to_path_buf()),
            Some(i) => match args.get(i + 1).map(String::as_str) {
                Some("none") | None => None,
                Some(dir) => Some(std::path::PathBuf::from(dir)),
            },
        }
    };
    let resume_file = reader::home::cache(memview::mine::FILE);
    let from = if want_resume {
        // ⚠ An unparseable resume file is FATAL here, not "nothing to resume":
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
                    // ⚠ Carried only when this run will WRITE a timeline. With
                    // `--exports none` there is nothing to carry and nothing
                    // worth producing; the roster and the day sets do not
                    // depend on either.
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
    // What the commit join could and could not do. An unattributed commit is
    // the ordinary case for anything predating the corpus — but the share has
    // to be visible, or these line
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
    // ⚠ **An export may only come from a run that HOLDS the whole timeline.**
    // A resumed run that could not carry the previous one produces the tail
    // alone — measured here as a 108-byte `doing.json` with 0 effects, which
    // pushed to the console would replace a real timeline with nothing. A full
    // mine always qualifies; a resumed one only if it loaded what it is about
    // to extend.
    //
    // Refusing rather than warning: the push reads whatever is on disk, so a
    // warning nobody sees would still ship the empty file.
    let partial = want_resume && carried_timeline_rows == 0 && !timeline.rows.is_empty();
    anyhow::ensure!(
        !(exports.is_some() && partial),
        "refusing to write exports from a resumed run that carried no timeline — \
         they would hold only the tail. Run a full mine, or pass --exports none."
    );
    if let Some(dir) = &exports {
        // ⚠ Explicit: the directory usually exists, and if it cannot be made
        // the `save` below fails with a message naming the path.
        let _ = std::fs::create_dir_all(dir);
        timeline.save(&dir.join("doing.json"))?;
    } else {
        // ⚠ Said out loud. A run that silently stopped producing the console's
        // data would look identical to one that produced it.
        println!("exports skipped (--exports none): no timeline or effects written");
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
    // ⚠ **The measured size, printed rather than estimated.** memview#93 was
    // planned against 55 MB, then against 20 MB once the unit was measured; both
    // were arithmetic on dictionaries, not a file on disk. Whoever adds this to
    // `sync.sh` should be reading a number nobody had to compute.
    println!(
        "{} effects over {} paths, {} commands, {} patterns → {} ({:.1} MB)",
        effects.rows.len(),
        effects.paths.len(),
        effects.commands.len(),
        effects.patterns.len(),
        effects_file.display(),
        size as f64 / 1e6,
    );

    // The memory days go the same way and for the same reason — a different
    // question, and one no view draws. `memory-rank` reads this file.
    //
    // ⚠ **"The same way" now includes HOW it is written.** This was the one
    // sibling on a plain `fs::write` while `agents.json`, `doing.json` and
    // `effects.json` all went through `atomic::write` — so a `memory-rank` run
    // during the 00:30 mine could read a half-written file, and the mine takes
    // ~8 minutes. Write-then-rename or the reader sees half.
    let mut days = std::mem::take(&mut found.memory_days);
    let days_file = std::path::Path::new(&out).with_file_name("memory-days.json");
    // ⚠ Union with what earlier runs saw — see [`memview::agents::carry_forward`].
    // #884's outcome IS this file, over a pre-period of 2026-07-17..08-14 that has
    // to survive to a harvest on 2026-09-11. Membership was already protected
    // this way (`index-history.json`); the outcome variable never was.
    let carried = memview::agents::carry_forward(&days_file, &mut days)?;
    if carried > 0 {
        // ⚠ **Said out loud rather than folded into a total**: a silent carry
        // reads exactly like a complete re-mine.
        //
        // ⚠ **It used to say "transcripts have been pruned", and that is false**
        // — memview#1240 measured that nothing holding a conversation has been
        // deleted since the archive began. What vanishes is temp-directory
        // sessions, which carried no conversation, plus whatever predates
        // 2026-07-31. The message states the observation, not a cause.
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

    // ⚠ **Written on EVERY run, including a whole one.** A full mine is how the
    // resume state is bootstrapped and how it is repaired after a
    // `Plan::Full` — writing it only when `--resume` was asked for would mean
    // the first resumable run could never happen.
    //
    // ⚠ **Written LAST, after every artefact it describes.** These marks assert
    // "the corpus up to here is already in those files"; saved first, a crash in
    // between would leave a resume state promising work no artefact holds, and
    // the next run would skip it silently.
    resume_state.save(&resume_file)?;
    println!(
        "wrote {} ({} transcript mark(s))",
        resume_file.display(),
        resume_state.marks.len()
    );
    Ok(())
}
