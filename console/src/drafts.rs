//! What has been written and not sent, per conversation, shared between devices.
//!
//! A draft is a [`yrs`] document and a push an update, which merges: there is no
//! conflict to resolve. One way in, `/api/sync/drafts`, shaped as the life app's sync is. Text only;
//! a picture stays in the client.

use base64::Engine;
use parking_lot::RwLock;
use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use yrs::updates::decoder::Decode;
use yrs::{GetString, ReadTxn, Text, Transact};

/// The name of the shared text inside a draft's document; both ends must agree.
pub const TEXT: &str = "text";

/// One conversation's unsent words, as the roster reports them — the text, not the
/// document.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Draft {
    /// What is being typed, read out of the merged document.
    pub text: String,
    /// The store-wide revision, for ordering a pull — see [`Drafts::merge`].
    pub rev: u64,
    /// Unix milliseconds of the last merge that changed the text.
    pub at: u64,
}

/// One draft on the wire: the merged document and its text. `ulid` is the session id.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct DraftDoc {
    pub ulid: String,
    /// The whole document state, base64 — idempotent and order-independent to apply.
    pub update: String,
    /// What [`Self::update`] reads as. Never input.
    #[serde(default)]
    pub text: String,
    pub at: u64,
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

/// One change from a client: an update carrying its own causal context.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct PushEntry {
    pub ulid: String,
    /// The client's document state, base64.
    pub update: String,
    pub at: u64,
}

/// One conversation's document, as it is kept.
#[derive(Debug, Clone)]
struct Held {
    /// The merged state, encoded as disk and clients take it.
    state: Vec<u8>,
    text: String,
    rev: u64,
    at: u64,
}

/// What is written to disk: [`Held`] with the state in base64.
#[derive(Debug, Serialize, Deserialize)]
struct Stored {
    update: String,
    rev: u64,
    at: u64,
}

/// Every conversation's unsent words, by session id.
#[derive(Debug)]
pub struct Drafts {
    store: PathBuf,
    held: RwLock<BTreeMap<String, Held>>,
}

/// The document an encoded update reads as, or `None`: a bad push is dropped,
/// never fatal.
fn decoded(update: &[u8]) -> Option<yrs::Doc> {
    let doc = yrs::Doc::new();
    let update = yrs::Update::decode_v1(update).ok()?;
    doc.transact_mut().apply_update(update).ok()?;
    Some(doc)
}

/// The text inside a document.
fn reads_as(doc: &yrs::Doc) -> String {
    let text = doc.get_or_insert_text(TEXT);
    let txn = doc.transact();
    text.get_string(&txn)
}

/// A document's whole state, which is what both disk and the wire carry.
fn encoded(doc: &yrs::Doc) -> Vec<u8> {
    doc.transact()
        .encode_state_as_update_v1(&yrs::StateVector::default())
}

fn to_base64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Bytes out of base64, or `None`.
pub fn from_base64(text: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::STANDARD.decode(text).ok()
}

/// A document holding `text`, encoded — for tests.
pub fn document_of(text: &str) -> Vec<u8> {
    let doc = yrs::Doc::new();
    let shared = doc.get_or_insert_text(TEXT);
    {
        let mut txn = doc.transact_mut();
        shared.insert(&mut txn, 0, text);
    }
    encoded(&doc)
}

impl Drafts {
    /// Read what the last run wrote; unreadable is empty, and logged.
    pub fn load(store: PathBuf) -> Self {
        let held = match std::fs::read_to_string(&store) {
            Ok(text) => match serde_json::from_str::<BTreeMap<String, Stored>>(&text) {
                Ok(held) => held
                    .into_iter()
                    .filter_map(|(id, one)| {
                        // Dropped, not kept empty, which would read as cleared.
                        let state = from_base64(&one.update)?;
                        let text = reads_as(&decoded(&state)?);
                        Some((
                            id,
                            Held {
                                state,
                                text,
                                rev: one.rev,
                                at: one.at,
                            },
                        ))
                    })
                    .collect(),
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
        self.held.read().get(id).map(Held::seen)
    }

    /// Every draft, for the roster.
    pub fn all(&self) -> BTreeMap<String, Draft> {
        self.held
            .read()
            .iter()
            .map(|(id, held)| (id.clone(), held.seen()))
            .collect()
    }

    /// Merge a device's document into what is held and answer with the result —
    /// never refused; `None` only for bytes that are not a document.
    pub fn merge(&self, id: &str, update: &[u8], at: u64) -> Option<Draft> {
        let update = yrs::Update::decode_v1(update).ok()?;
        let (one, all) = {
            let mut held = self.held.write();
            let doc = match held.get(id) {
                Some(mine) => decoded(&mine.state)?,
                None => yrs::Doc::new(),
            };
            doc.transact_mut().apply_update(update).ok()?;
            let text = reads_as(&doc);
            // Store-wide: the pull cursor is one number across all drafts.
            let rev = held.values().map(|one| one.rev).max().unwrap_or(0) + 1;
            // Only a changed text moves the draft's date.
            let at = match held.get(id) {
                Some(was) if was.text == text => was.at,
                _ => at,
            };
            let one = Held {
                state: encoded(&doc),
                text,
                rev,
                at,
            };
            held.insert(id.to_string(), one.clone());
            (one, held.clone())
        };
        self.write(&all);
        Some(one.seen())
    }

    /// Drop the drafts of conversations no longer on disk; an empty `alive` drops nothing.
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

    /// Every draft past `since`, oldest revision first, with the new checkpoint.
    pub fn pull(&self, since: u64) -> PullResponse {
        let held = self.held.read();
        let mut documents: Vec<DraftDoc> = held
            .iter()
            .filter(|(_, one)| one.rev > since)
            .map(|(id, one)| one.wire(id))
            .collect();
        documents.sort_by_key(|one| one.rev);
        let rev = documents.last().map_or(since, |one| one.rev);
        PullResponse {
            documents,
            checkpoint: Checkpoint { rev },
        }
    }

    /// Merge everything a client sent and answer with the merged documents.
    pub fn push(&self, entries: Vec<PushEntry>) -> Vec<DraftDoc> {
        entries
            .into_iter()
            .filter_map(|entry| {
                let Some(update) = from_base64(&entry.update) else {
                    tracing::info!("{}: a draft push that is not base64", entry.ulid);
                    return None;
                };
                if self.merge(&entry.ulid, &update, entry.at).is_none() {
                    tracing::info!("{}: a draft push that is not a document", entry.ulid);
                    return None;
                }
                self.held
                    .read()
                    .get(&entry.ulid)
                    .map(|one| one.wire(&entry.ulid))
            })
            .collect()
    }

    /// Written whole each time.
    fn write(&self, all: &BTreeMap<String, Held>) {
        let stored: BTreeMap<&String, Stored> = all
            .iter()
            .map(|(id, one)| {
                (
                    id,
                    Stored {
                        update: to_base64(&one.state),
                        rev: one.rev,
                        at: one.at,
                    },
                )
            })
            .collect();
        if let Ok(text) = serde_json::to_string_pretty(&stored)
            && let Some(dir) = self.store.parent()
        {
            let _ = std::fs::create_dir_all(dir);
            if let Err(why) = std::fs::write(&self.store, text) {
                tracing::warn!("drafts: could not write {}: {why}", self.store.display());
            }
        }
    }
}

impl Held {
    fn seen(&self) -> Draft {
        Draft {
            text: self.text.clone(),
            rev: self.rev,
            at: self.at,
        }
    }

    fn wire(&self, id: &str) -> DraftDoc {
        DraftDoc {
            ulid: id.to_string(),
            update: to_base64(&self.state),
            text: self.text.clone(),
            at: self.at,
            rev: self.rev,
        }
    }
}
