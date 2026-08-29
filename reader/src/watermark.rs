//! How much of a transcript has already been read, and whether it is still the
//! same file underneath.
//!
//! ⚠ **A whole-corpus fold cannot be made cheap by parsing faster.** Measured
//! 2026-08-28: the mine takes 347 s over 5.9 GB, and removing its entire
//! shell-parsing arm leaves 241 s. Nothing per-operation reaches seconds — only
//! reading less does, and the corpus is shaped for it: the ten largest
//! transcripts hold 90% of the bytes and they only grow at the tail
//! (memview#1240).
//!
//! ⚠ **Resuming is only sound because the CLI APPENDS.** It writes earlier
//! stretches of a conversation back into the same file, which reads like history
//! being rewritten — but the copies are appended, a median 21 MB apart, and the
//! prefix does not move (`reference_claude_transcript_rewrites_history`). This
//! module exists to keep checking that rather than trusting it: a resume that is
//! wrong reads no error, it silently mines a file from the wrong offset.
//!
//! ⚠ **And the miner does not dedup by message uuid**, so a resumed scan sees
//! exactly the lines a full scan sees, in the same order. Parity needs no
//! carried set of ids.

use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use serde::{Deserialize, Serialize};

/// How much of one transcript has been consumed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Watermark {
    /// Bytes consumed — where a resumed read begins.
    pub read_to: u64,
    /// A fingerprint of the bytes just before `read_to`.
    ///
    /// ⚠ **A window, not the whole prefix.** Hashing everything already read
    /// costs exactly what resuming is meant to save. This catches the file being
    /// truncated, replaced, or rewritten near the boundary — which is what a
    /// wrong offset looks like — and is explicitly not proof that a byte a
    /// gigabyte back is untouched. [`Drift::Rewritten`] is the fallback when it
    /// fails, and re-reading whole is always correct.
    pub tail_sha: String,
}

/// How many bytes the fingerprint covers.
pub const WINDOW: u64 = 64 * 1024;

/// One transcript's resume record: where the read stopped, **and the fold state
/// the next read has to start from**.
///
/// ⚠ **The offset alone is not enough, and the shortfall is measurable.** An
/// episode is bracketed by a user's turn, so a cut taken while an instruction is
/// still being carried out orphans every row until the next prompt. Measured
/// 2026-08-29 against a real 21:38 watermark: **66 of 2,815 tail calls** would
/// land in no episode. The comparable loss from *not* carrying a call's pending
/// result was **3 calls** — twenty times smaller, and twenty times more
/// expensive to fix, which is why one is carried here and the other is not
/// (`crate::doing::Log::resume`).
///
/// ⚠ **The fold fields default**, so a `transcript-drift.json` written before
/// they existed still parses as offsets with no episode open — which is exactly
/// what a run that never carried one had.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resume {
    #[serde(flatten)]
    pub mark: Watermark,
    /// Index into `Doing::episodes` for the instruction still being carried out
    /// at `read_to`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode: Option<u32>,
    /// An agent whose prompt was seen but which had done no work yet — the
    /// episode is materialised by its first row, so before that there is only a
    /// name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
}

impl Resume {
    /// A first read of a file: an offset with nothing open above it.
    pub fn fresh(mark: Watermark) -> Self {
        Self {
            mark,
            episode: None,
            prompt: None,
        }
    }
}

/// What a second look at a file found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Drift {
    /// Same prefix, nothing appended. Nothing to do.
    Unchanged,
    /// Same prefix, more bytes after it — resume at `read_to`.
    Grew { by: u64 },
    /// Shorter than it was: the file was truncated or replaced.
    Shrank,
    /// The window before `read_to` is not what it was, so the offset means
    /// something else now. Read the whole file.
    Rewritten,
    /// Never seen, or unreadable now.
    Unknown,
}

impl Drift {
    /// Can the reader start at `read_to`, or must it start over?
    pub fn resumable(&self) -> bool {
        matches!(self, Drift::Unchanged | Drift::Grew { .. })
    }
}

fn sha_of(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    // `sha2` 0.11's output is a generic array with no `LowerHex`, so the hex is
    // written out — the same shape `console/src/tls.rs` uses for key
    // fingerprints.
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Fingerprint the last [`WINDOW`] bytes before `read_to`.
pub fn window_at(path: &Path, read_to: u64) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let from = read_to.saturating_sub(WINDOW);
    file.seek(SeekFrom::Start(from)).ok()?;
    let mut buf = vec![0u8; (read_to - from) as usize];
    file.read_exact(&mut buf).ok()?;
    Some(sha_of(&buf))
}

/// Record where a file has been read to.
pub fn observe(path: &Path) -> Option<Watermark> {
    let read_to = std::fs::metadata(path).ok()?.len();
    Some(Watermark {
        read_to,
        tail_sha: window_at(path, read_to)?,
    })
}

/// Compare a file against what was recorded for it.
pub fn drift(path: &Path, mark: &Watermark) -> Drift {
    let Ok(meta) = std::fs::metadata(path) else {
        return Drift::Unknown;
    };
    let now = meta.len();
    if now < mark.read_to {
        return Drift::Shrank;
    }
    // ⚠ The window is taken at the RECORDED offset, not at the current end —
    // the question is whether the bytes already consumed still say what they
    // said, and hashing the new end would answer a different question and
    // always disagree.
    match window_at(path, mark.read_to) {
        Some(sha) if sha == mark.tail_sha => {
            if now == mark.read_to {
                Drift::Unchanged
            } else {
                Drift::Grew {
                    by: now - mark.read_to,
                }
            }
        }
        Some(_) => Drift::Rewritten,
        None => Drift::Unknown,
    }
}

/// What a run may do, given what it read last time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// Read every transcript whole and DISCARD the carried artefacts.
    ///
    /// ⚠ **All-or-nothing, and that is forced by the artefacts rather than
    /// chosen.** Their rows carry no per-transcript provenance, so one file that
    /// cannot be resumed cannot have its old contribution subtracted — it would
    /// be counted once from the carried artefact and again from the re-read. A
    /// full re-mine is the only sound answer, and it is cheap because it is
    /// rare: `transcript-drift` has reported "no prefix moved" run after run.
    Full { because: String },
    /// Keep the carried artefacts and read only what is listed.
    Resume {
        /// Transcripts that grew, and where to start in each.
        tails: BTreeMap<String, Resume>,
        /// Transcripts nothing has read before — new since the last run, so they
        /// are read whole WITHOUT invalidating anything carried.
        whole: Vec<String>,
        /// Transcripts the last run saw and that are gone now.
        ///
        /// ⚠ **Carried, not a reason to re-mine.** A vanished transcript's rows
        /// are history and stay; `carry_forward` already treats memory-days this
        /// way deliberately. Forcing a full re-mine on one would mean a full
        /// re-mine most days — 343 transcripts disappeared in 22 days, nearly
        /// all of them `/private/tmp` scratch — which is the whole saving, gone
        /// for bookkeeping.
        gone: Vec<String>,
    },
}

/// Decide what this run may do.
///
/// `marks` is what the last run recorded; `present` is what is on disk now.
///
/// ⚠ **Fails CLOSED.** Anything not provably an append — a rewritten prefix, a
/// file shorter than it was, a file that cannot be read — returns [`Plan::Full`]
/// with the reason, because a wrong resume produces no error at all: it mines
/// from an offset that means something else and the artefact simply becomes
/// quietly untrue.
pub fn plan(marks: &BTreeMap<String, Resume>, present: &[std::path::PathBuf]) -> Plan {
    let mut tails = BTreeMap::new();
    let mut whole = Vec::new();
    let here: std::collections::BTreeSet<String> = present
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();

    for path in present {
        let key = path.to_string_lossy().into_owned();
        let Some(held) = marks.get(&key) else {
            whole.push(key);
            continue;
        };
        match drift(path, &held.mark) {
            Drift::Unchanged => {}
            Drift::Grew { .. } => {
                tails.insert(key, held.clone());
            }
            Drift::Rewritten => {
                return Plan::Full {
                    because: format!("{key} was rewritten before its recorded offset"),
                };
            }
            Drift::Shrank => {
                return Plan::Full {
                    because: format!("{key} is shorter than it was"),
                };
            }
            Drift::Unknown => {
                return Plan::Full {
                    because: format!("{key} could not be read"),
                };
            }
        }
    }

    let gone = marks
        .keys()
        .filter(|key| !here.contains(*key))
        .cloned()
        .collect();
    Plan::Resume { tails, whole, gone }
}
