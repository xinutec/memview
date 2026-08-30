//! Talk to the live Claude Code sessions on this Mac: find one by name, read
//! what it last said, send it a message.
//!
//!     cargo run -p console --bin talk -- who
//!     cargo run -p console --bin talk -- last home
//!     cargo run -p console --bin talk -- last home 3 --user
//!     cargo run -p console --bin talk -- send home "ready to compact?"
//!     echo "..." | cargo run -p console --bin talk -- send home -
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
//! shell-quoted into JSON and silently dropped, and a message sent to a session
//! that had already exited.

use anyhow::{Context, Result, bail};
use console::config::Config;
use serde::Deserialize;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

/// How much of a transcript's tail to read for `last`.
///
/// ⚠ **A live transcript can be gigabytes** — 1.7 GB on this Mac in August — so
/// reading the whole file to print one message is not an option. Eight
/// megabytes covers hundreds of exchanges of the largest session seen; when it
/// does not, the answer is fewer messages rather than a slower tool.
const TAIL: u64 = 8 << 20;

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
        Some((&"last", tail)) => last(tail).await,
        Some((&"send", tail)) => send(tail).await,
        Some((&("-h" | "--help" | "help"), _)) => {
            println!("{}", USAGE);
            Ok(())
        }
        Some((other, _)) => bail!("no such command {other:?}\n\n{USAGE}"),
    }
}

const USAGE: &str = "usage:
  talk who                       list the sessions
  talk last <session> [n]        print the last n things it said (default 1)
  talk last <session> [n] --user print the last n things Pippijn said instead
  talk send <session> <text>     send it a message, as Pippijn; `-` reads stdin

<session> is a name (`home`) or the start of an id (`1b6f2e45`).";

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

async fn who() -> Result<()> {
    let mut sessions = overview().await?;
    sessions.sort_by(|a, b| a.label().cmp(b.label()));
    println!(
        "{:<10} {:<8} {:<8} {:>6}  CONTEXT",
        "NAME", "ID", "STATE", "UNREAD"
    );
    for row in &sessions {
        let context = match (row.context, row.window) {
            (Some(used), Some(window)) => format!("{}k/{}k", used / 1000, window / 1000),
            (Some(used), None) => format!("{}k", used / 1000),
            _ => String::new(),
        };
        println!(
            "{:<10} {:<8} {:<8} {:>6}  {}",
            row.label(),
            &row.id[..8.min(row.id.len())],
            row.state(),
            row.unread,
            context
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
                .is_some_and(|n| n.eq_ignore_ascii_case(needle))
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
fn tail_of(path: &PathBuf) -> Result<Vec<u8>> {
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
        && let Some(nl) = buf.iter().position(|byte| *byte == b'\n')
    {
        buf.drain(..=nl);
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

async fn last(args: &[&str]) -> Result<()> {
    let mine = args.iter().any(|arg| *arg == "--user" || *arg == "-u");
    let args: Vec<&&str> = args.iter().filter(|arg| !arg.starts_with('-')).collect();
    let Some(needle) = args.first() else {
        bail!("usage: talk last <session> [n] [--user]");
    };
    let count: usize = match args.get(1) {
        Some(n) => n
            .parse()
            .with_context(|| format!("{n:?} is not a number"))?,
        None => 1,
    };

    let sessions = overview().await?;
    let row = resolve(&sessions, needle)?;
    let bytes = tail_of(&transcript(&row.id)?)?;

    let turns: Vec<(String, String)> = if mine {
        // The five facts about what counts as a human turn live in `reader`, and
        // getting any of them wrong has cost somebody an afternoon already.
        reader::transcript::human_turns(&bytes)
            .into_iter()
            .map(|turn| (turn.at, turn.text))
            .collect()
    } else {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for line in bytes.split(|byte| *byte == b'\n') {
            let Ok(row) = serde_json::from_slice::<serde_json::Value>(line) else {
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
                out.push((
                    row["timestamp"].as_str().unwrap_or_default().to_string(),
                    text,
                ));
            }
        }
        out
    };

    if turns.is_empty() {
        bail!(
            "nothing found in the last {} MB of {}",
            TAIL >> 20,
            row.label()
        );
    }
    for (at, text) in turns.iter().rev().take(count).rev() {
        println!("=== {} {}\n{text}\n", &at[..19.min(at.len())], row.label());
    }
    Ok(())
}

async fn send(args: &[&str]) -> Result<()> {
    let Some((needle, words)) = args.split_first() else {
        bail!("usage: talk send <session> <text>");
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
