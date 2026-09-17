//! Where the bytes in a transcript actually are. `claude_disk.py` charts three
//! lines that do not sum to its total, because they name WHERE bytes sit and
//! never WHAT they are (memview#1199, #1200).
//!
//! The buckets PARTITION: every byte lands in exactly one, so the report sums to
//! the file on disk. Copy is the dimension nothing had — the CLI re-appends
//! earlier stretches, and on the largest transcript that is 48.5% of 1.7 GB.

use std::collections::BTreeMap;

use serde::Serialize;

/// Whether these bytes are a message's first appearance in the file, or a
/// re-appended copy of one already seen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Copy {
    First,
    Repeat,
}

/// What a run of bytes is, at the granularity a reader can act on.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Kind {
    /// The model's reasoning.
    Thinking,
    /// What the model said.
    AssistantText,
    /// What the user typed.
    UserText,
    /// A call's arguments, by tool.
    ToolUse(String),
    /// What a call returned, by tool where the call is known.
    ToolResult(String),
    /// Claude Code's per-turn injected attachments.
    Attachment,
    /// Claude Code's own before/after file snapshots.
    FileHistory,
    /// The JSON around the content. Named rather than distributed across the parts,
    /// so it is a fact a reader can see.
    Envelope,
    /// A line type nothing above names, kept verbatim so the partition holds.
    Other(String),
}

/// Bytes by bucket. The two dimensions are orthogonal: "how much is repeats" is
/// asked of every content kind.
#[derive(Debug, Default, Serialize)]
pub struct Bytes {
    pub by: BTreeMap<(Copy, Kind), u64>,
    /// Lines that could not be parsed as JSON. Counted, never skipped: dropping
    /// them would break the partition silently.
    pub unparseable: u64,
    pub lines: u64,
    pub messages: u64,
}

impl Bytes {
    pub fn add(&mut self, copy: Copy, kind: Kind, n: u64) {
        *self.by.entry((copy, kind)).or_default() += n;
    }

    /// Every byte this has accounted for.
    pub fn total(&self) -> u64 {
        self.by.values().sum::<u64>() + self.unparseable
    }

    /// Bytes in re-appended copies, over the whole.
    pub fn repeat_share(&self) -> f64 {
        let repeats: u64 = self
            .by
            .iter()
            .filter(|((c, _), _)| *c == Copy::Repeat)
            .map(|(_, n)| *n)
            .sum();
        let total = self.total();
        if total == 0 {
            0.0
        } else {
            repeats as f64 / total as f64
        }
    }
}

/// Which tool a call names, or `?` when the line does not say.
fn tool_of(part: &serde_json::Value) -> String {
    part["name"].as_str().unwrap_or("?").to_string()
}

/// Bucket one transcript line. `seen` is per FILE: the same uuid in two
/// transcripts is two conversations referring to one message.
///
/// `raw` is the bytes the line OCCUPIES, newline included, counted while reading:
/// `line.len()` strips the terminator, and comparing against a later
/// `metadata()` measured a file still being written — 13,611 bytes unexplained.
pub fn absorb(
    out: &mut Bytes,
    line: &str,
    raw: u64,
    seen: &mut std::collections::HashSet<String>,
    calls: &mut BTreeMap<String, String>,
) -> anyhow::Result<()> {
    out.lines += 1;
    // A line that is not JSON is DATA: the one deserialise here allowed to fall
    // back, into a NAMED bucket.
    let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else {
        out.unparseable += raw;
        return Ok(());
    };

    let copy = match row["uuid"].as_str() {
        Some(id) if !seen.insert(id.to_string()) => Copy::Repeat,
        Some(_) => {
            out.messages += 1;
            Copy::First
        }
        // A line with no uuid cannot be told from its own second copy, so it is a first
        // appearance rather than a guess.
        None => Copy::First,
    };

    let kind_of_line = match row["type"].as_str().unwrap_or("?") {
        "attachment" => Some(Kind::Attachment),
        "file-history-snapshot" => Some(Kind::FileHistory),
        "assistant" | "user" => None,
        other => Some(Kind::Other(other.to_string())),
    };
    if let Some(kind) = kind_of_line {
        out.add(copy, kind, raw);
        return Ok(());
    }

    // Charge each content part its own serialised size and the remainder to the
    // envelope, so the parts and the line agree exactly.
    let content = &row["message"]["content"];
    let mut accounted = 0u64;
    if let Some(parts) = content.as_array() {
        for part in parts {
            // Propagated, never defaulted to zero: charged as 0 the part's bytes fall
            // silently into `Envelope`, and the partition still balances while the
            // attribution is wrong.
            let size = serde_json::to_string(part)?.len() as u64;
            let kind = match part["type"].as_str().unwrap_or("?") {
                "thinking" | "redacted_thinking" => Kind::Thinking,
                "tool_use" => {
                    if let (Some(id), Some(name)) = (part["id"].as_str(), part["name"].as_str()) {
                        calls.insert(id.to_string(), name.to_string());
                    }
                    Kind::ToolUse(tool_of(part))
                }
                "tool_result" => {
                    // The result names only the call, so the tool comes from the `tool_use` that opened it.
                    let tool = part["tool_use_id"]
                        .as_str()
                        .and_then(|id| calls.get(id))
                        .cloned()
                        .unwrap_or_else(|| "?".to_string());
                    Kind::ToolResult(tool)
                }
                "text" if row["type"].as_str() == Some("assistant") => Kind::AssistantText,
                "text" => Kind::UserText,
                other => Kind::Other(other.to_string()),
            };
            // Never charge a part more than the line has left: re-applied escaping can
            // exceed its share, and the envelope would go below zero.
            let size = size.min(raw - accounted);
            out.add(copy, kind, size);
            accounted += size;
        }
    } else if content.is_string() {
        let size = (serde_json::to_string(content)?.len() as u64).min(raw);
        let kind = if row["type"].as_str() == Some("assistant") {
            Kind::AssistantText
        } else {
            Kind::UserText
        };
        out.add(copy, kind, size);
        accounted += size;
    }
    out.add(copy, Kind::Envelope, raw - accounted);
    Ok(())
}

/// One top-level entry of `~/.claude` and what it costs — the WHERE dimension,
/// carrying a REMAINDER so the parts sum by construction (memview#1199, #1200).
#[derive(Debug, Clone, Serialize)]
pub struct Part {
    pub name: String,
    pub bytes: u64,
    pub files: u64,
}

/// Bytes and file counts per top-level entry, largest first. Apparent size, not
/// allocated blocks: `du` would disagree by block size times file count, and
/// `file-history/` holds 25,000 small files.
pub fn top_level(root: &std::path::Path) -> std::io::Result<Vec<Part>> {
    let mut parts = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let (bytes, files) = weigh(&entry.path());
        parts.push(Part {
            name: entry.file_name().to_string_lossy().into_owned(),
            bytes,
            files,
        });
    }
    parts.sort_by_key(|p| std::cmp::Reverse(p.bytes));
    Ok(parts)
}

/// Bytes and file count under a path. Symlinks are counted as links, never
/// followed: `~/.claude` is itself a symlink to an external volume.
fn weigh(path: &std::path::Path) -> (u64, u64) {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return (0, 0);
    };
    if meta.file_type().is_symlink() {
        return (meta.len(), 1);
    }
    if meta.is_file() {
        return (meta.len(), 1);
    }
    let mut bytes = 0;
    let mut files = 0;
    let Ok(dir) = std::fs::read_dir(path) else {
        return (0, 0);
    };
    for entry in dir.flatten() {
        let (b, f) = weigh(&entry.path());
        bytes += b;
        files += f;
    }
    (bytes, files)
}

/// How much of a set of file sizes the largest `n` hold — the shape that decides
/// what a cleanup could ever be worth.
pub fn concentration(mut sizes: Vec<u64>, n: usize) -> (u64, u64, usize) {
    sizes.sort_unstable_by_key(|b| std::cmp::Reverse(*b));
    let top: u64 = sizes.iter().take(n).sum();
    let rest: u64 = sizes.iter().skip(n).sum();
    (top, rest, sizes.len().saturating_sub(n))
}

/// The census as an artefact, for a collector that must not read 6 GB itself.
/// Every field becomes a fleetwatch TREND KEY, forever, so the bucket names are a
/// FIXED set with an `other` catch-all, never the raw [`Kind`].
#[derive(Debug, Default, Serialize, serde::Deserialize)]
pub struct Census {
    /// When the walk finished, so a reader can grade its own staleness.
    pub at: String,
    pub total_bytes: u64,
    pub lines: u64,
    pub messages: u64,
    /// Bytes per top-level entry, `remainder` included so the parts sum (memview#1200).
    pub where_bytes: BTreeMap<String, u64>,
    /// Bytes per content bucket, over the transcripts only.
    pub what_bytes: BTreeMap<String, u64>,
    /// Of `total_bytes`, how many are re-appended copies.
    pub repeat_bytes: u64,
    /// Bytes in the largest 16 transcripts, and in all the others.
    pub top16_bytes: u64,
    pub rest_bytes: u64,
}

/// The stable label for a bucket, or `None` for one that folds into `other`.
/// Adding a name here adds a permanent chart line: the set is the buckets above
/// 1% when first measured.
pub fn stable_label(kind: &Kind) -> Option<&'static str> {
    Some(match kind {
        Kind::Envelope => "envelope",
        Kind::Thinking => "thinking",
        Kind::FileHistory => "file-history in transcript",
        Kind::Attachment => "attachment",
        Kind::AssistantText => "assistant text",
        Kind::UserText => "user text",
        Kind::ToolUse(t) if t == "Bash" => "call: Bash",
        Kind::ToolUse(t) if t == "Edit" => "call: Edit",
        Kind::ToolUse(t) if t == "Write" => "call: Write",
        Kind::ToolResult(t) if t == "Bash" => "result: Bash",
        Kind::ToolResult(t) if t == "Read" => "result: Read",
        Kind::ToolResult(t) if t == "Edit" => "result: Edit",
        _ => return None,
    })
}

impl Bytes {
    /// Fold into the fixed label set, with everything unnamed under `other`.
    pub fn stable(&self) -> BTreeMap<String, u64> {
        let mut out: BTreeMap<String, u64> = BTreeMap::new();
        for ((_, kind), n) in &self.by {
            let label = stable_label(kind).unwrap_or("other");
            *out.entry(label.to_string()).or_default() += n;
        }
        if self.unparseable > 0 {
            *out.entry("other".to_string()).or_default() += self.unparseable;
        }
        out
    }
}
