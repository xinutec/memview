//! What the bytes in `~/.claude/projects` actually are.
//!
//!     cargo run --release --bin corpus-bytes            # the whole corpus
//!     cargo run --release --bin corpus-bytes -- <file>  # one transcript
//!
//! ⚠ **`claude_disk.py` charts three lines that do not sum to the total beside
//! them** — transcripts, file history, uploads. They name WHERE bytes are and
//! never WHAT they are, so "what is it growing on?" has had no answer
//! (memview#1199; #1200 is pushing this to fleetwatch).
//!
//! ⚠ **The partition is checked against the filesystem, not asserted.** Every
//! byte of every line is charged to exactly one bucket and the total is compared
//! with the size on disk; a mismatch is printed as a failure rather than
//! rounded away. A report that explains 97% of a corpus while claiming to
//! explain all of it is the defect this replaces, one level down.

use std::collections::{BTreeMap, HashSet};
use std::io::BufRead;

use anyhow::{Context, Result};
use memview::bytes::{Bytes, Copy, Kind, absorb};

fn label(kind: &Kind) -> String {
    match kind {
        Kind::Thinking => "thinking".into(),
        Kind::AssistantText => "assistant text".into(),
        Kind::UserText => "user text".into(),
        Kind::ToolUse(t) => format!("call: {t}"),
        Kind::ToolResult(t) => format!("result: {t}"),
        Kind::Attachment => "attachment (injected)".into(),
        Kind::FileHistory => "file-history snapshot".into(),
        Kind::Envelope => "envelope (uuid, stamps, links)".into(),
        Kind::Other(t) => format!("line: {t}"),
    }
}

fn main() -> Result<()> {
    let arg = std::env::args().nth(1);
    let files: Vec<std::path::PathBuf> = match &arg {
        Some(one) => vec![std::path::PathBuf::from(one)],
        None => memview::blame::transcripts(&reader::home::projects_dir()),
    };

    // ⚠ **The WHERE dimension first, because it is the one with a REMAINDER.**
    // `claude_disk.py` charts three named parts against a total they do not sum
    // to, so whatever is not transcripts, file history or uploads is invisible.
    // Naming every entry plus "everything else" makes that impossible.
    if arg.is_none() {
        let root = reader::home::claude_dir();
        let parts = memview::bytes::top_level(&root)?;
        let total: u64 = parts.iter().map(|p| p.bytes).sum();
        println!(
            "{} — {:.2} GB in {} entries\n",
            root.display(),
            total as f64 / 1e9,
            parts.len()
        );
        let mut shown = 0u64;
        for part in parts.iter().take(8) {
            shown += part.bytes;
            println!(
                "  {:<24} {:>7.2} GB {:>6.1}%  {:>7} files",
                part.name,
                part.bytes as f64 / 1e9,
                100.0 * part.bytes as f64 / total as f64,
                part.files
            );
        }
        // The line that makes the rest honest.
        println!(
            "  {:<24} {:>7.2} GB {:>6.1}%  (everything else, {} entries)",
            "remainder",
            (total - shown) as f64 / 1e9,
            100.0 * (total - shown) as f64 / total as f64,
            parts.len().saturating_sub(8)
        );
    }

    let mut all = Bytes::default();
    // What we actually consumed, and what the filesystem claimed at the end.
    // They differ on a live corpus and the difference is the finding, not an
    // error — see the note below.
    let mut read_bytes = 0u64;
    let mut on_disk = 0u64;
    let mut sizes: Vec<u64> = Vec::new();
    for path in &files {
        let file = std::fs::File::open(path).with_context(|| format!("{}", path.display()))?;
        // ⚠ Per FILE, not per corpus. The same uuid in two transcripts is one
        // message two conversations refer to, not a second copy on disk.
        let mut seen = HashSet::new();
        let mut calls = BTreeMap::new();
        let mut reader = std::io::BufReader::new(file);
        // ⚠ `read_until` rather than `lines()`: it hands back the terminator, so
        // the bytes charged are the bytes the line occupies. `lines()` strips it
        // and any reconstruction is a guess that a live file will not match.
        let mut buf: Vec<u8> = Vec::new();
        loop {
            buf.clear();
            let n = reader.read_until(b'\n', &mut buf)? as u64;
            if n == 0 {
                break;
            }
            read_bytes += n;
            let text = String::from_utf8_lossy(&buf);
            absorb(
                &mut all,
                text.trim_end_matches('\n'),
                n,
                &mut seen,
                &mut calls,
            )
            .with_context(|| format!("in {}", path.display()))?;
        }
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        on_disk += size;
        sizes.push(size);
    }

    let total = all.total();
    println!(
        "{} transcript(s), {:.2} GB accounted, {} lines, {} distinct messages",
        files.len(),
        total as f64 / 1e9,
        all.lines,
        all.messages
    );

    // ⚠ **The check that makes the rest worth reading.** Every byte consumed is
    // charged to exactly one bucket, so this compares the buckets against the
    // read and not against a guess.
    if total != read_bytes {
        println!("⚠ PARTITION BROKEN: {total} in buckets against {read_bytes} read");
    } else {
        println!("partition holds: every byte read is in exactly one bucket below");
    }
    // ⚠ **A live corpus grows underneath the walk, and that is not an error.**
    // The first run of this compared its total against `metadata()` and reported
    // 13,611 bytes unexplained; every one of them was another session appending
    // to its own transcript while this read it. Reported as drift, separately,
    // so it can never be mistaken for a leaking bucket.
    if on_disk != read_bytes {
        println!(
            "  (the corpus moved while reading: {:+} bytes against the size afterwards — \
             sessions were writing)",
            on_disk as i64 - read_bytes as i64
        );
    }

    // Roll the two dimensions up separately: what the bytes are, and how much of
    // each is a second copy. A single sorted list of pairs would bury the
    // finding that the largest category is duplication rather than content.
    let mut by_kind: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    for ((copy, kind), n) in &all.by {
        let slot = by_kind.entry(label(kind)).or_default();
        match copy {
            Copy::First => slot.0 += n,
            Copy::Repeat => slot.1 += n,
        }
    }
    let mut rows: Vec<(String, u64, u64)> =
        by_kind.into_iter().map(|(k, (f, r))| (k, f, r)).collect();
    rows.sort_by_key(|(_, f, r)| std::cmp::Reverse(f + r));

    println!(
        "\n  {:<34} {:>9} {:>9} {:>8} {:>7}",
        "", "first", "repeat", "repeat%", "share%"
    );
    for (name, first, repeat) in rows.iter().take(20) {
        let sum = first + repeat;
        println!(
            "  {name:<34} {:>7.2} GB {:>7.2} GB {:>7.1} {:>6.1}",
            *first as f64 / 1e9,
            *repeat as f64 / 1e9,
            if sum == 0 {
                0.0
            } else {
                100.0 * *repeat as f64 / sum as f64
            },
            100.0 * sum as f64 / total as f64,
        );
    }
    if all.unparseable > 0 {
        println!(
            "  {:<34} {:>7.2} GB",
            "unparseable lines",
            all.unparseable as f64 / 1e9
        );
    }
    // ⚠ **Concentration decides what any cleanup could ever be worth**, and the
    // byte census cannot see it: it says what bytes ARE and nothing about how
    // they are spread across files.
    let (top, rest, others) = memview::bytes::concentration(sizes, 16);
    println!(
        "\n  the 16 largest transcripts hold {:.2} GB; the other {others} share {:.2} GB ({:.1}% vs {:.1}%)",
        top as f64 / 1e9,
        rest as f64 / 1e9,
        100.0 * top as f64 / (top + rest).max(1) as f64,
        100.0 * rest as f64 / (top + rest).max(1) as f64,
    );
    println!(
        "\n  re-appended copies are {:.1}% of the corpus — the CLI writes earlier stretches\n  \
         of a conversation back into the same file (reference_claude_transcript_rewrites_history)",
        all.repeat_share() * 100.0
    );
    Ok(())
}
