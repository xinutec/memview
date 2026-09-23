//! What has been written and not sent, per conversation, shared between devices.
//!
//! A draft is a record, not an instruction: it acts on nothing until a person
//! presses send, so it can be replicated freely.
//!
//! The runner holds it because both clients talk to this process. One way in,
//! `/api/sync/drafts`, in the shape life uses: two mechanisms writing one map is
//! how drafts diverge silently.
//!
//! A draft is a [`yrs`] document and a push is an update, which merges, so there
//! is no conflict to resolve. Two devices typing produce one text; the usual
//! disagreement is one device a few keystrokes behind the other.
//!
//! Text only. A picture is hundreds of kilobytes and there is no meaningful way
//! to combine two; it stays in the client's own storage.

use base64::Engine;
use parking_lot::RwLock;
use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use yrs::updates::decoder::Decode;
use yrs::{GetString, ReadTxn, Text, Transact};

/// The name of the shared text inside a draft's document. Both ends must agree on
/// it or each would read an empty string out of the other's writes.
pub const TEXT: &str = "text";

/// One conversation's unsent words, as the roster reports them.
///
/// A VIEW of the document, not the document: what the session list draws is the
/// sentence, and handing it the encoded state would put a CRDT somewhere that
/// only wants a string.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Draft {
    /// What is being typed, read out of the merged document.
    pub text: String,
    /// Bumped on every merge, and used for ONE thing: ordering a pull. It counts
    /// across the WHOLE store, not per conversation — see [`Drafts::merge`].
    pub rev: u64,
    /// Unix milliseconds of the last merge that changed the text.
    pub at: u64,
}

/// One draft on the wire: the merged document, and the text it reads as.
///
/// `ulid` is the SESSION id — a draft is one per conversation, and the
/// conversation already has a stable identity.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct DraftDoc {
    pub ulid: String,
    /// The whole document state, base64. Applying it is idempotent and
    /// order-independent, which is the whole reason this design has no conflicts: a
    /// client that applies it twice, or applies it after its own newer edit,
    /// converges either way.
    pub update: String,
    /// What [`Self::update`] reads as, for anything that wants the sentence rather
    /// than the document. Never read as input — the document is the truth.
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

/// One change from a client: an update to merge into whatever is held.
///
/// No assumed state, and nothing to refuse. An update carries its own
/// causal context, so the runner never has to be told what the client thought was
/// here.
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
    /// The merged state, encoded. Kept encoded rather than as a live document
    /// because that is what goes to disk and to clients, and decoding per write
    /// costs nothing at a draft's size.
    state: Vec<u8>,
    text: String,
    rev: u64,
    at: u64,
}

/// What is written to disk: [`Held`] with the state in base64, so the file stays
/// readable text like every other the console keeps.
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

/// The document an encoded update reads as, or `None` if it is not one.
///
/// A bad update is dropped, never fatal. It arrives over the wire from a
/// client that may be older than this binary, and refusing to start — or
/// poisoning the store — would turn one malformed push into an outage for every
/// conversation.
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

/// Bytes out of base64, or `None` — the same tolerance as [`decoded`], for the
/// same reason.
pub fn from_base64(text: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::STANDARD.decode(text).ok()
}

/// A document holding `text`, encoded — for a caller that has words rather than a
/// document, which is every test and nothing in production.
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
    /// Read what the last run wrote. An unreadable file is an empty set and a loud
    /// line, as in [`crate::modes::Modes::load`]: the cost is a draft missing from the
    /// OTHER device, which still holds its own copy.
    pub fn load(store: PathBuf) -> Self {
        let held = match std::fs::read_to_string(&store) {
            Ok(text) => match serde_json::from_str::<BTreeMap<String, Stored>>(&text) {
                Ok(held) => held
                    .into_iter()
                    .filter_map(|(id, one)| {
                        // A row that will not decode is dropped rather than kept as an empty
                        // draft, which would look like somebody had cleared it.
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

    /// Every draft, for the roster — a client that has just connected learns which
    /// conversations hold unsent words without asking per session.
    pub fn all(&self) -> BTreeMap<String, Draft> {
        self.held
            .read()
            .iter()
            .map(|(id, held)| (id.clone(), held.seen()))
            .collect()
    }

    /// Merge a device's document into what is held, and answer with the result.
    ///
    /// This cannot fail on a disagreement, and that is the point. Two updates
    /// written without knowledge of each other merge into one document holding both
    /// edits, and applying the same update twice changes nothing. There is no state a
    /// caller can be in that this has to refuse.
    ///
    /// `None` only when the bytes are not a document at all.
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
            // One counter for the whole STORE: the pull cursor is one number across the
            // collection, and a per-document counter left a fresh conversation at rev 1
            // behind a client already at 3 — never synced, and random-looking from outside.
            let rev = held.values().map(|one| one.rev).max().unwrap_or(0) + 1;
            // A merge that changed no character does not move the clock the list dates a
            // draft by: a device can contribute history without anybody having typed.
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

    /// Every draft past `since`, oldest revision first, with the new checkpoint. Not
    /// paged: a sentence per conversation cannot outgrow one response.
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

    /// Merge everything a client sent, and answer with the merged documents.
    ///
    /// Always the merged state, never a refusal. The client applies what comes
    /// back and is then level with the runner.
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

    /// Written whole each time — a short line per conversation, and a rewrite cannot
    /// leave a half-updated entry.
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
