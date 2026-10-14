//! The live Claude Code sessions on this Mac: list them, read a conversation,
//! send one a message.
//!
//!     sessions                                    the listing
//!     sessions log home 20 --since 22:00 --full   the conversation, both sides
//!     sessions last home 3 --user                 what one side said, in full
//!     sessions send home "ready to compact?"      `-` reads stdin
//!
//! On PATH from home-manager; `cargo run -p console --bin sessions -- <args>` in
//! a checkout.
//!
//! Speaks to the ungated loopback port, `CONSOLE_DESK_ADDR`. `send` arrives as the
//! user, unmarked, so it must be in their words.

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use console::config::Config;
use console::conversation::{Voice, conversation};
use serde::Deserialize;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// How much of a transcript's tail to read.
const TAIL: u64 = 8 << 20;

/// How much of a message `log` shows before saying how much it kept back.
const WIDTH: usize = 700;

#[derive(Deserialize)]
struct Overview {
    sessions: Vec<Row>,
}

/// The fields of the console's session summary this tool shows.
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
    /// When the conversation last moved, in milliseconds.
    #[serde(default)]
    touched: Option<u64>,
    #[serde(default)]
    unread: usize,
    #[serde(default)]
    waiting: usize,
    /// Commands still running after their turn returned.
    #[serde(default)]
    background: usize,
    #[serde(default)]
    running: Vec<Running>,
    #[serde(default)]
    context: Option<u64>,
    #[serde(default)]
    window: Option<u64>,
}

/// One command still in flight, by the `label` a person wrote.
#[derive(Deserialize)]
struct Running {
    #[serde(default)]
    tool: Option<String>,
    #[serde(default)]
    label: Option<String>,
}

impl Running {
    fn describe(&self) -> Option<&str> {
        self.label.as_deref().or(self.tool.as_deref())
    }
}

impl Row {
    fn label(&self) -> &str {
        self.name.as_deref().unwrap_or("(unnamed)")
    }

    fn state(&self) -> &'static str {
        match (
            self.alive,
            self.working,
            self.waiting > 0,
            self.background > 0,
        ) {
            (false, ..) => "exited",
            (_, _, true, _) => "waiting",
            (_, true, ..) => "working",
            // Idle for input, not idle: one is safe to compact, the other is not.
            (.., true) => "bg",
            _ => "idle",
        }
    }

    /// What it is doing, whether or not a turn is running.
    fn doing(&self) -> String {
        if self.working {
            return self.busy.clone().unwrap_or_default();
        }
        if self.background == 0 {
            return String::new();
        }
        let names: Vec<&str> = self.running.iter().filter_map(Running::describe).collect();
        match names.is_empty() {
            true => format!("{} background", self.background),
            false => format!("{} background: {}", self.background, names.join(", ")),
        }
    }
}

/// The live Claude Code sessions on this Mac: list them, read a conversation,
/// send one a message. A <SESSION> is a name (`home`) or the start of an id
/// (`1b6f2e45`).
#[derive(Parser)]
struct Cli {
    /// The listing when absent.
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// The sessions: state, what they are doing, how long ago.
    Who,
    /// The last N messages of the conversation, both sides.
    Log(Conversation),
    /// The last N things the session said, in full.
    Last(Conversation),
    /// Send it a message, as the user; `-` reads stdin.
    Send {
        session: String,
        /// The message, every word after the session taken literally.
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        text: Vec<String>,
    },
}

/// Which conversation, and how much of it.
#[derive(clap::Args)]
struct Conversation {
    session: String,
    /// How many [default: 10 for log, 1 for last].
    n: Option<usize>,
    /// With `last`, show what the user said instead.
    #[arg(short, long)]
    user: bool,
    /// With `log`, do not shorten long messages.
    #[arg(short, long)]
    full: bool,
    /// Only messages at or after T: `22:00` today, or a full ISO stamp.
    #[arg(long, value_name = "T")]
    since: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    // rustls has no default provider; without this the first request panics.
    let _ = rustls::crypto::ring::default_provider().install_default();

    match Cli::parse().command.unwrap_or(Command::Who) {
        Command::Who => who().await,
        Command::Log(args) => read(args, Mode::Log).await,
        Command::Last(args) => read(args, Mode::Last).await,
        Command::Send { session, text } => send(&session, &text).await,
    }
}

enum Mode {
    Log,
    Last,
}

/// The console's ungated loopback address, from the same place the server reads it.
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
        // `busy` only while working; background work outlives the turn.
        let doing = row.doing();
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

/// A name, or the start of an id. Names win.
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

/// Where a session's conversation is on disk, found by looking.
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
    // A seek lands mid-line.
    if from > 0
        && let Some(newline) = buf.iter().position(|byte| *byte == b'\n')
    {
        buf.drain(..=newline);
    }
    Ok(buf)
}

/// `--since 22:00` means today at 22:00; a longer value is compared as a prefix.
fn since(value: &str) -> String {
    if value.len() == 5 && value.as_bytes()[2] == b':' {
        let today = &time::OffsetDateTime::now_utc();
        format!(
            "{:04}-{:02}-{:02}T{value}",
            today.year(),
            today.month() as u8,
            today.day()
        )
    } else {
        value.to_string()
    }
}

async fn read(args: Conversation, mode: Mode) -> Result<()> {
    let (mine, full) = (args.user, args.full);
    let since = args.since.as_deref().map(since);
    let needle = args.session.as_str();
    let count = args.n.unwrap_or(match mode {
        Mode::Log => 10,
        Mode::Last => 1,
    });

    let sessions = overview().await?;
    let row = resolve(&sessions, needle)?;
    let bytes = tail_of(&transcript(&row.id)?)?;
    let mut lines = conversation(&bytes);

    if let Some(since) = &since {
        lines.retain(|line| line.at.as_str() >= since.as_str());
    }
    if let Mode::Last = mode {
        let want = if mine { Voice::User } else { Voice::Session };
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
            Voice::User => "user",
            Voice::Session => row.label(),
        };
        let stamp = &line.at[..19.min(line.at.len())];
        let body = match (
            matches!(mode, Mode::Log) && !full,
            line.text.char_indices().nth(WIDTH),
        ) {
            // Say how much was kept back.
            (true, Some((cut, _))) => format!(
                "{}\n    … +{} more chars",
                &line.text[..cut],
                line.text.chars().count() - WIDTH
            ),
            _ => line.text.clone(),
        };
        println!("=== {stamp} {who}\n{body}\n");
    }
    // Say what was left out.
    if lines.len() > shown {
        println!("({} earlier, ask for more)", lines.len() - shown);
    }
    Ok(())
}

async fn send(needle: &str, words: &[String]) -> Result<()> {
    let text = if words == ["-"] {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        buf
    } else {
        words.join(" ")
    };
    let text = text.trim();
    // The API would accept an empty message.
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
    // The receipt — a doubled message leaves no other trace. The name comes from
    // the resolved row; the summary has none.
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
