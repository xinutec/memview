//! What has been written and not sent, per conversation, shared between devices.
//!
//! A draft is a RECORD, not an instruction: it acts on nothing until a person
//! presses send, so it can be replicated freely — unlike the queued send memview
//! #90 refused, which would deliver an instruction minutes after it was meant.
//!
//! The runner holds it because both clients talk to this process; the only
//! conflict is a genuine one, both edited while a device was away. One way in,
//! `/api/sync/drafts`, in the shape life uses: two mechanisms writing one map is
//! how drafts diverge silently.
//!
//! Text only. A picture is hundreds of kilobytes and there is no meaningful way
//! to combine two; it stays in the client's own storage.

use parking_lot::RwLock;
use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// One conversation's unsent words.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Draft {
    /// What was being typed.
    pub text: String,
    /// Bumped on every accepted write, and used for ONE thing: ordering a pull. It
    /// counts across the WHOLE store, not per conversation — see [`Drafts::apply`].
    /// It is NOT what a conflict is judged on; the text is.
    pub rev: u64,
    /// Unix milliseconds. The only thing distinguishing the two drafts, deliberately:
    /// whoever is choosing stands at one of the two devices, and the useful question
    /// is which thought is newer.
    pub at: u64,
}

/// What a write did, which is what the client draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Wrote {
    /// Stored. The new state is the caller's own text at the returned revision.
    Stored(Draft),
    /// Refused: somebody else has written since the revision this edit was made
    /// from. Carries THEIRS, because the client cannot resolve without it.
    Conflict(Draft),
}

/// One draft as it travels over sync — RxDB's document shape, as in life's
/// `src/sync/types.rs`. `ulid` is the SESSION id: a draft is one per conversation,
/// and the conversation already has a stable identity.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct DraftDoc {
    pub ulid: String,
    pub text: String,
    pub at: u64,
    /// RxDB's tombstone flag. A cleared draft is a live document with empty text, not
    /// a deletion — see [`Drafts::apply`] — so this is written `false`.
    #[serde(rename = "_deleted", default)]
    pub deleted: bool,
    /// Server revision, for the pull cursor. Ignored as push input; set here.
    #[serde(default)]
    pub rev: u64,
}

/// What a pull answers: the rows past the caller's checkpoint, and the new one.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct PullResponse {
    pub documents: Vec<DraftDoc>,
    pub checkpoint: Checkpoint,
}

/// The pull cursor: the highest `rev` delivered so far.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Checkpoint {
    pub rev: u64,
}

/// One change from a client: the state it wants, and the state it assumed —
/// `None` for a fresh insert. Only the assumed `text` is read; [`Drafts::apply`]
/// says why its `rev` cannot be.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct PushEntry {
    #[serde(rename = "newDocumentState")]
    pub new_document_state: DraftDoc,
    #[serde(rename = "assumedMasterState", default)]
    pub assumed_master_state: Option<DraftDoc>,
}

/// Every conversation's unsent words, by session id.
#[derive(Debug)]
pub struct Drafts {
    store: PathBuf,
    held: RwLock<BTreeMap<String, Draft>>,
}

impl Drafts {
    /// Read what the last run wrote. An unreadable file is an empty set and a loud
    /// line, as in [`crate::modes::Modes::load`]: the cost is a draft missing from the
    /// OTHER device, which still holds its own copy.
    pub fn load(store: PathBuf) -> Self {
        let held = match std::fs::read_to_string(&store) {
            Ok(text) => match serde_json::from_str(&text) {
                Ok(held) => held,
                Err(why) => {
                    tracing::error!(
                        "drafts: {} will not parse ({why}) — unsent words will not cross devices",
                        store.display()
                    );
                    BTreeMap::new()
                }
            },
            Err(_) => BTreeMap::new(),
        };
        Self {
            store,
            held: RwLock::new(held),
        }
    }

    /// This conversation's draft, if any device has written one.
    pub fn get(&self, id: &str) -> Option<Draft> {
        self.held.read().get(id).cloned()
    }

    /// Every draft, for the roster — a client that has just connected learns which
    /// conversations hold unsent words without asking per session.
    pub fn all(&self) -> BTreeMap<String, Draft> {
        self.held.read().clone()
    }

    /// Record what a device is holding, if what it assumed is what is here.
    ///
    /// `assumed` is the TEXT the edit was made against, never the revision: a client
    /// learns its new revision only on the next pull, so judging on revisions calls
    /// the second keystroke in a pull interval a conflict. `None` assumes nothing is
    /// here, so against an existing draft it conflicts.
    ///
    /// A cleared draft is a TOMBSTONE, not a removal: erasing the entry lets the other
    /// device push the words back and resurrect a message already sent.
    pub fn apply(&self, id: &str, text: &str, assumed: Option<&str>, at: u64) -> Wrote {
        let (result, all) = {
            let mut held = self.held.write();
            let current = held.get(id).cloned();
            if let Some(theirs) = current.as_ref()
                && assumed != Some(theirs.text.as_str())
            {
                return Wrote::Conflict(theirs.clone());
            }
            // One counter for the whole STORE: the pull cursor is one number across the
            // collection, and a per-document counter left a fresh conversation at rev 1
            // behind a client already at 3 — never synced, and random-looking from outside.
            let next = Draft {
                text: text.to_string(),
                rev: held.values().map(|d| d.rev).max().unwrap_or(0) + 1,
                at,
            };
            held.insert(id.to_string(), next.clone());
            (Wrote::Stored(next), held.clone())
        };
        self.write(&all);
        result
    }

    /// Drop the drafts of conversations no longer on disk. An empty `alive` is a
    /// sweep that found nothing, not evidence that everything has gone.
    pub fn forget(&self, alive: &std::collections::BTreeSet<String>) {
        if alive.is_empty() {
            return;
        }
        let all = {
            let mut held = self.held.write();
            let before = held.len();
            held.retain(|id, _| alive.contains(id));
            if held.len() == before {
                return;
            }
            tracing::info!(
                "drafts: {} conversation(s) gone from disk, forgetting their unsent words",
                before - held.len()
            );
            held.clone()
        };
        self.write(&all);
    }

    /// Written whole each time — a short line per conversation, and a rewrite cannot
    /// leave a half-updated entry.
    fn write(&self, all: &BTreeMap<String, Draft>) {
        if let Ok(text) = serde_json::to_string_pretty(all)
            && let Some(dir) = self.store.parent()
        {
            let _ = std::fs::create_dir_all(dir);
            if let Err(why) = std::fs::write(&self.store, text) {
                tracing::warn!("drafts: could not write {}: {why}", self.store.display());
            }
        }
    }
}

impl Drafts {
    /// Every draft past `since`, oldest revision first, with the new checkpoint. Not
    /// paged: a sentence per conversation cannot outgrow one response. Revisit if a
    /// draft ever carries the picture.
    pub fn pull(&self, since: u64) -> PullResponse {
        let held = self.held.read();
        let mut documents: Vec<DraftDoc> = held
            .iter()
            .filter(|(_, d)| d.rev > since)
            .map(|(id, d)| DraftDoc {
                ulid: id.clone(),
                text: d.text.clone(),
                at: d.at,
                deleted: false,
                rev: d.rev,
            })
            .collect();
        documents.sort_by_key(|d| d.rev);
        let rev = documents.last().map_or(since, |d| d.rev);
        PullResponse {
            documents,
            checkpoint: Checkpoint { rev },
        }
    }

    /// Apply a batch, and answer with the documents that lost. An empty answer means
    /// every entry landed — RxDB's contract, the opposite of an HTTP status: a
    /// conflicting push is a successful request carrying the current master.
    pub fn push(&self, entries: Vec<PushEntry>) -> Vec<DraftDoc> {
        entries
            .into_iter()
            .filter_map(|entry| {
                let id = entry.new_document_state.ulid.clone();
                let assumed = entry.assumed_master_state.as_ref().map(|d| d.text.as_str());
                match self.apply(
                    &id,
                    &entry.new_document_state.text,
                    assumed,
                    entry.new_document_state.at,
                ) {
                    Wrote::Stored(_) => None,
                    Wrote::Conflict(theirs) => {
                        // The one outcome nobody can diagnose from a screen: which texts were compared.
                        // Lengths rather than words — a conversation does not belong in a log.
                        tracing::info!(
                            "{id}: refused a draft push assuming {} char(s), holding {} at rev {}",
                            assumed.map_or(0, |t| t.chars().count()),
                            theirs.text.chars().count(),
                            theirs.rev,
                        );
                        Some(DraftDoc {
                            ulid: id,
                            text: theirs.text,
                            at: theirs.at,
                            deleted: false,
                            rev: theirs.rev,
                        })
                    }
                }
            })
            .collect()
    }
}
