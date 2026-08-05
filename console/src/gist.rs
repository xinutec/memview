//! One sentence about what each conversation is currently about.
//!
//! The list can say a conversation's name, how warm it is and how full it is,
//! and none of that says what it is *for*. The name is a word; the instruction it
//! was started with drifts out of date within a day — measured on this console's
//! own sessions, where the first prompt in view was `push`, `Proceed`, and the
//! boilerplate a compaction writes. So the sentence is written now, from the
//! conversation as it currently stands.
//!
//! ## Written by a model, and said to be
//!
//! ⚠ **This is inference, and the client marks it as such.** Everything else on
//! that page is read off a file or off the process; this is a guess made by
//! Haiku from a few thousand characters of transcript, and a confidently wrong
//! sentence about a conversation somebody has not opened is exactly the failure
//! worth avoiding. It carries the moment it was written for the same reason.
//!
//! ## Only when there is something new to read
//!
//! Each sentence is kept against the byte length of the transcript it was
//! written from. A conversation whose file has not grown since needs no new
//! call, so thirteen idle conversations cost nothing and a sweep spends model
//! time only on the ones that worked. The cache is a file rather than memory
//! because the alternative is paying for all thirteen again on every restart —
//! and this console restarts whenever it is upgraded, which is often.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::RwLock;

use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

/// What a conversation is about, as last written.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Gist {
    /// One sentence, as the model returned it.
    pub text: String,
    /// When it was written, in epoch milliseconds — so a client can say how old
    /// the sentence is rather than implying it is current.
    pub at: i64,
    /// How long the transcript was when it was written. The whole of the
    /// freshness test: a file that has not grown has nothing new to say.
    #[serde(default)]
    pub bytes: u64,
}

/// The model that writes them.
///
/// The cheapest one available, deliberately: this rides the same subscription
/// allowance as the sessions themselves, and a summary is not worth taking room
/// from the work. Named in full rather than by alias so that a change of model is
/// a change to this line.
const MODEL: &str = "claude-haiku-4-5-20251001";

/// How many exchanges from the end to show it. Enough to see what the
/// conversation has turned into; short enough that the opening still counts.
const RECENT: usize = 20;

/// How long a single summary may take before it is abandoned.
///
/// Generous for a one-shot Haiku call, because the failure this guards is a
/// process that never returns rather than one that is slow.
const PATIENCE: std::time::Duration = std::time::Duration::from_secs(90);

/// How many summaries one sweep will pay for.
///
/// A first run has every conversation to write and no reason to do them all at
/// once; the rest arrive on the next sweep. This is also the cap on what a
/// runaway — a transcript that somehow always looks changed — can spend.
const PER_SWEEP: usize = 8;

/// The sentences, and the file they are kept in.
pub struct Gists {
    store: PathBuf,
    held: RwLock<BTreeMap<String, Gist>>,
}

impl Gists {
    /// Read whatever the last run wrote. An unreadable or absent file is an
    /// empty set rather than an error: the worst case is paying for the
    /// sentences again, and refusing to start would be a much worse trade.
    pub fn load(store: PathBuf) -> Self {
        // ⚠ A file that is there and unreadable is said out loud. It means this
        // console is about to pay a model for sentences it already had, which is
        // a small loss and an invisible one — the list looks the same either
        // way, which is exactly why silence would be wrong here.
        let held = match std::fs::read_to_string(&store) {
            Ok(text) => match serde_json::from_str(&text) {
                Ok(held) => held,
                Err(why) => {
                    tracing::error!(
                        "gists: {} will not parse ({why}) — writing them again from scratch",
                        store.display()
                    );
                    BTreeMap::new()
                }
            },
            // No file at all is the ordinary first run, and says nothing.
            Err(_) => BTreeMap::new(),
        };
        Self {
            store,
            held: RwLock::new(held),
        }
    }

    /// Every sentence there is, for the front page.
    pub fn all(&self) -> BTreeMap<String, Gist> {
        self.held.read().expect("gists poisoned").clone()
    }

    fn keep(&self, id: &str, gist: Gist) {
        let all = {
            let mut held = self.held.write().expect("gists poisoned");
            held.insert(id.to_string(), gist);
            held.clone()
        };
        // Written whole each time. The file is a few kilobytes and holds one
        // line per conversation, so there is nothing here worth the complexity
        // of appending — and a rewrite cannot leave a half-updated entry.
        if let Ok(text) = serde_json::to_string_pretty(&all)
            && let Some(dir) = self.store.parent()
        {
            let _ = std::fs::create_dir_all(dir);
            let _ = std::fs::write(&self.store, text);
        }
    }

    /// Whether this conversation's sentence still describes it.
    fn current(&self, id: &str, bytes: u64) -> bool {
        self.held
            .read()
            .expect("gists poisoned")
            .get(id)
            .is_some_and(|gist| gist.bytes == bytes)
    }

    /// Write a sentence for every conversation that has moved since its last
    /// one, newest first and no more than [`PER_SWEEP`] of them.
    ///
    /// Sequential on purpose. These are model calls against the same allowance
    /// the sessions are using, and there is nothing to be gained by having eight
    /// of them in flight at once on a console driven by one person.
    pub async fn sweep(&self, binary: &str, root: &Path) {
        let mut spent = 0;
        for conversation in crate::past::conversations(root) {
            if spent >= PER_SWEEP {
                tracing::info!("gists: {PER_SWEEP} written this sweep, leaving the rest");
                break;
            }
            if self.current(&conversation.id, conversation.bytes) {
                continue;
            }
            let Some(path) = crate::past::transcript_of(root, &conversation.id) else {
                continue;
            };
            let material = crate::past::material(&path, RECENT);
            if material.opening.is_none() && material.recent.is_empty() {
                continue;
            }
            spent += 1;
            match ask(binary, &prompt(&material)).await {
                Some(text) => self.keep(
                    &conversation.id,
                    Gist {
                        text,
                        at: crate::session::now(),
                        bytes: conversation.bytes,
                    },
                ),
                // Not recorded as anything: a failed call must not mark the
                // conversation as summarised, or one bad moment would leave it
                // blank until the next time it happened to grow.
                None => tracing::warn!("gists: nothing came back for {}", conversation.id),
            }
        }
    }
}

/// What Haiku is asked.
///
/// ⚠ **The shape of the answer is most of the prompt.** Asked plainly, a model
/// returns a paragraph with a preamble, and a card has room for neither. The
/// instruction that matters is the length; the rest is context.
fn prompt(material: &crate::past::Material) -> String {
    let mut text = String::from(
        "Below is part of a conversation between a person and a coding agent, \
         with the tool calls removed.\n\n",
    );
    if let Some(opening) = &material.opening {
        text.push_str("It began with this instruction:\n");
        text.push_str(opening);
        text.push_str("\n\n");
    }
    if !material.recent.is_empty() {
        text.push_str("The most recent exchanges were:\n");
        text.push_str(&material.recent.join("\n"));
        text.push_str("\n\n");
    }
    text.push_str(
        "In ONE sentence of at most twenty words, say what this conversation is about now — \
         what is being worked on, not what it started as. \
         Write only that sentence: no preamble, no quotes, no trailing full stop needed.",
    );
    text
}

/// Put the question to a one-shot session and take its answer.
///
/// ⚠ **On stdin, not in the argument list.** The prompt carries a few thousand
/// characters of somebody's transcript, and an argument that size is at the mercy
/// of a shell's limits and of anything that logs a command line.
///
/// Run from the temporary directory, which is where the transcript of this call
/// itself will be filed — and [`crate::past::conversations`] already leaves those
/// out of the list, so summarising thirteen conversations does not add thirteen
/// more.
async fn ask(binary: &str, prompt: &str) -> Option<String> {
    let mut child = tokio::process::Command::new(binary)
        .current_dir(std::env::temp_dir())
        .arg("-p")
        .args(["--model", MODEL])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .ok()?;
    let mut stdin = child.stdin.take()?;
    stdin.write_all(prompt.as_bytes()).await.ok()?;
    stdin.flush().await.ok()?;
    // Closed, because `-p` reads until end of file and would otherwise wait for
    // the rest of a prompt that has already been sent in full.
    drop(stdin);
    let output = tokio::time::timeout(PATIENCE, child.wait_with_output())
        .await
        .ok()?
        .ok()?;
    let said = String::from_utf8_lossy(&output.stdout);
    sentence(&said)
}

/// The one line of what came back that is the answer.
///
/// A model asked for a sentence occasionally supplies a blank line, a preamble
/// or a wrapper anyway. The first non-empty line is the answer in every one of
/// those cases and in the ordinary one.
pub fn sentence(said: &str) -> Option<String> {
    let line = said.lines().map(str::trim).find(|line| !line.is_empty())?;
    // Quotes get returned about a third of the time despite being asked not to.
    let line = line.trim_matches(|c| c == '"' || c == '\'').trim();
    (!line.is_empty()).then(|| line.to_string())
}
