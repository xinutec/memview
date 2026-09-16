//! What has been written and not sent, per conversation, shared between devices.
//!
//! ⚠ **A draft is a RECORD, not an instruction, and that is what separates this
//! from the queued send memview #90 refused.** A message handed to a session is
//! an instruction, and one delivered minutes later — after the conversation has
//! moved on — is not what was meant. A draft leaves nothing until a person
//! presses send, so it stays true until it is changed and can be replicated
//! without ever acting on its own.
//!
//! **Why the runner holds it.** Both clients talk to this process: the browser
//! on the Mac and the phone over the tunnel. There is no second server to
//! reconcile against, so a draft written on one device is on the other as soon
//! as it is pushed, and the only conflict is a genuine one — both edited while
//! a device was away.
//!
//! **One way in: `/api/sync/drafts`**, pull and push, in the shape life uses.
//! ⚠ **One way in, and it stays that way.** Two mechanisms writing one map is
//! how drafts diverge silently; this is the one a replication library drives.
//!
//! **Text only.** A draft can also carry a scaled screenshot, which is hundreds
//! of kilobytes of base64 against a sentence's few hundred bytes; there is no
//! meaningful way to combine two images, and the device that took one is
//! usually the device that wants it. The picture stays in the client's own
//! storage. Reopen that if a picture is ever wanted on the other screen —
//! nothing here assumes it never will be.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::RwLock;

use serde::{Deserialize, Serialize};

/// One conversation's unsent words.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Draft {
    /// What was being typed.
    pub text: String,
    /// Bumped on every accepted write, and used for ONE thing: ordering a pull.
    /// A client's checkpoint is the highest `rev` it has been handed, so this
    /// counts across the WHOLE store rather than per conversation — see
    /// [`Drafts::apply`], where a per-document counter silently stranded every
    /// draft written after another conversation had got ahead.
    ///
    /// ⚠ **It is NOT what a conflict is judged on** — see [`Drafts::apply`],
    /// which compares the text, and says why a revision cannot.
    pub rev: u64,
    /// Unix milliseconds.
    ///
    /// ⚠ **The only thing that distinguishes the two drafts, and deliberately
    /// so.** A field naming the writing device was tried and removed: whoever is
    /// choosing is standing at one of the two, so "the other one" needs no name,
    /// and the useful question is which thought is newer.
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

/// One draft as it travels over sync — RxDB's document shape, mirroring life's
/// `src/sync/types.rs` rather than inventing a second protocol.
///
/// ⚠ **`ulid` is the SESSION id.** Every other collection in the fleet mints one
/// per row; a draft is one per conversation and the conversation already has a
/// stable identity, so minting a second would be an identity nothing else could
/// join on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftDoc {
    pub ulid: String,
    pub text: String,
    pub at: u64,
    /// RxDB's tombstone flag. A cleared draft is a live document with empty text
    /// rather than a deletion — see [`Drafts::apply`] for why the entry has to
    /// survive — so this is written `false` and read for protocol conformance.
    #[serde(rename = "_deleted", default)]
    pub deleted: bool,
    /// Server revision, for the pull cursor. Ignored as push input; set here.
    #[serde(default)]
    pub rev: u64,
}

/// What a pull answers: the rows past the caller's checkpoint, and the new one.
#[derive(Debug, Serialize)]
pub struct PullResponse {
    pub documents: Vec<DraftDoc>,
    pub checkpoint: Checkpoint,
}

/// The pull cursor: the highest `rev` delivered so far.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Checkpoint {
    pub rev: u64,
}

/// One change from a client: the state it wants, and the state it assumed —
/// `None` for a fresh insert. The assumed state is what makes the conflict
/// detectable rather than the last writer silently winning. Only its `text` is
/// read; [`Drafts::apply`] says why its `rev` cannot be.
#[derive(Debug, Deserialize)]
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
    /// Read what the last run wrote.
    ///
    /// An unreadable file is an empty set and a loud line, not an error, for the
    /// reason [`crate::modes::Modes::load`] gives: a console that will not start
    /// because it cannot remember a preference is worse than one that starts and
    /// asks. Here the cost of the empty case is that a draft is missing from the
    /// OTHER device — the device that typed it still holds its own copy.
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
        self.held.read().expect("drafts poisoned").get(id).cloned()
    }

    /// Every draft, for the roster — a client that has just connected learns
    /// which conversations hold unsent words without asking per session.
    pub fn all(&self) -> BTreeMap<String, Draft> {
        self.held.read().expect("drafts poisoned").clone()
    }

    /// Record what a device is holding, if what it assumed is what is here.
    ///
    /// ⚠ **`assumed` is the TEXT the edit was made against, never the
    /// revision.** A revision is minted here, so a client learns its own new one
    /// only on the next PULL — until then it still believes the one it edited
    /// from. Judging on revisions therefore refuses the second keystroke inside
    /// a pull interval and calls it a conflict, which is ordinary typing. The
    /// text answers the question actually being asked: has somebody else changed
    /// this since you last saw it.
    ///
    /// `None` assumes there is nothing here; against an existing draft that is a
    /// device overwriting words it has never seen, so it conflicts.
    ///
    /// ⚠ **A cleared draft is a TOMBSTONE, not a removal.** Erasing the entry
    /// lets the other device, still holding the words, push them back and
    /// resurrect a message already sent.
    pub fn apply(&self, id: &str, text: &str, assumed: Option<&str>, at: u64) -> Wrote {
        let (result, all) = {
            let mut held = self.held.write().expect("drafts poisoned");
            let current = held.get(id).cloned();
            if let Some(theirs) = current.as_ref()
                && assumed != Some(theirs.text.as_str())
            {
                return Wrote::Conflict(theirs.clone());
            }
            // ⚠ **One counter for the whole STORE, not one per conversation.**
            // The pull cursor is a single number across the collection, so a
            // per-document counter cannot serve it: a draft written in a fresh
            // conversation would take rev 1 while a client that has already
            // pulled another sits at 3, and `rev > since` then never matches it.
            // That conversation never syncs — not late, never — and it looks
            // random from the outside, because whether it bites depends on
            // what OTHER conversations have been typed in.
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

    /// Drop the drafts of conversations that are no longer on disk.
    ///
    /// Same argument as [`crate::gist::Gists::forget`]: a map keyed by id that
    /// is only ever written to grows forever. An empty `alive` is a sweep that
    /// found nothing and is not evidence that everything has gone.
    pub fn forget(&self, alive: &std::collections::BTreeSet<String>) {
        if alive.is_empty() {
            return;
        }
        let all = {
            let mut held = self.held.write().expect("drafts poisoned");
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

    /// Written whole each time — a short line per conversation, like
    /// [`crate::modes::Modes`], and a rewrite cannot leave a half-updated entry.
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
    /// Every draft past `since`, oldest revision first, with the new checkpoint.
    ///
    /// ⚠ **Ordered by revision and NOT paged.** A page is what a checkpoint
    /// protocol needs when a collection can outgrow one response; this one holds
    /// a sentence per conversation, so a limit would be machinery guarding
    /// against a size this cannot reach. Revisit if a draft ever carries the
    /// picture.
    pub fn pull(&self, since: u64) -> PullResponse {
        let held = self.held.read().expect("drafts poisoned");
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

    /// Apply a batch, and answer with the documents that lost.
    ///
    /// ⚠ **An empty answer means every entry landed**, which is RxDB's contract
    /// and the opposite of an HTTP status: a push that conflicts is a successful
    /// request carrying the current master, not a failed one. The client resolves
    /// and pushes again.
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
                        // ⚠ **The one outcome nobody can diagnose from a
                        // screen.** A clash says two devices wrote; what says
                        // whether that is TRUE is which texts were compared, and
                        // that was guessed at from the symptom twice before this
                        // line existed. Lengths rather than words — enough to
                        // tell two drafts apart, and a conversation does not
                        // belong in a log.
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
