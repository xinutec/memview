//! Where the bytes in a transcript actually are.
//!
//! `claude_disk.py` charts three lines — transcripts, file history, uploads —
//! and they do not sum to the total it also charts, because they name WHERE
//! bytes sit and never WHAT they are (memview#1199, #1200).
//!
//! ⚠ **The buckets PARTITION, and that is the whole design constraint.** Every
//! byte of every line lands in exactly one, so the report sums to the file on
//! disk. Overlapping categories are what produced the three lines that do not
//! add up; a bucket that double-counts is worse than a missing one, because it
//! makes the total look explained.
//!
//! ⚠ **Copy is the dimension nothing had.** The CLI re-appends earlier stretches
//! of a conversation into the same file, so a transcript holds many messages
//! twice. Measured 2026-08-29 on the largest one, that is **48.5% of 1.7 GB** —
//! bigger than any content category. A report that shows only what the bytes ARE
//! answers "reads and edits are large" and misses that half of them are second
//! copies of themselves.

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
    /// The JSON around the content: uuid, timestamps, parent links, and the
    /// separators between parts.
    ///
    /// ⚠ **Named rather than distributed.** Spreading it across the content
    /// parts would make every other number slightly wrong in a way nothing could
    /// check; as its own bucket it is a fact about the format that a reader can
    /// see and judge.
    Envelope,
    /// A line type nothing above names, kept verbatim so the partition holds and
    /// a new format shows up as itself rather than vanishing into `Envelope`.
    Other(String),
}

/// Bytes by bucket. The two dimensions are orthogonal on purpose: "how much of
/// this is repeats" is asked of every content kind, not of the file as a whole.
#[derive(Debug, Default, Serialize)]
pub struct Bytes {
    pub by: BTreeMap<(Copy, Kind), u64>,
    /// Lines that could not be parsed as JSON. ⚠ Counted, never skipped — a
    /// damaged line is still bytes on disk, and dropping it would break the
    /// partition silently.
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

/// Bucket one transcript line.
///
/// `seen` carries the message uuids already met in this file, so the second
/// appearance of one is charged to [`Copy::Repeat`]. It is per FILE, not per
/// corpus: the same uuid in two transcripts is two conversations referring to
/// one message, not a duplicate on disk.
///
/// ⚠ **`raw` is the bytes the line OCCUPIES, newline included, and the caller
/// must have counted them while reading.** Deriving it from `line.len()` was
/// wrong twice over: the terminator is already stripped, and comparing the
/// result against a later `metadata()` call measures a file that is still being
/// written — the first run over a live transcript reported 13,611 bytes
/// unexplained and the whole discrepancy was the file growing underneath it.
/// Counting what was consumed makes the partition exact by construction.
pub fn absorb(
    out: &mut Bytes,
    line: &str,
    raw: u64,
    seen: &mut std::collections::HashSet<String>,
    calls: &mut BTreeMap<String, String>,
) -> anyhow::Result<()> {
    out.lines += 1;
    // ⚠ A line that is not JSON is DATA, not an error — a damaged transcript is
    // still bytes on disk. This is the one (de)serialise here allowed to fall
    // back, and it falls back into a NAMED bucket rather than into nothing.
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
        // A line with no uuid cannot be told from its own second copy, so it is
        // charged as a first appearance rather than guessed at.
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

    // A message: charge each content part its own serialised size and give the
    // remainder to the envelope, so the parts and the line agree exactly.
    let content = &row["message"]["content"];
    let mut accounted = 0u64;
    if let Some(parts) = content.as_array() {
        for part in parts {
            // ⚠ **Propagated, never defaulted to zero.** A `Value` parsed from
            // this line re-serialises by construction, so a failure is
            // impossible-in-practice — which is precisely why swallowing it
            // would never be noticed. Charged as 0, the part's bytes fall
            // silently into `Envelope`: the partition still balances and the
            // attribution is wrong, which is the one failure this design exists
            // to make impossible.
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
                    // The result names only the call, so the tool comes from the
                    // `tool_use` that opened it — which is why calls are carried.
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
            // Never charge a part more than the line has left: a part's own
            // serialisation can exceed its share once escaping is re-applied,
            // and an over-charge would put the envelope below zero.
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

/// One top-level entry of `~/.claude` and what it costs.
///
/// ⚠ **The WHERE dimension, and it exists to carry a REMAINDER.** `claude_disk.py`
/// charts transcripts, file history and uploads against a total they do not add
/// up to, so the gap between them is invisible and unnamed. A census that names
/// every entry plus "everything else" cannot have that gap: the parts sum by
/// construction or the walk is wrong (memview#1199, #1200).
#[derive(Debug, Clone, Serialize)]
pub struct Part {
    pub name: String,
    pub bytes: u64,
    pub files: u64,
}

/// Bytes and file counts per top-level entry, largest first.
///
/// ⚠ **Apparent size, not allocated blocks**, so it agrees with the byte census
/// over the same transcripts. `du` reports allocation and would disagree with
/// every other figure here by the filesystem's block size times the file count —
/// 25,000 small files under `file-history/` is where that difference stops being
/// rounding.
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

/// Bytes and file count under a path, following nothing.
///
/// ⚠ **Symlinks are counted as links, never followed.** `~/.claude` is itself a
/// symlink to an external volume and several entries under it point elsewhere;
/// following them would count another disk's bytes into this total and could
/// recurse forever.
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

/// How much of a set of file sizes the largest `n` hold.
///
/// ⚠ **The shape that decides what a cleanup could ever be worth.** A corpus of
/// a thousand equal files and one where sixteen hold 97% need different answers,
/// and the byte census cannot tell them apart — it says what bytes ARE and
/// nothing about how they are distributed across files.
pub fn concentration(mut sizes: Vec<u64>, n: usize) -> (u64, u64, usize) {
    sizes.sort_unstable_by_key(|b| std::cmp::Reverse(*b));
    let top: u64 = sizes.iter().take(n).sum();
    let rest: u64 = sizes.iter().skip(n).sum();
    (top, rest, sizes.len().saturating_sub(n))
}
