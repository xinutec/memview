//! One sentence about what each conversation is about, guessed by Haiku and marked
//! as a guess. Rewritten only when the transcript has grown; kept in a file.

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
    /// A few words offered as a name when renaming, never applied on their own.
    #[serde(default)]
    pub name: Option<String>,
    /// When it was written, in epoch milliseconds.
    pub at: i64,
    /// The transcript's length when it was written — the freshness test.
    #[serde(default)]
    pub bytes: u64,
}

/// The model that writes them: the cheapest, on the sessions' allowance.
const MODEL: &str = "claude-haiku-4-5-20251001";

/// How many exchanges from the end to show it.
const RECENT: usize = 20;

/// How long one summary may take, against a process that never returns.
const PATIENCE: std::time::Duration = std::time::Duration::from_secs(90);

/// How many summaries one sweep will pay for.
const PER_SWEEP: usize = 8;

/// The sentences, and the file they are kept in.
pub struct Gists {
    store: PathBuf,
    held: RwLock<BTreeMap<String, Gist>>,
}

impl Gists {
    /// Read whatever the last run wrote; unreadable is empty.
    pub fn load(store: PathBuf) -> Self {
        // Logged: the sentences will be paid for again.
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
            // No file: the first run.
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

    /// Drop the sentences of conversations no longer there; an empty list drops nothing.
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

    /// Written whole each time.
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

    /// Write a sentence for each conversation that has moved, newest first, at most
    /// [`PER_SWEEP`], one at a time.
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
                // A failure must not count as summarised.
                None => tracing::warn!("gists: nothing came back for {}", conversation.id),
            }
        }
    }
}

/// What Haiku is asked; mostly the shape of the answer.
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

/// Ask a one-shot session and remove the transcript it leaves — [`discard`].
async fn ask(binary: &str, prompt: &str) -> Option<(String, Option<String>)> {
    let named = uuid::Uuid::new_v4().to_string();
    let said = call(binary, prompt, &named).await;
    // On every path: a timed-out call leaves the file too.
    discard(&crate::past::projects_root(), &named);
    answer(&said?)
}

/// The call itself, with the prompt on stdin, not argv.
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
    // Logged, so a `<defunct>` can be traced to its spawn.
    tracing::info!(
        "gists: asking pid {} about {named}",
        child.id().unwrap_or(0)
    );
    // Every path must end in a wait, or an exited child stays `<defunct>`.
    let sent = async {
        let mut stdin = child.stdin.take()?;
        stdin.write_all(prompt.as_bytes()).await.ok()?;
        stdin.flush().await.ok()?;
        // Closed: `-p` reads to end of file.
        drop(stdin);
        Some(())
    }
    .await;
    if sent.is_none() {
        // Reaped, not leaked.
        let _ = child.wait().await;
        return None;
    }
    let output = tokio::time::timeout(PATIENCE, child.wait_with_output())
        .await
        .ok()?
        .ok()?;
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Remove the transcript one of these calls left, found by id.
pub fn discard(root: &Path, id: &str) {
    let Some(path) = crate::past::transcript_of(root, id) else {
        return;
    };
    if let Err(why) = std::fs::remove_file(&path) {
        // Debug: it would repeat every sweep.
        tracing::debug!("gists: {} would not go ({why})", path.display());
    }
}

/// The line of what came back that is the answer.
pub fn sentence(said: &str) -> Option<String> {
    tidy(said.lines().map(str::trim).find(|line| !line.is_empty())?)
}

/// The longest a suggested name may be — a little over what the prompt asks.
const NAME_AT_MOST: usize = 40;

/// The most words a name may be.
const NAME_WORDS_AT_MOST: usize = 6;

/// Both lines of an answer: the sentence, and the name if it looks like one.
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
    // Backticks and asterisks; an underscore is likelier part of a name.
    let line: String = line.chars().filter(|c| *c != '`' && *c != '*').collect();
    let line = line.trim();
    (!line.is_empty()).then(|| line.to_string())
}
