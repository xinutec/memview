//! What a resumed mine has to carry, and why the artefacts cannot supply it.
//!
//! [`reader::watermark::plan`] decides WHICH transcripts a run must read. This
//! module holds the other half: the fold state a run must start FROM, so that
//! reading only the tails gives the answer reading everything would give.
//!
//! ⚠ **The mined artefacts do not contain that state, and this is the finding
//! that shapes the file.** `agents.json` carries per-agent totals,
//! `memory-days.json` carries the corpus-wide union, `doing.json` and
//! `effects.json` carry rows. Three of `agents::scan`'s five folds are consumed
//! on the way out and never written down:
//!
//!   * **`resolved`** — session id to agent name. A session is named in the HEAD
//!     of its transcript, so a tail-only read cannot re-derive it. Without this
//!     carried, a resumed run files every long-running session under its raw
//!     uuid, which reads as "every agent was renamed overnight".
//!   * **`first_seen`** — the earliest sighting of each commit hash. A tail read
//!     sees only later ones, so the dates would creep forward run after run.
//!     [`crate::agents::keep_earliest`] is already written to make the merge and
//!     the scan agree; it just had nothing to merge FROM.
//!   * **per-agent `DaysSeen`** — the day sets behind each agent's weights. Only
//!     their union survives, in `memory-days.json`, and a union cannot be taken
//!     apart again. The weights decay by recency against the run's own `today`,
//!     so yesterday's decayed number cannot be merged into today's either: the
//!     raw days have to be kept.
//!
//! ⚠ **This is a CACHE, not a record.** Everything here is reproduced exactly by
//! one full mine, which is also the fallback whenever [`Plan::Full`] is chosen.
//! Losing it costs one slow night and nothing else, so it belongs beside the
//! other rebuildable artefacts rather than among the things that only exist
//! because somebody wrote them down.
//!
//! ⚠ **`transcript-drift.json` must NEVER be used for this.** That file is an
//! observatory: `transcript-drift` advances every `read_to` to the current end
//! each time it runs, to keep the append assumption under standing measurement.
//! A miner resuming from it would skip everything written between a drift run
//! and the next mine, silently and with no error — the exact failure
//! [`reader::watermark`] fails closed to avoid. The two files are separate so
//! that one tool writing cannot move the other tool's floor.
//!
//! [`Plan::Full`]: reader::watermark::Plan::Full

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use reader::watermark::Resume;

use crate::agents::{DaysSeen, FirstSeen};

/// Where the miner's own resume state lives, under the cache directory.
pub const FILE: &str = "mine-resume.json";

/// Everything a resumed mine must be handed to produce the whole-corpus answer.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Carried {
    /// The stamp of the run that wrote this, for reading a stale file's age.
    #[serde(default)]
    pub generated: String,
    /// How far each transcript was read, and the fold state open at that point.
    #[serde(default)]
    pub marks: BTreeMap<String, Resume>,
    /// Session id to the name its agent settled on.
    #[serde(default)]
    pub resolved: BTreeMap<String, String>,
    /// Earliest sighting of each commit hash.
    #[serde(default)]
    pub first_seen: FirstSeen,
    /// Per-agent day sets, keyed by agent name.
    #[serde(default)]
    pub days: BTreeMap<String, DaysSeen>,
    /// The roster **before** renames are applied.
    ///
    /// ⚠ **Not the roster from `agents.json`, and the difference is not
    /// cosmetic.** Git's rename map is applied on the way out, and it is NOT
    /// idempotent: the live history contains a 2-CYCLE — `docs/proposals/X.md`
    /// to `docs/proposals/archive/X.md` and back again, because the file was
    /// archived and later restored. Feeding an already-renamed roster back in
    /// flips those paths, and every resumed run would toggle them.
    ///
    /// So renames are treated the way commit attribution is: a derivation from
    /// raw state, recomputed each run, never accumulated. What is carried is the
    /// raw accumulation; `agents.json` keeps the renamed view.
    ///
    /// Found by the first full-corpus parity run, 2026-08-30. No fixture has a
    /// rename in it, let alone a cyclic one.
    #[serde(default)]
    pub agents: Vec<crate::agents::Agent>,
}

impl Carried {
    /// Read the state a previous run left.
    ///
    /// ⚠ **Absent is a first run; UNPARSEABLE is fatal.** Returning
    /// `Ok(None)` for a corrupt file would make "nothing to resume from" and
    /// "the resume state is damaged" the same answer — and the second one would
    /// then silently mine from offsets whose fold state was thrown away, which
    /// is a wrong resume rather than a slow one. Same rule, and the same reason,
    /// as [`crate::agents::carry_forward`].
    pub fn load(path: &std::path::Path) -> Result<Option<Self>> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        let carried = serde_json::from_str(&text)
            .with_context(|| format!("{} exists but will not parse", path.display()))?;
        Ok(Some(carried))
    }

    /// Write this run's state, atomically.
    pub fn save(&self, path: &std::path::Path) -> Result<()> {
        crate::atomic::write(path, &serde_json::to_vec_pretty(self)?)
    }
}
