//! Who last wrote each file — the one question `staged-check` asks, kept small
//! enough to live on this machine.
//!
//! One row per path, where `effects.json` is too big to keep. A full mine builds
//! from empty; only a resumed mine absorbs onto the carried map.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};

/// The artefact's name under the cache directory.
pub const FILE: &str = "last-writer.json";

/// The last recorded write of one path.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Wrote {
    /// The agent the evidence says wrote it.
    pub who: String,
    /// Minutes since the epoch, so a caller can say how recent the claim is.
    pub minute: i64,
}

/// Absolute path → its last recorded writer. Keyed absolutely, like its rows.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LastWriter(pub BTreeMap<String, Wrote>);

impl LastWriter {
    /// Fold a scan's write rows on top of what is known; `>=`, so the last of
    /// several writes in one minute wins.
    pub fn absorb(&mut self, effects: &reader::effects::Effects) {
        for row in &effects.rows {
            if !matches!(row.k, reader::effects::Did::Wrote) {
                continue;
            }
            let (Some(path), Some(who)) = (
                row.p.and_then(|p| effects.paths.get(p as usize)),
                effects.agents.get(row.a as usize),
            ) else {
                continue;
            };
            // An empty path is not a path.
            if path.is_empty() {
                continue;
            }
            match self.0.get_mut(path.as_str()) {
                Some(seen) if row.t >= seen.minute => {
                    seen.who = who.clone();
                    seen.minute = row.t;
                }
                Some(_) => {}
                None => {
                    self.0.insert(
                        path.clone(),
                        Wrote {
                            who: who.clone(),
                            minute: row.t,
                        },
                    );
                }
            }
        }
    }

    /// What the record says about one absolute path.
    pub fn who_wrote(&self, path: &str) -> Option<&Wrote> {
        self.0.get(path)
    }

    /// The agent last recorded writing a memory, by stem. `memory_dir` must be the
    /// logical `~/.claude` spelling, or every lookup misses silently.
    pub fn of_memory(&self, memory_dir: &str, memory: &str) -> Option<&str> {
        let dir = memory_dir.trim_end_matches('/');
        // The index is named in full; every other memory is a stem.
        let file = if memory.ends_with(".md") {
            memory.to_string()
        } else {
            format!("{memory}.md")
        };
        self.who_wrote(&format!("{dir}/{file}"))
            .map(|wrote| wrote.who.as_str())
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Absent is `None`, never an empty map that would read as all-clear.
    pub fn load(at: &Path) -> Result<Option<Self>> {
        if !at.exists() {
            return Ok(None);
        }
        let text =
            std::fs::read_to_string(at).with_context(|| format!("reading {}", at.display()))?;
        Ok(Some(
            serde_json::from_str(&text).with_context(|| format!("parsing {}", at.display()))?,
        ))
    }

    pub fn save(&self, at: &Path) -> Result<()> {
        crate::atomic::write(at, &serde_json::to_vec(self)?)
    }
}
