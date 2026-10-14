//! The mined artefacts, brought up to date at the moment they are read.
//!
//! Catching up costs seconds, so a reader does it before answering rather than
//! warning of staleness. Readers never write — only `bin/agents` owns the
//! artefacts — so each catches up from the last mine. The nightly stays a full
//! rebuild, the baseline that repairs any drift.

use anyhow::Result;

use crate::agents::{Agents, Needs, Resumed};

/// Where the miner reads and writes, defaulted from the environment, in one place.
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
            memory_dir: reader::home::memory_dir().to_string_lossy().into_owned(),
            home,
            out: reader::home::cache("agents.json"),
        }
    }
}

/// The effects — who last touched which file, ever — current as of now, carried
/// from `effects.json`. Writes nothing.
pub fn effects(at: &Where) -> Result<reader::effects::Effects> {
    // Refuse rather than answer "nobody" from minutes of history.
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
        // Absent is not empty: the nightly deletes this export.
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

/// The mined view a reader needs, current as of now, in memory: only what grew,
/// unless an append cannot be proved. Writes nothing.
pub fn mined(at: &Where, needs: Needs) -> Result<Agents> {
    let generated = crate::couse::stamp(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    );
    let resume_file = reader::home::cache(crate::mine::FILE);
    // Not the timeline and evidence: a reader needs only what `Carried` holds.
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

/// Who last wrote each path, current as of now, from [`crate::last_writer`].
/// Refuses rather than answer from nothing, like [`effects`].
pub fn last_writer(at: &Where) -> Result<(crate::last_writer::LastWriter, Agents)> {
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
    // The roster too, so the caller knows who it is — never from the repo's name.
    Ok((known, tail))
}
