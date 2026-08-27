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
//!
//! ## And dropped when the conversation is
//!
//! A cache keyed by id that is only ever written to is a cache that grows
//! forever. Each sweep therefore also forgets the sentences whose transcripts
//! have gone from disk — see [`Gists::forget`], which is where the one case that
//! could go wrong is written down.

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
    /// A few words for the same conversation, offered when somebody renames it
    /// and never applied on its own — see `RenameSheet`.
    ///
    /// ⚠ **Optional for ever, not just until the next sweep.** Every gist
    /// written before this existed is on disk without one, a model that answers
    /// with a single line leaves it unset, and a name that comes back looking
    /// like a second sentence is refused by [`answer`]. A caller that treats
    /// this as reliably present is wrong about all three.
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
        self.write(&all);
    }

    /// Drop the sentences whose conversations are no longer there.
    ///
    /// ⚠ **Without this nothing is ever removed.** [`Self::keep`] only inserts,
    /// so a deleted transcript left its sentence in the map and in the file for
    /// good — invisible on screen, since the rows come from a walk of the disk
    /// and a sentence is only ever looked up by a row's id, but the file grew by
    /// one dead entry per deletion and never shrank.
    ///
    /// ⚠ **An empty list is not an answer.** [`crate::past::conversations`]
    /// yields nothing both when there are no conversations and when it could not
    /// read the directory at all, and the two are indistinguishable from here.
    /// Treating the second as the first would throw away every sentence over one
    /// bad moment and pay a model for all of them again, so an empty list is
    /// left alone: the true empty case has nothing to forget anyway.
    ///
    /// The list to judge against is the one the front page itself is drawn from,
    /// which is narrower than what is on disk — [`crate::past::conversations`]
    /// leaves out conversations that ran from a temporary directory. That is the
    /// right list anyway, and the two halves agree by construction: a
    /// conversation the walk hides is one [`Self::sweep`] never writes a sentence
    /// for either, so there is nothing of it here to lose.
    pub fn forget(&self, alive: &std::collections::BTreeSet<String>) {
        if alive.is_empty() {
            return;
        }
        let all = {
            let mut held = self.held.write().expect("gists poisoned");
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

    /// Written whole each time. The file is a few kilobytes and holds one line
    /// per conversation, so there is nothing here worth the complexity of
    /// appending — and a rewrite cannot leave a half-updated entry.
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
    ///
    /// The walk is taken once and used twice: to forget the conversations that
    /// have gone, and then to write for the ones that moved. It has to be the
    /// whole walk rather than the loop below, which stops after [`PER_SWEEP`]
    /// and so has no opinion about the conversations it never reached.
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
/// ⚠ **Every call is a conversation, and a conversation is a file that outlives
/// it.** Run from the temporary directory, this call's own transcript is filed
/// under `~/.claude/projects/` in a folder named for that directory — and
/// [`crate::past::conversations`] hides those from the list, which is why it
/// went unnoticed that they were still there: 2,299 of them, 57 MB, in the three
/// days after this was written, growing at a sweep's worth every quarter of an
/// hour, for ever. Hidden is not gone. So the id is named here rather than left
/// to the CLI, and the file is removed the moment the answer is in hand — see
/// [`discard`], which is the whole reason for naming it.
async fn ask(binary: &str, prompt: &str) -> Option<(String, Option<String>)> {
    let named = uuid::Uuid::new_v4().to_string();
    let said = call(binary, prompt, &named).await;
    // Before the answer is examined, and on the failing paths as well: a call
    // that timed out or came back empty has left exactly the same file as one
    // that worked.
    discard(&crate::past::projects_root(), &named);
    answer(&said?)
}

/// The call itself, up to the words that came back.
///
/// ⚠ **On stdin, not in the argument list.** The prompt carries a few thousand
/// characters of somebody's transcript, and an argument that size is at the mercy
/// of a shell's limits and of anything that logs a command line.
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
    // **So a `<defunct>` under the console can be traced to a spawn site.** #797
    // is a zombie whose parent is the console and whose origin nothing recorded;
    // sessions are already attributable because the roster holds their pids, and
    // this and `deaf.rs` were the two that spawned anonymously. Logged at the
    // spawn rather than at the exit, because the case worth explaining is the one
    // that never reaches an exit.
    // `unwrap_or(0)` as `session.rs` does, rather than `{:?}`: this line exists
    // to be read beside a `<defunct>` in `ps`, and `Some(52925)` is not what ps
    // prints. A child that has already been waited for has no id, and 0 is a pid
    // no process has.
    tracing::info!(
        "gists: asking pid {} about {named}",
        child.id().unwrap_or(0)
    );
    // ⚠ **Every `?` here used to leak the child, and one of them did.** Handing
    // the prompt over is three fallible steps, and returning early from any of
    // them dropped `child` having never waited on it. `kill_on_drop` does not
    // save that case: a child that has ALREADY exited cannot be killed, so the
    // signal is a no-op and the `<defunct>` stays.
    //
    // ⚠ **This is not the shape #797 tested.** Both its cases model a child that
    // outlives the timeout, and both reap. The real one died in 66 seconds
    // against a 90-second `PATIENCE` — before the timeout could fire — and the
    // gist that was eventually stored came from a later call seventeen minutes
    // on, so this one returned nothing and was never waited for. Traced 2026-08-27
    // from pid 93988, three days defunct under the console.
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

/// Remove the transcript one of these calls left behind.
///
/// ⚠ **Found by id rather than by working out the path.** The folder under
/// `projects/` is named by an undocumented encoding of the working directory,
/// and [`crate::past`] opens with the account of guessing it wrong; the id is
/// enough, because [`crate::past::transcript_of`] answers exactly this question
/// and is the same lookup the viewer uses.
///
/// Silent when there is nothing to remove. A CLI that never got as far as
/// writing a file, or one that files them somewhere else entirely, is not a
/// failure of the sweep it was summarising for.
pub fn discard(root: &Path, id: &str) {
    let Some(path) = crate::past::transcript_of(root, id) else {
        return;
    };
    if let Err(why) = std::fs::remove_file(&path) {
        // Debug, not warn: the sentence was written either way, and a console
        // that cannot delete its own leftovers would otherwise say so every
        // quarter of an hour for ever.
        tracing::debug!("gists: {} would not go ({why})", path.display());
    }
}

/// The one line of what came back that is the answer.
///
/// A model asked for a sentence occasionally supplies a blank line, a preamble
/// or a wrapper anyway. The first non-empty line is the answer in every one of
/// those cases and in the ordinary one.
pub fn sentence(said: &str) -> Option<String> {
    tidy(said.lines().map(str::trim).find(|line| !line.is_empty())?)
}

/// The longest a suggested name may be, in characters.
///
/// The prompt asks for thirty and this allows a little over, because a model
/// that overshoots by a word has still answered the question — where one that
/// returns a second sentence has not. It is the difference between the two that
/// matters, not the exact figure.
const NAME_AT_MOST: usize = 40;

/// The most words a name may be, for the same reason.
const NAME_WORDS_AT_MOST: usize = 6;

/// Both lines of an answer: the sentence, and the name if there is one.
///
/// ⚠ **The name is refused rather than trimmed when it does not look like
/// one.** A model that ignores the second instruction returns another sentence,
/// and cutting that to [`NAME_AT_MOST`] characters would produce a plausible
/// half-sentence — offered to somebody naming a conversation, that is worse
/// than offering nothing, because a suggestion is read as considered.
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
    // Quotes get returned about a third of the time despite being asked not to.
    let line = line.trim_matches(|c| c == '"' || c == '\'').trim();
    // ⚠ **And the marks come out, because the card draws this as text.** The
    // front page prints the sentence rather than rendering it, so a backtick or
    // a pair of asterisks arrives as punctuation — seen live: "Widening `Fact`
    // in Rust to support configuration-derived IDs". Asking for no markdown is
    // the other half of this and is in [`prompt`]; the instruction is what
    // usually works and this is what always does.
    //
    // Rendering it instead was the alternative, and was rejected: this is one
    // line in a card head, and a pipe that can emit a list or a code fence into
    // a row sized for one line is a layout defect waiting for the sentence that
    // triggers it.
    //
    // ⚠ **Backticks and asterisks only.** An underscore is far likelier to be
    // part of a name than an emphasis in this vocabulary — the sentences here
    // are about code, and one of them today was about
    // `project_health_verified_core_lean`, which stripping would have turned
    // into a word.
    let line: String = line.chars().filter(|c| *c != '`' && *c != '*').collect();
    let line = line.trim();
    (!line.is_empty()).then(|| line.to_string())
}
