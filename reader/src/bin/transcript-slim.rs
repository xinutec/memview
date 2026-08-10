//! Empty the task-reminder payloads in a transcript, keeping every node.
//!
//! One session accumulated 2.95 GB of re-injected task lists — 73.8% of a 4.00
//! GB file, and 45% of the whole corpus — because every turn carried the full
//! list again. `claude_tasks.py` fixed the cause on 2026-08-08 (the same nodes
//! now arrive carrying nothing), so this is a one-off clearing of what was
//! already written, not a recurring sweep.
//!
//! ⚠ **Deleting the lines is not an option and was measured, not assumed.** A
//! reminder is an `attachment` node with its own `uuid` AND `parentUuid`: it is
//! a NODE IN THE TREE, and 5,679 of them are named as the parent of another
//! line. Removing them severs the chain the conversation is threaded on, and
//! the CLI walks that chain backwards from the last record and silently drops
//! whatever it cannot reach. So the node stays and only its payload goes.
//!
//! **The shape written here is one the CLI already writes.** 844 nodes in this
//! very file are `"content":[],"itemCount":0` — 151 of them this month —
//! because that is what a reminder looks like when there are no tasks. Nothing
//! novel is being invented for a reader to choke on, which is what made a
//! resume experiment unnecessary.
//!
//!     transcript-slim FILE              # dry run
//!     transcript-slim --apply FILE
//!
//! `--apply` never edits in place: the file is being appended to, so see `swap`.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use reader::transcript::{self, Tail};

/// The payload we clear, and the exact form we leave behind.
const REMINDER: &str = "task_reminder";

fn main() -> Result<()> {
    let mut apply = false;
    let mut target: Option<PathBuf> = None;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--apply" => apply = true,
            other => target = Some(PathBuf::from(other)),
        }
    }
    let path = target.context("usage: transcript-slim [--apply] FILE")?;

    let cut = std::fs::metadata(&path)
        .with_context(|| format!("reading {}", path.display()))?
        .len();

    let temp = path.with_extension("jsonl.slim-tmp");
    let report = rewrite(&path, &temp, cut)?;

    println!("{}", path.display());
    println!(
        "  {} reminder nodes emptied of {} already empty",
        report.emptied, report.already_empty
    );
    println!(
        "  {:.2} GB -> {:.2} GB  ({:.2} GB reclaimed, {} lines untouched)",
        cut as f64 / 1e9,
        report.written as f64 / 1e9,
        (cut - report.written) as f64 / 1e9,
        report.untouched
    );

    if !apply {
        // Worth saying out loud rather than discarding: the rewrite it just
        // built is the size of the file itself, so a failure here leaves a
        // gigabyte behind under a name nothing will ever look at again.
        if let Err(err) = std::fs::remove_file(&temp) {
            eprintln!("could not remove {}: {err}", temp.display());
        }
        println!("\ndry run — nothing swapped. Pass --apply.");
        return Ok(());
    }

    let backup = swap(&path, &temp, cut)?;
    println!("\nswapped. previous file kept at {}", backup.display());

    let bytes = std::fs::read(&path)?;
    let found = transcript::check(
        &bytes,
        Tail::MayBeIncomplete,
        path.file_stem().and_then(|s| s.to_str()),
    );
    let damage: Vec<_> = found.iter().filter(|v| v.rule.is_damage()).collect();
    if !damage.is_empty() {
        for violation in damage.iter().take(5) {
            eprintln!("  line {}: {}", violation.line, violation.rule.name());
        }
        bail!(
            "the rewritten file is DAMAGED — restore it with:\n    mv {} {}",
            backup.display(),
            path.display()
        );
    }
    println!("verified: the rewritten file is intact");
    Ok(())
}

#[derive(Default)]
struct Report {
    emptied: usize,
    already_empty: usize,
    untouched: usize,
    written: u64,
}

/// Stream `path` into `temp`, clearing reminder payloads, up to byte `cut`.
///
/// Only the reminder lines are re-serialised. Every other line is copied
/// verbatim, byte for byte — the viewer scans raw bytes for needles like
/// `{"type":"agent-name","agentName":"` that depend on key ORDER, and
/// re-encoding a line it never needed to touch would be a gratuitous way to
/// break one.
fn rewrite(path: &Path, temp: &Path, cut: u64) -> Result<Report> {
    let source = std::fs::File::open(path)?;
    let mut source = std::io::BufReader::with_capacity(1 << 20, source.take(cut));
    let mut sink = std::io::BufWriter::with_capacity(1 << 20, std::fs::File::create(temp)?);

    let mut report = Report::default();
    let mut line = Vec::new();
    loop {
        line.clear();
        let read = read_line(&mut source, &mut line)?;
        if read == 0 {
            break;
        }
        match slim(&line)? {
            Some(replacement) => {
                report.emptied += 1;
                report.written += replacement.len() as u64;
                sink.write_all(&replacement)?;
            }
            None => {
                if line
                    .windows(REMINDER.len())
                    .any(|w| w == REMINDER.as_bytes())
                {
                    report.already_empty += 1;
                } else {
                    report.untouched += 1;
                }
                report.written += line.len() as u64;
                sink.write_all(&line)?;
            }
        }
    }
    sink.flush()?;
    Ok(report)
}

fn read_line(source: &mut impl std::io::BufRead, into: &mut Vec<u8>) -> Result<usize> {
    Ok(source.read_until(b'\n', into)?)
}

/// The emptied form of a populated reminder line, or `None` to leave it alone.
///
/// Parses, mutates, re-encodes, and then **proves the result differs only where
/// intended** by comparing the two values field by field. A hand-rolled scan
/// over JSON with escaped strings is exactly where a subtle bug would live, so
/// the check is not optional: any other difference aborts the whole run rather
/// than writing one wrong line into a medical record.
fn slim(line: &[u8]) -> Result<Option<Vec<u8>>> {
    let trimmed = line.strip_suffix(b"\n").unwrap_or(line);
    if trimmed.is_empty()
        || !trimmed
            .windows(REMINDER.len())
            .any(|w| w == REMINDER.as_bytes())
    {
        return Ok(None);
    }
    let Ok(mut value) = serde_json::from_slice::<serde_json::Value>(trimmed) else {
        return Ok(None);
    };
    if value.get("type").and_then(|t| t.as_str()) != Some("attachment") {
        return Ok(None);
    }
    let Some(attachment) = value.get_mut("attachment").and_then(|a| a.as_object_mut()) else {
        return Ok(None);
    };
    if attachment.get("type").and_then(|t| t.as_str()) != Some(REMINDER) {
        return Ok(None);
    }
    match attachment.get("content") {
        Some(serde_json::Value::Array(items)) if !items.is_empty() => {}
        _ => return Ok(None),
    }

    let before: serde_json::Value = serde_json::from_slice(trimmed)?;
    attachment.insert("content".into(), serde_json::Value::Array(Vec::new()));
    if attachment.contains_key("itemCount") {
        attachment.insert("itemCount".into(), serde_json::Value::from(0));
    }

    let mut left = before.clone();
    let mut right = value.clone();
    for side in [&mut left, &mut right] {
        if let Some(a) = side.get_mut("attachment").and_then(|a| a.as_object_mut()) {
            a.remove("content");
            a.remove("itemCount");
        }
    }
    if left != right {
        bail!("rewriting a reminder changed more than its payload — refusing");
    }

    let mut out = serde_json::to_vec(&value)?;
    out.push(b'\n');
    Ok(Some(out))
}

/// Put the rewritten file in place without losing a concurrent append.
///
/// ⚠ **The whole problem, and why a plain rename is wrong.** Emptying payloads
/// SHRINKS the file, so it cannot be patched in place the way a 36-byte
/// re-parent can. The session is live and never ends — it goes quiet and comes
/// back — and Claude Code appends by open/append/close, holding no descriptor.
/// So between the last byte we copied and the moment we rename, a line may
/// land, and a plain rename would drop it.
///
/// A hard link closes that. `pre-slim` is a second name for the ORIGINAL inode,
/// made instantly and copying nothing. After the rename, new appends open the
/// path afresh and land on the rewritten file, while anything that arrived on
/// the old inode is still reachable through `pre-slim` — so the tail is
/// recovered and appended rather than lost.
fn swap(path: &Path, temp: &Path, cut: u64) -> Result<PathBuf> {
    // Catch up first, so the window the hard link has to cover is as small as
    // the filesystem will allow rather than as large as the rewrite took.
    let mut at = cut;
    loop {
        let now = std::fs::metadata(path)?.len();
        if now == at {
            break;
        }
        append_range(path, temp, at, now)?;
        at = now;
    }

    let backup = path.with_extension("jsonl.pre-slim");
    if backup.exists() {
        bail!("{} already exists — move it aside first", backup.display());
    }
    std::fs::hard_link(path, &backup)
        .with_context(|| format!("hard-linking {} aside", path.display()))?;
    std::fs::rename(temp, path).context("renaming the rewritten file into place")?;

    // Anything that landed on the old inode after the final catch-up is still
    // there, under the other name.
    let stranded = std::fs::metadata(&backup)?.len();
    if stranded > at {
        eprintln!(
            "recovering {} bytes appended during the swap",
            stranded - at
        );
        append_range(&backup, path, at, stranded)?;
    }
    Ok(backup)
}

/// Copy `from[start..end]` onto the end of `to`, verbatim.
///
/// Verbatim on purpose: these are lines written after the cut, and since the
/// 2026-08-08 cutover a reminder carries nothing anyway, so there is no payload
/// left to clear and no reason to re-encode a live session's newest records.
fn append_range(from: &Path, to: &Path, start: u64, end: u64) -> Result<()> {
    let mut source = std::fs::File::open(from)?;
    source.seek(SeekFrom::Start(start))?;
    let mut sink = std::fs::OpenOptions::new().append(true).open(to)?;
    let mut remaining = end - start;
    let mut buffer = vec![0u8; 1 << 20];
    while remaining > 0 {
        let want = remaining.min(buffer.len() as u64) as usize;
        let read = source.read(&mut buffer[..want])?;
        if read == 0 {
            break;
        }
        sink.write_all(&buffer[..read])?;
        remaining -= read as u64;
    }
    sink.flush()?;
    Ok(())
}
