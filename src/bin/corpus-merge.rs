//! Merge the night's fresh Bash corpus into the union, and refuse to write a
//! union that has lost a command.
//!
//!     cargo run --release --bin corpus-merge -- union.jsonl fresh.jsonl out.jsonl
//!
//! Lives here rather than in `claude-corpus-snapshot.sh` because the rule it
//! applies is the kind that has to be argued with a test. The shell that calls
//! it still owns the lock, the dated snapshot and the atomic rename — this does
//! one thing and says what it did.
//!
//! ⚠ **Writes to `out` and never touches `union`.** The caller renames, so a
//! merge that dies half way leaves the union it started with.

use std::io::Write;

use memview::bash_corpus::merge;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [union_path, fresh_path, out_path] = args.as_slice() else {
        anyhow::bail!("usage: corpus-merge <union> <fresh> <out>");
    };

    // A first run has no union, which is not an error. Anything else that fails
    // to read IS one: an unreadable union must never be merged as if it were
    // empty, because the result would pass every check and hold only tonight.
    let union = match std::fs::read_to_string(union_path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(anyhow::Error::new(err).context(format!("reading {union_path}"))),
    };
    let fresh = std::fs::read_to_string(fresh_path)?;

    let rows_before = union.lines().filter(|l| !l.trim().is_empty()).count();
    let merged = merge(&union, &fresh);

    if !merged.safe() {
        anyhow::bail!(
            "⚠ the union would lose {} distinct commands, {} → {} — refusing",
            merged.subjects_before - merged.subjects_after,
            merged.subjects_before,
            merged.subjects_after,
        );
    }

    let mut out = std::io::BufWriter::new(std::fs::File::create(out_path)?);
    for row in &merged.rows {
        writeln!(out, "{row}")?;
    }
    out.flush()?;

    // Rows and subjects are reported side by side on purpose: a fall in rows is
    // only good news next to a subject count that held.
    eprintln!(
        "union {} → {} rows ({:+}), {} collapsed into a timestamped twin; \
         subjects {} → {} ({:+})",
        rows_before,
        merged.rows.len(),
        merged.rows.len() as i64 - rows_before as i64,
        merged.collapsed,
        merged.subjects_before,
        merged.subjects_after,
        merged.subjects_after as i64 - merged.subjects_before as i64,
    );
    if merged.unparsed > 0 {
        eprintln!(
            "⚠ {} line(s) were not JSON objects, kept verbatim",
            merged.unparsed
        );
    }
    Ok(())
}
