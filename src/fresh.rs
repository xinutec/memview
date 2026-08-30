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
//! ⚠ **It writes back.** The refreshed artefacts are saved, so the next tool
//! inherits the work instead of repeating it. Two sessions refreshing at once
//! both produce valid states and `atomic::write` makes a torn file impossible —
//! the loser of the race has simply done redundant work, not damage.

use anyhow::{Context, Result};

use crate::agents::{Agents, Resumed};

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

    fn beside(&self, name: &str) -> std::path::PathBuf {
        self.out.with_file_name(name)
    }
}

/// The roster, the timeline and the evidence, current as of now.
///
/// Reads only the transcripts that grew since the last run. Falls back to a full
/// read whenever [`reader::watermark::plan`] cannot prove an append, and whenever
/// the resume state is missing or damaged — see [`crate::mine::Carried::load`].
pub fn mined(at: &Where) -> Result<Agents> {
    let generated = crate::couse::stamp(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    );
    let resume_file = reader::home::cache(crate::mine::FILE);
    let from = crate::mine::Carried::load(&resume_file)?.map(|carried| Resumed {
        carried,
        doing: reader::doing::Doing::load(&at.beside("doing.json")).unwrap_or_default(),
        effects: reader::effects::Effects::load(&at.beside("effects.json")).unwrap_or_default(),
    });

    let (mut found, resume_state) = crate::agents::scan_resumed(
        &at.projects,
        &at.sessions,
        &at.code_root,
        &at.memory_dir,
        &at.home,
        &generated,
        from,
    )?;

    // Each artefact to its own file, for the reasons `bin/agents` gives: they
    // answer different questions and the two large ones must not ride on
    // `/api/agents`.
    let timeline = std::mem::take(&mut found.doing);
    timeline.save(&at.beside("doing.json"))?;
    let effects = std::mem::take(&mut found.effects);
    effects.save(&at.beside("effects.json"))?;

    let mut days = std::mem::take(&mut found.memory_days);
    let days_file = at.beside("memory-days.json");
    // ⚠ Union with what earlier runs saw — a day is a historical fact and cannot
    // stop being true. #884's outcome IS this file.
    crate::agents::carry_forward(&days_file, &mut days)?;
    crate::atomic::write(&days_file, serde_json::to_string(&days)?.as_bytes())
        .with_context(|| format!("writing {}", days_file.display()))?;

    found.save(&at.out)?;
    // ⚠ **Written LAST, after every artefact it describes.** These marks assert
    // "the corpus up to here is already in those files"; saved first, a crash in
    // between would leave a resume state promising work no artefact holds.
    resume_state.save(&resume_file)?;

    // Handed back with the two large fields restored, so a caller that wants the
    // timeline does not have to read the file this just wrote.
    found.doing = timeline;
    found.effects = effects;
    found.memory_days = days;
    Ok(found)
}
