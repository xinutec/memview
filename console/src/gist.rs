//! One sentence about what each conversation is currently about, since a name
//! and a warmth say nothing about what it is for.
//!
//! This is inference — Haiku guessing from a few thousand characters — and the
//! client marks it as such, with the moment it was written.
//!
//! Only when there is something new to read: each sentence is kept against the
//! byte length of the transcript it came from. A file, because the console
//! restarts whenever it is upgraded. Dropped when the conversation is — see
//! [`Gists::forget`].

use parking_lot::RwLock;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

/// What a conversation is about, as last written.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Gist {
    /// One sentence, as the model returned it.
    pub text: String,
    /// A few words for the same conversation, offered when somebody renames it and
    /// never applied on its own. Optional for ever: older gists lack one, a model may
    /// answer with a single line, and [`answer`] refuses a name that looks like a
    /// second sentence.
    #[serde(default)]
    pub name: Option<String>,
    /// When it was written, in epoch milliseconds — so a client can say how old
    /// the sentence is rather than implying it is current.
    pub at: i64,
    /// How long the transcript was when it was written. The whole of the
    /// freshness test: a file that has not grown has nothing new to say.
    #[serde(default)]
    pub bytes: u64,
}

/// The model that writes them: the cheapest, since this rides the sessions' own
/// allowance. Named in full so a change of model is a change to this line.
const MODEL: &str = "claude-haiku-4-5-20251001";

/// How many exchanges from the end to show it.
const RECENT: usize = 20;

/// How long one summary may take. Generous, because the failure this guards is a
/// process that never returns.
const PATIENCE: std::time::Duration = std::time::Duration::from_secs(90);

/// How many summaries one sweep will pay for — the rest wait for the next sweep,
/// and it caps what a transcript that always looks changed can spend.
const PER_SWEEP: usize = 8;

/// The sentences, and the file they are kept in.
pub struct Gists {
    store: PathBuf,
    held: RwLock<BTreeMap<String, Gist>>,
}

impl Gists {
    /// Read whatever the last run wrote. An unreadable or absent file is an empty set:
    /// the worst case is paying for the sentences again.
    pub fn load(store: PathBuf) -> Self {
        // Said out loud: the console is about to pay for sentences it already had, and
        // the list looks the same either way.
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
        self.held.read().clone()
    }

    fn keep(&self, id: &str, gist: Gist) {
        let all = {
            let mut held = self.held.write();
            held.insert(id.to_string(), gist);
            held.clone()
        };
        self.write(&all);
    }

    /// Drop the sentences whose conversations are no longer there; nothing else ever
    /// removes one.
    ///
    /// An empty list is not an answer: [`crate::past::conversations`] is empty both
    /// when there are no conversations and when it could not read the directory, and
    /// the true empty case has nothing to forget anyway. That walk hides
    /// conversations run from a temporary directory, and [`Self::sweep`] never writes
    /// for those either, so the two halves agree.
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
                "gists: {} conversation(s) gone from disk, forgetting their sentences",
                before - held.len()
            );
            held.clone()
        };
        self.write(&all);
    }

    /// Written whole each time: a few kilobytes, one line per conversation, and a
    /// rewrite cannot leave a half-updated entry.
    fn write(&self, all: &BTreeMap<String, Gist>) {
        if let Ok(text) = serde_json::to_string_pretty(all)
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
            .get(id)
            .is_some_and(|gist| gist.bytes == bytes)
    }

    /// Write a sentence for every conversation that has moved since its last one,
    /// newest first and no more than [`PER_SWEEP`] of them.
    ///
    /// Sequential: these are model calls against the same allowance the sessions use.
    /// The walk is taken once for both halves — forgetting and writing — since the
    /// loop stops after [`PER_SWEEP`] and has no opinion about what it never reached.
    pub async fn sweep(&self, binary: &str, root: &Path) {
        let conversations = crate::past::conversations(root);
        self.forget(&conversations.iter().map(|c| c.id.clone()).collect());
        let mut spent = 0;
        for conversation in conversations {
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
                Some((text, name)) => self.keep(
                    &conversation.id,
                    Gist {
                        text,
                        name,
                        at: crate::session::now(),
                        bytes: conversation.bytes,
                    },
                ),
                // Not recorded: a failed call must not mark the conversation as summarised.
                None => tracing::warn!("gists: nothing came back for {}", conversation.id),
            }
        }
    }
}

/// What Haiku is asked. The shape of the answer is most of the prompt: asked
/// plainly, a model returns a paragraph with a preamble.
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
         Then, on a SECOND line, a name for it: two or three words, at most thirty \
         characters, the way somebody would label a tab. \
         Write only those two lines: no preamble, no labels, no quotes, no trailing full \
         stop needed, and no markdown — they are printed as plain text, so backticks and \
         asterisks arrive as punctuation.",
    );
    text
}

/// Put the question to a one-shot session, take its answer, and leave nothing
/// behind.
///
/// Every call is a conversation, and a conversation is a file that outlives it —
/// filed under `~/.claude/projects/` and hidden from the list. So the id is named
/// here and the file removed the moment the answer is in hand — see [`discard`].
async fn ask(binary: &str, prompt: &str) -> Option<(String, Option<String>)> {
    let named = uuid::Uuid::new_v4().to_string();
    let said = call(binary, prompt, &named).await;
    // Before the answer is examined and on the failing paths too: a call that timed
    // out has left the same file as one that worked.
    discard(&crate::past::projects_root(), &named);
    answer(&said?)
}

/// The call itself. The prompt goes on stdin, not the argument list: it is a few
/// thousand characters of somebody's transcript.
async fn call(binary: &str, prompt: &str, named: &str) -> Option<String> {
    let mut child = tokio::process::Command::new(binary)
        .current_dir(std::env::temp_dir())
        .arg("-p")
        .args(["--session-id", named])
        .args(["--model", MODEL])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .ok()?;
    // Logged at the spawn, so a `<defunct>` under the console can be traced to a
    // spawn site. `unwrap_or(0)` rather than `{:?}`: read beside what `ps` prints.
    tracing::info!(
        "gists: asking pid {} about {named}",
        child.id().unwrap_or(0)
    );
    // Every `?` here must still end in a wait: `kill_on_drop` cannot kill a child
    // that has already exited, so its `<defunct>` would stay.
    let sent = async {
        let mut stdin = child.stdin.take()?;
        stdin.write_all(prompt.as_bytes()).await.ok()?;
        stdin.flush().await.ok()?;
        // Closed, because `-p` reads until end of file and would otherwise wait
        // for the rest of a prompt that has already been sent in full.
        drop(stdin);
        Some(())
    }
    .await;
    if sent.is_none() {
        // Reaped rather than leaked. The answer is lost either way; the process
        // table entry need not be.
        let _ = child.wait().await;
        return None;
    }
    let output = tokio::time::timeout(PATIENCE, child.wait_with_output())
        .await
        .ok()?
        .ok()?;
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Remove the transcript one of these calls left behind. Found by id rather than
/// by working out the path: the folder under `projects/` is named by an
/// undocumented encoding, and [`crate::past::transcript_of`] already answers this.
/// Silent when there is nothing to remove.
pub fn discard(root: &Path, id: &str) {
    let Some(path) = crate::past::transcript_of(root, id) else {
        return;
    };
    if let Err(why) = std::fs::remove_file(&path) {
        // Debug, not warn: the sentence was written either way, and this would otherwise
        // repeat every quarter of an hour for ever.
        tracing::debug!("gists: {} would not go ({why})", path.display());
    }
}

/// The one line of what came back that is the answer: a model asked for a
/// sentence sometimes adds a blank line, a preamble or a wrapper.
pub fn sentence(said: &str) -> Option<String> {
    tidy(said.lines().map(str::trim).find(|line| !line.is_empty())?)
}

/// The longest a suggested name may be. A little over the thirty the prompt asks
/// for: a model that overshoots by a word has still answered, one that returns a
/// second sentence has not.
const NAME_AT_MOST: usize = 40;

/// The most words a name may be, for the same reason.
const NAME_WORDS_AT_MOST: usize = 6;

/// Both lines of an answer: the sentence, and the name if there is one. A name
/// that does not look like one is refused rather than trimmed — a plausible
/// half-sentence offered as a suggestion reads as considered.
pub fn answer(said: &str) -> Option<(String, Option<String>)> {
    let mut lines = said.lines().map(str::trim).filter(|line| !line.is_empty());
    let text = tidy(lines.next()?)?;
    let name = lines.next().and_then(tidy).filter(|name| {
        name.chars().count() <= NAME_AT_MOST
            && name.split_whitespace().count() <= NAME_WORDS_AT_MOST
    });
    Some((text, name))
}

/// One line of an answer, with what a model puts round it taken off.
fn tidy(line: &str) -> Option<String> {
    // Models return quotes despite being asked not to.
    let line = line.trim_matches(|c| c == '"' || c == '\'').trim();
    // And the marks: the card draws this as plain text, where they would show as
    // punctuation.
    // Backticks and asterisks only: an underscore is likelier part of a name.
    let line: String = line.chars().filter(|c| *c != '`' && *c != '*').collect();
    let line = line.trim();
    (!line.is_empty()).then(|| line.to_string())
}
