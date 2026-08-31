//! The mined artefacts, brought up to date at the moment they are read.
//!
//! ⚠ **A cache nobody refreshes is a cache that lies, and every reader here
//! handled that differently.** Before this existed: `memory-rank` REFUSED when
//! the artefacts were stale and told you to spend 4m31 re-mining, `memory-tiers`
//! disclosed the staleness and used the old numbers anyway, and
//! `demotion-study`, `memory-blame` and the viewer API did not look. Three
//! answers to one question, none of them "be up to date".
//!
//! ⚠ **The point is not that the nightly gets faster.** The nightly is ~9
//! minutes, unattended, and nobody waits for it. The point is that catching up
//! now costs about **9 seconds**, which is cheap enough to do before answering
//! rather than to warn about. That is what makes `--stale-ok` and the refusal in
//! `memory-rank` unnecessary.
//!
//! ⚠ **The nightly stays a FULL rebuild on purpose.** A resumed run is only ever
//! as correct as the chain of resumes behind it; a from-scratch mine is the
//! thing that repairs any drift the chain accumulates, and it is the baseline
//! every parity check is measured against.
//!
//! ⚠ **A reader here NEVER WRITES, and that is the point.** Only `bin/agents`
//! owns the artefacts. A reader that wrote them would be a second writer racing
//! every other session — but worse, it could not then skip any work, because
//! writing a partially-computed artefact corrupts it. Staying read-only is what
//! lets a caller say [`Needs::MEMORIES`] and not pay 4.4s of git walk for
//! numbers it never looks at.
//!
//! ⚠ **So each reader catches up from the last MINE, not from the last reader.**
//! Measured 2026-08-30: the corpus grows about 1 MB per eight minutes, so a full
//! day of drift is a few hundred MB of tails — seconds, not minutes. Cheaper
//! than the coordination a shared writable cache would need.

use anyhow::Result;

use crate::agents::{Agents, Needs, Resumed};

/// Where the miner reads and writes, defaulted from the environment.
///
/// Held in one struct because five call sites derived these separately and any
/// disagreement between them is a tool reading a different corpus from the one
/// it refreshes.
pub struct Where {
    pub projects: std::path::PathBuf,
    pub sessions: std::path::PathBuf,
    pub code_root: String,
    pub memory_dir: String,
    pub home: String,
    pub out: std::path::PathBuf,
}

impl Where {
    pub fn from_env() -> Self {
        let home = std::env::var("HOME").unwrap_or_default();
        Self {
            projects: reader::home::projects_dir(),
            sessions: std::path::PathBuf::from(format!("{home}/.claude/sessions")),
            // Overridable so nothing is welded to one machine's layout, and so
            // no public repo publishes a home directory.
            code_root: std::env::var("CODE_ROOT").unwrap_or_else(|_| format!("{home}/Code")),
            memory_dir: std::env::var("MEMORY_DIR")
                .unwrap_or_else(|_| format!("{home}/.claude/projects/-Users-pippijn-Code/memory")),
            home,
            out: reader::home::cache("agents.json"),
        }
    }
}

/// The effects — who last touched which file — current as of now.
///
/// ⚠ **This one DOES carry `effects.json`**, because the question is "who wrote
/// this path, ever", not "what happened lately". [`mined`] deliberately does not,
/// and the difference is the whole reason both exist: a memory tool wants a fold
/// over the corpus, this wants the corpus.
///
/// Still writes nothing. Costs the 70 MB parse plus whatever grew.
pub fn effects(at: &Where) -> Result<reader::effects::Effects> {
    // ⚠ **Refuse rather than answer from nothing.** Without the carried artefact
    // a resumed scan sees only what grew, so "who last wrote this path" would be
    // answered from a few minutes of history and read as "nobody" — a check that
    // reports all-clear because it has no evidence, which is worse than one that
    // does not run.
    anyhow::ensure!(
        reader::home::cache("effects.json").exists(),
        "no effects.json on this machine — it is an export the nightly builds and \
         deletes (memview#1240). Rebuild it with: cargo run --release --bin agents \
         -- --exports <dir>"
    );
    let generated = crate::couse::stamp(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    );
    let carried = crate::mine::Carried::load(&reader::home::cache(crate::mine::FILE))?;
    let from = carried.map(|carried| Resumed {
        carried,
        doing: reader::doing::Doing::default(),
        // ⚠ **Absent is NOT empty here.** `effects.json` is an export the nightly
        // now builds into a temp directory and deletes, so on this Mac it is
        // usually GONE. Defaulting to empty would make every caller — the
        // staged-work check above all — report "nothing found" when it in fact
        // read nothing. The caller is told below instead.
        effects: reader::effects::Effects::load(&reader::home::cache("effects.json"))
            .unwrap_or_default(),
    });
    let (found, _) = crate::agents::scan_resumed(
        crate::agents::Roots {
            projects: &at.projects,
            sessions: &at.sessions,
            code_root: &at.code_root,
            memory_root: &at.memory_dir,
            home: &at.home,
        },
        &generated,
        from,
        Needs::MEMORIES,
    )?;
    Ok(found.effects)
}

/// The mined view a reader needs, current as of now, computed in memory.
///
/// Reads only the transcripts that grew since the last mine. Falls back to a
/// full read whenever [`reader::watermark::plan`] cannot prove an append, and
/// whenever the resume state is missing or damaged — see
/// [`crate::mine::Carried::load`].
///
/// ⚠ **Writes nothing.** See this module's head.
pub fn mined(at: &Where, needs: Needs) -> Result<Agents> {
    let generated = crate::couse::stamp(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    );
    let resume_file = reader::home::cache(crate::mine::FILE);
    // ⚠ **The timeline and the evidence are 122 MB and are NOT loaded here.**
    // A reader that does not write them does not need them carried; the fold
    // state a memory tool actually reads — the roster, the day sets, `resolved`,
    // `first_seen` — all rides in `Carried`, which is 1.5 MB.
    let from = crate::mine::Carried::load(&resume_file)?.map(|carried| Resumed {
        carried,
        doing: reader::doing::Doing::default(),
        effects: reader::effects::Effects::default(),
    });

    let (found, _resume_state) = crate::agents::scan_resumed(
        crate::agents::Roots {
            projects: &at.projects,
            sessions: &at.sessions,
            code_root: &at.code_root,
            memory_root: &at.memory_dir,
            home: &at.home,
        },
        &generated,
        from,
        needs,
    )?;

    Ok(found)
}

/// Who last wrote each path, current as of now.
///
/// ⚠ **Carries [`crate::last_writer`], NOT `effects.json`.** The 70 MB export
/// left this machine, and it was only ever being parsed to answer this one
/// question. Loading the fold instead costs a few MB and the tail.
///
/// ⚠ **Refuses rather than answering from nothing**, for the reason [`effects`]
/// gives: a resumed scan alone sees only what grew, so "who last wrote this"
/// would be answered from minutes of history and read as "nobody" — a check
/// reporting all-clear because it has no evidence.
pub fn last_writer(at: &Where) -> Result<crate::last_writer::LastWriter> {
    let file = reader::home::cache(crate::last_writer::FILE);
    let mut known = crate::last_writer::LastWriter::load(&file)?.ok_or_else(|| {
        anyhow::anyhow!(
            "no {} on this machine — the miner writes it. Build it with: \
             cargo run --release --bin agents -- --exports none",
            file.display()
        )
    })?;
    let tail = mined(at, Needs::MEMORIES)?;
    // Only what grew since the last mine: `mined` carries no effects, so these
    // rows ARE the tail and folding them is the catch-up.
    known.absorb(&tail.effects);
    Ok(known)
}
