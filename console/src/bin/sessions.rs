//! The live Claude Code sessions on this Mac: list them, read a conversation,
//! send one a message.
//!
//!     cargo run -p console --bin sessions -- who
//!     cargo run -p console --bin sessions -- log home
//!     cargo run -p console --bin sessions -- log home 20 --since 22:00 --full
//!     cargo run -p console --bin sessions -- last home
//!     cargo run -p console --bin sessions -- last home 3 --user
//!     cargo run -p console --bin sessions -- send home "ready to compact?"
//!     echo "..." | cargo run -p console --bin sessions -- send home -
//!
//! ⚠ **The console listens on two ports, and the one that gets written down is
//! the gated one.** `BIND_ADDR` (8097) is TLS behind the pinned-client gate —
//! the phone's door, reached through the reverse tunnel. A caller on this Mac
//! holds no pinned key, so that port answers it with a handshake that completes
//! and then dies on first read with `certificate_required`: a true diagnosis of
//! the wrong door. `CONSOLE_DESK_ADDR` (8096) is the ungated loopback one, and
//! is what this speaks to. The default is taken from [`Config::from_env`]
//! rather than written out again here, so the two cannot drift apart.
//!
//! The rest of it exists because every step has already gone wrong by hand: a
//! 36-character session id pasted where `home` was meant, an apostrophe
//! shell-quoted into JSON and silently dropped, a message sent to a session that
//! had already exited, and two half-conversations interleaved by eye because
//! reading each side took its own command.

use anyhow::{Context, Result, bail};
use console::config::Config;
use serde::Deserialize;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// How much of a transcript's tail to read.
///
/// ⚠ **A live transcript can be gigabytes** — 1.7 GB on this Mac in August — so
/// reading the whole file to print one message is not an option. Eight
/// megabytes covers hundreds of exchanges of the largest session seen; when it
/// does not, [`Line`] counting says so rather than quietly showing less.
const TAIL: u64 = 8 << 20;

/// How much of a message `log` shows before saying how much it kept back.
///
/// ⚠ **Never truncate silently.** A cut that leaves no mark is indistinguishable
/// from a message that was short, which is the same shape of mistake as a probe
/// returning empty and being read as a result.
const WIDTH: usize = 700;

#[derive(Deserialize)]
struct Overview {
    sessions: Vec<Row>,
}

/// The fields of the console's session summary this tool actually shows. It is
/// deliberately a subset: `#[serde(default)]` throughout means a summary that
/// grows a field, or drops an optional one, does not break the CLI.
#[derive(Deserialize)]
struct Row {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    alive: bool,
    #[serde(default)]
    working: bool,
    /// What the CLI last said it was doing. Only meaningful while `working`.
    #[serde(default)]
    busy: Option<String>,
    /// When the conversation last moved, in milliseconds — not when the console
    /// picked the process up. For a session running since last night the two are
    /// hours apart, and this is the one worth showing.
    #[serde(default)]
    touched: Option<u64>,
    #[serde(default)]
    unread: usize,
    #[serde(default)]
    waiting: usize,
    #[serde(default)]
    context: Option<u64>,
    #[serde(default)]
    window: Option<u64>,
}

impl Row {
    fn label(&self) -> &str {
        self.name.as_deref().unwrap_or("(unnamed)")
    }

    fn state(&self) -> &'static str {
        match (self.alive, self.working, self.waiting > 0) {
            (false, _, _) => "exited",
            (_, _, true) => "waiting",
            (_, true, _) => "working",
            _ => "idle",
        }
    }
}

/// Who said it. The session is named rather than called "assistant" because the
/// name is what a person asking about it uses.
#[derive(PartialEq, Clone, Copy)]
enum Voice {
    Pippijn,
    Session,
}

struct Line {
    at: String,
    voice: Voice,
    text: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    // ⚠ **`reqwest` here is a rustls build with no default provider**, chosen so
    // that nothing in this tree needs cmake and a C toolchain to compile. That
    // makes installing one the caller's job, and skipping it does not fail to
    // link — it panics with "No provider set" on the first request, which reads
    // like a bug in the console rather than a missing line here. `main.rs` does
    // the same thing for the same reason.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest: Vec<&str> = args.iter().map(String::as_str).collect();
    match rest.split_first() {
        None | Some((&"who", [])) => who().await,
        Some((&"log", tail)) => read(tail, Mode::Log).await,
        Some((&"last", tail)) => read(tail, Mode::Last).await,
        Some((&"send", tail)) => send(tail).await,
        Some((&("-h" | "--help" | "help"), _)) => {
            println!("{}", usage());
            Ok(())
        }
        Some((other, _)) => bail!("no such command {other:?}\n\n{}", usage()),
    }
}

/// What to type, with the binary's own name in it.
///
/// ⚠ **`CARGO_BIN_NAME`, never the name written out again.** Renaming this tool
/// on 2026-08-31 left the old word in two `bail!` strings and every line of this
/// text, because a rename satisfies the filename — which is where Cargo gets the
/// bin name — and says nothing about the copies. Nothing failed; the tool simply
/// printed a command that did not exist. The compiler supplies the name, so the
/// two cannot disagree and the next rename is `git mv` and nothing else.
fn usage() -> String {
    let me = env!("CARGO_BIN_NAME");
    format!(
        "usage:
  {me}                       the sessions: state, what they are doing, how long ago
  {me} log <session> [n]     the last n messages of the conversation, both sides
  {me} last <session> [n]    the last n things the session said, in full
  {me} send <session> <text> send it a message, as Pippijn; `-` reads stdin

  `{me} who` is still accepted; the bare form is the same listing.

  --user      with `last`, show what Pippijn said instead
  --full      with `log`, do not shorten long messages
  --since T   only messages at or after T — `22:00` today, or a full ISO stamp

<session> is a name (`home`) or the start of an id (`1b6f2e45`)."
    )
}

enum Mode {
    Log,
    Last,
}

/// The console's ungated loopback address, from the same place the server reads
/// it, so this cannot go stale when that default moves.
fn desk() -> String {
    Config::from_env().desk
}

async fn overview() -> Result<Vec<Row>> {
    let addr = desk();
    let url = format!("http://{addr}/api/state");
    let body = reqwest::get(&url)
        .await
        .with_context(|| format!("asking the console at {url} — is it running?"))?
        .error_for_status()?
        .json::<Overview>()
        .await
        .context("reading the session list")?;
    Ok(body.sessions)
}

/// How long ago, in the shortest form that is not a lie.
fn ago(touched: Option<u64>) -> String {
    let Some(touched) = touched else {
        return String::new();
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis() as u64)
        .unwrap_or(touched);
    let secs = now.saturating_sub(touched) / 1000;
    match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m", secs / 60),
        3600..86400 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86400),
    }
}

async fn who() -> Result<()> {
    let mut sessions = overview().await?;
    sessions.sort_by(|a, b| a.label().cmp(b.label()));
    println!(
        "{:<10} {:<8} {:<8} {:>4} {:>6} {:>10}  DOING",
        "NAME", "ID", "STATE", "AGO", "UNREAD", "CONTEXT"
    );
    for row in &sessions {
        let context = match (row.context, row.window) {
            (Some(used), Some(window)) => format!("{}k/{}k", used / 1000, window / 1000),
            (Some(used), None) => format!("{}k", used / 1000),
            _ => String::new(),
        };
        // Only while it is working: the CLI leaves the last thing it narrated in
        // place, so showing it for an idle session says it is still doing that.
        let doing = match (row.working, &row.busy) {
            (true, Some(busy)) => busy.as_str(),
            _ => "",
        };
        println!(
            "{:<10} {:<8} {:<8} {:>4} {:>6} {:>10}  {}",
            row.label(),
            &row.id[..8.min(row.id.len())],
            row.state(),
            ago(row.touched),
            row.unread,
            context,
            doing
        );
    }
    Ok(())
}

/// A name, or the start of an id. Names win: an id prefix that happens to spell
/// a session's name is asking for that session, not for the coincidence.
fn resolve<'a>(sessions: &'a [Row], needle: &str) -> Result<&'a Row> {
    let by_name: Vec<&Row> = sessions
        .iter()
        .filter(|row| {
            row.name
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case(needle))
        })
        .collect();
    let found = match by_name.len() {
        1 => return Ok(by_name[0]),
        0 => sessions
            .iter()
            .filter(|row| row.id.starts_with(needle))
            .collect::<Vec<_>>(),
        _ => by_name,
    };
    match found.len() {
        1 => Ok(found[0]),
        0 => {
            let names: Vec<&str> = sessions.iter().map(Row::label).collect();
            bail!("no session {needle:?}. there is: {}", names.join(", "))
        }
        _ => {
            let all: Vec<String> = found
                .iter()
                .map(|row| format!("{} ({})", row.label(), &row.id[..8]))
                .collect();
            bail!("{needle:?} matches {}", all.join(", "))
        }
    }
}

/// Where a session's conversation is on disk.
///
/// Found by looking rather than by rebuilding the path: Claude Code encodes the
/// working directory into the project directory's name, and re-deriving that
/// encoding is a way to be wrong about a session whose directory has a dot or a
/// symlink in it.
fn transcript(id: &str) -> Result<PathBuf> {
    let root = reader::home::projects_dir();
    let entries =
        std::fs::read_dir(&root).with_context(|| format!("listing {}", root.display()))?;
    for entry in entries {
        let path = entry?.path().join(format!("{id}.jsonl"));
        if path.is_file() {
            return Ok(path);
        }
    }
    bail!("no transcript for {id} under {}", root.display())
}

/// The tail of a file, starting at a line boundary.
fn tail_of(path: &Path) -> Result<Vec<u8>> {
    let mut file =
        std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let len = file.metadata()?.len();
    let from = len.saturating_sub(TAIL);
    file.seek(SeekFrom::Start(from))?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;
    // ⚠ A seek lands mid-line, and half a JSON object parses as nothing at all —
    // silently, which would read as "the session said nothing".
    if from > 0
        && let Some(newline) = buf.iter().position(|byte| *byte == b'\n')
    {
        buf.drain(..=newline);
    }
    Ok(buf)
}

/// The text of an assistant turn, or `None` for every other kind of row.
fn said(row: &serde_json::Value) -> Option<String> {
    if row["type"].as_str()? != "assistant" {
        return None;
    }
    let text = row["message"]["content"]
        .as_array()?
        .iter()
        .filter(|block| block["type"] == "text")
        .filter_map(|block| block["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let text = text.trim().to_string();
    (!text.is_empty()).then_some(text)
}

/// Both sides of the conversation, in the order they were recorded.
///
/// ⚠ **The two sides are read by different rules and neither generalises.** What
/// counts as a human turn is five facts that have each cost somebody an
/// afternoon — a queued message lives in an `attachment` row and never in a
/// `user` one, a `tool_result` wears the user's role — and they live in
/// `reader`, which owns them for the whole workspace. The assistant side needs
/// only the dedupe rule, because nothing else wears its type.
fn conversation(bytes: &[u8]) -> Vec<Line> {
    let mut lines: Vec<Line> = reader::transcript::human_turns(bytes)
        .into_iter()
        .map(|turn| Line {
            at: turn.at,
            voice: Voice::Pippijn,
            text: turn.text,
        })
        .collect();

    let mut seen = std::collections::HashSet::new();
    for row in bytes.split(|byte| *byte == b'\n') {
        let Ok(row) = serde_json::from_slice::<serde_json::Value>(row) else {
            continue;
        };
        // ⚠ The CLI rewrites earlier stretches back into the same file, so a
        // linear read returns some turns twice — and the later copy is the
        // degraded one. Same rule as `reader::transcript::human_turns`.
        let uuid = row["uuid"].as_str().unwrap_or_default().to_string();
        if !uuid.is_empty() && !seen.insert(uuid) {
            continue;
        }
        if let Some(text) = said(&row) {
            lines.push(Line {
                at: row["timestamp"].as_str().unwrap_or_default().to_string(),
                voice: Voice::Session,
                text,
            });
        }
    }
    // ISO-8601 in a fixed zone sorts as text, which is why the transcripts use
    // it. ⚠ A queued turn is stamped when it was ENQUEUED, so it can sort before
    // the reply to the message ahead of it — that is the truth about when it was
    // typed, not a bug to correct here.
    lines.sort_by(|a, b| a.at.cmp(&b.at));
    lines
}

/// `--since 22:00` means today at 22:00, in whatever the transcript's stamps
/// are; a longer value is compared as the prefix it is.
fn since_of(args: &[&str]) -> Option<String> {
    let at = args.iter().position(|arg| *arg == "--since")?;
    let value = args.get(at + 1)?;
    if value.len() == 5 && value.as_bytes()[2] == b':' {
        let today = &time::OffsetDateTime::now_utc();
        let stamp = format!(
            "{:04}-{:02}-{:02}T{value}",
            today.year(),
            today.month() as u8,
            today.day()
        );
        Some(stamp)
    } else {
        Some((*value).to_string())
    }
}

async fn read(args: &[&str], mode: Mode) -> Result<()> {
    let mine = args.iter().any(|arg| *arg == "--user" || *arg == "-u");
    let full = args.iter().any(|arg| *arg == "--full" || *arg == "-f");
    let since = since_of(args);
    // Skip the flags and the value `--since` consumed.
    let mut positional: Vec<&str> = Vec::new();
    let mut skip = false;
    for arg in args {
        if skip {
            skip = false;
            continue;
        }
        if *arg == "--since" {
            skip = true;
        } else if !arg.starts_with('-') {
            positional.push(arg);
        }
    }
    let Some(needle) = positional.first() else {
        bail!(
            "usage: {} {} <session> [n]",
            env!("CARGO_BIN_NAME"),
            match mode {
                Mode::Log => "log",
                Mode::Last => "last",
            }
        );
    };
    let count: usize = match positional.get(1) {
        Some(n) => n
            .parse()
            .with_context(|| format!("{n:?} is not a number"))?,
        None => match mode {
            Mode::Log => 10,
            Mode::Last => 1,
        },
    };

    let sessions = overview().await?;
    let row = resolve(&sessions, needle)?;
    let bytes = tail_of(&transcript(&row.id)?)?;
    let mut lines = conversation(&bytes);

    if let Some(since) = &since {
        lines.retain(|line| line.at.as_str() >= since.as_str());
    }
    if let Mode::Last = mode {
        let want = if mine { Voice::Pippijn } else { Voice::Session };
        lines.retain(|line| line.voice == want);
    }
    if lines.is_empty() {
        let scope = since
            .map(|since| format!("since {since}"))
            .unwrap_or_else(|| format!("in the last {} MB", TAIL >> 20));
        bail!("nothing from {} {scope}", row.label());
    }

    let shown = lines.len().min(count);
    for line in &lines[lines.len() - shown..] {
        let who = match line.voice {
            Voice::Pippijn => "pippijn",
            Voice::Session => row.label(),
        };
        let stamp = &line.at[..19.min(line.at.len())];
        let body = match (
            matches!(mode, Mode::Log) && !full,
            line.text.char_indices().nth(WIDTH),
        ) {
            // ⚠ The count of what was kept back is the point — see `WIDTH`.
            (true, Some((cut, _))) => format!(
                "{}\n    … +{} more chars",
                &line.text[..cut],
                line.text.chars().count() - WIDTH
            ),
            _ => line.text.clone(),
        };
        println!("=== {stamp} {who}\n{body}\n");
    }
    // Saying what was left out, so a short answer is never mistaken for a short
    // conversation.
    if lines.len() > shown {
        println!("({} earlier, ask for more)", lines.len() - shown);
    }
    Ok(())
}

async fn send(args: &[&str]) -> Result<()> {
    let Some((needle, words)) = args.split_first() else {
        bail!("usage: {} send <session> <text>", env!("CARGO_BIN_NAME"));
    };
    if words.is_empty() {
        bail!("nothing to send");
    }
    let text = if words == ["-"] {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        buf
    } else {
        words.join(" ")
    };
    let text = text.trim();
    // An empty message is accepted by the API and arrives as a turn that says
    // nothing, which reads to the session as though Pippijn sent whitespace.
    if text.is_empty() {
        bail!("nothing to send");
    }

    let sessions = overview().await?;
    let row = resolve(&sessions, needle)?;
    if !row.alive {
        bail!("{} has exited — a message would go nowhere", row.label());
    }

    let addr = desk();
    let url = format!("http://{addr}/api/sessions/{}/input", row.id);
    let reply = reqwest::Client::new()
        .post(&url)
        .json(&serde_json::json!({ "text": text }))
        .send()
        .await
        .with_context(|| format!("sending to {url}"))?;
    let status = reply.status();
    let body = reply.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("the console refused it ({status}): {body}");
    }
    // The receipt is the point: a message that arrives twice leaves no other
    // trace, and one that arrives while the session is working waits its turn
    // rather than interrupting it.
    //
    // ⚠ **The name comes from the row we resolved, not from the receipt.** This
    // endpoint answers with the session's own summary, which carries no `name` —
    // the roster attaches that in `/api/state` only — so reading it back here
    // reported every successful send as going to `(unnamed)`.
    let after: Row = serde_json::from_str(&body).context("reading the receipt")?;
    println!(
        "sent to {} ({}), {} unread, {} chars",
        row.label(),
        after.state(),
        after.unread,
        text.chars().count()
    );
    Ok(())
}
