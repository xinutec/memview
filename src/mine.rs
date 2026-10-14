//! What a resumed mine has to carry, and why the artefacts cannot supply it.
//!
//! The fold state a resumed mine starts from, which the artefacts do not hold: a
//! session's name (in its transcript's head), each commit's first sighting, and
//! each agent's day sets. A cache — one full mine reproduces it.
//!
//! Never resume from `transcript-drift.json`: that tool moves its own floor, and
//! a miner reading it would silently skip what was written in between.
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
    /// The roster before renames: git's rename map has a cycle, so re-applying it to
    /// a renamed roster would toggle paths every run.
    #[serde(default)]
    pub agents: Vec<crate::agents::Agent>,
}

impl Carried {
    /// Read the state a previous run left. Absent is a first run; unparseable is
    /// logged and treated as absent — a full mine, slow but right. Not fatal, unlike
    /// `memory-days.json`, which is not recomputable.
    pub fn load(path: &std::path::Path) -> Result<Option<Self>> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        match serde_json::from_str(&text) {
            Ok(carried) => Ok(Some(carried)),
            Err(why) => {
                eprintln!(
                    "⚠ {} will not parse ({why}) — reading the whole corpus instead",
                    path.display()
                );
                Ok(None)
            }
        }
    }

    /// Write this run's state, atomically.
    pub fn save(&self, path: &std::path::Path) -> Result<()> {
        crate::atomic::write(path, &serde_json::to_vec_pretty(self)?)
    }
}
