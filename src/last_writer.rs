//! Who last wrote each file — the one question `staged-check` asks, kept small
//! enough to live on this machine.
//!
//! ⚠ **This exists because `effects.json` LEFT.** The evidence it is folded from
//! is 70 MB and is now an export the nightly builds in a temp directory and
//! deletes (memview#1240), so the check that reads it could not run here at all.
//! Last-writer-per-path is the same answer at about a twentieth of the size,
//! because it keeps one row per path instead of every row.
//!
//! ⚠ **A fold with carried state, which is the bug family this repo has already
//! paid for.** The rule that keeps it right: a FULL mine builds from empty, a
//! RESUMED mine loads and absorbs the tail. Absorbing onto a stale map after a
//! full read would leave entries for paths the full read no longer mentions, and
//! the artefact would stop being a function of the corpus — which is exactly
//! what makes a from-scratch run the baseline a parity check can stand on.

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

/// Absolute path → its last recorded writer.
///
/// ⚠ **Keyed ABSOLUTELY**, like the rows it is folded from. A first attempt at
/// the check joined `repo/path` and matched nothing against real data while five
/// fixture tests passed, because the fixture agreed with the same wrong
/// assumption.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LastWriter(pub BTreeMap<String, Wrote>);

impl LastWriter {
    /// Fold a scan's WRITE rows on top of what is already known.
    ///
    /// ⚠ **`>=`, not `>`.** Rows arrive in order and a minute is coarse, so
    /// several writes to one path commonly share a stamp; taking the later of
    /// two equal stamps is what makes "last" mean last-seen rather than
    /// first-seen-in-that-minute.
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
            // ⚠ **An empty path is not a path.** The real artefact grew one
            // (`"" -> dev-lint`): harmless, since a lookup is always
            // `repo/path` and never empty, but an entry that cannot be
            // addressed is one a reader has to explain every time they see it.
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

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// ⚠ **Absent is `None`, never an empty map.** A caller that read a missing
    /// artefact as "nobody has written anything" would report all-clear from no
    /// evidence, which is worse than not running — the same distinction
    /// `fresh::effects` refuses on.
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
