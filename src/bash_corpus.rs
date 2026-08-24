//! Merging a night's fresh Bash corpus into the union that outlives it.
//!
//! ⚠ **A row and its own timestamped twin are two different LINES.** The union
//! was merged with `sort -u`, which dedups whole lines, while `bash-corpus`
//! writes `at` only when the transcript carried one — and an older version
//! wrote it never. So every command mined before `at` existed sits in the union
//! twice, once bare and once stamped. Measured 2026-08-24: 141,545 of 298,895
//! rows, 47% of the corpus being one era counted twice (memview#1130).
//!
//! ⚠ **Collapsing is done on IDENTITY, not on `(cmd, cwd)`.** Identity is the
//! row with `at` removed. Two rows that agree on identity and both carry a
//! timestamp are the same command seen on two different days, which is real and
//! is kept — collapsing on `(cmd, cwd)` would have thrown away 4,800 of those.
//! Only a row with no timestamp at all is dropped, and only when a row with the
//! same identity has one, because then it is the same observation recorded
//! twice and the twin is strictly better informed.
//!
//! ⚠ **The ratchet moved from ROWS to SUBJECTS, and that is not a weaker
//! check.** The union may never lose a command, and the old guard enforced it by
//! refusing to let the row count fall — which is also what made this bug
//! permanent, since fixing it has to remove rows. Distinct `(cmd, cwd)` is what
//! the union actually promises to preserve; rows are how it happens to store it.
//! So subjects are the thing that may not shrink, and a fall in rows with
//! subjects intact is exactly the shape of a collapsed duplicate.

use std::collections::{BTreeSet, HashSet};

/// What a merge did, in the terms the guard needs to decide whether to keep it.
pub struct Merge {
    /// The merged corpus: deduplicated and byte-sorted, as `LC_ALL=C sort -u`
    /// left it, because the union is read as a sorted file and restic's
    /// content-defined chunking dedups it far better when the order is stable.
    pub rows: Vec<String>,
    /// Rows dropped because a row with the same identity carried a timestamp.
    pub collapsed: usize,
    /// Distinct `(cmd, cwd)` in the union BEFORE the merge — the ratchet's floor.
    pub subjects_before: usize,
    /// Distinct `(cmd, cwd)` after it. Falling below `subjects_before` is the
    /// one outcome that must never be written.
    pub subjects_after: usize,
    /// Lines that were not JSON objects. Kept verbatim and counted rather than
    /// dropped: this is an archive, and a line nobody can parse today is not
    /// evidence that it says nothing.
    pub unparsed: usize,
}

impl Merge {
    /// ⚠ **The precondition, checked on the data rather than assumed of it.**
    /// Dropping untimestamped rows is safe only while every one of them has a
    /// twin, which measured 100% on 2026-08-24 — a fact about that corpus, not
    /// an invariant of it. This is what makes the difference visible if it stops
    /// being true, instead of the loss being silent.
    pub fn safe(&self) -> bool {
        self.subjects_after >= self.subjects_before
    }
}

/// The row's identity: everything about it except when it was seen.
///
/// `serde_json`'s object is a `BTreeMap` here — `preserve_order` is off — so
/// this is canonical, and two rows written by different versions of the miner
/// with different key orders still land on the same string.
fn identity(row: &serde_json::Value) -> String {
    let mut bare = row.clone();
    if let Some(map) = bare.as_object_mut() {
        map.remove("at");
    }
    bare.to_string()
}

/// The subject the union exists to preserve: a command, and the directory that
/// gives its relative paths a meaning.
fn subject(row: &serde_json::Value) -> Option<String> {
    let cmd = row.get("cmd")?;
    let cwd = row.get("cwd")?;
    Some(serde_json::json!({ "cmd": cmd, "cwd": cwd }).to_string())
}

/// Merge `fresh` into `union`, collapsing each untimestamped row into the
/// timestamped twin that already says everything it says.
pub fn merge(union: &str, fresh: &str) -> Merge {
    let parse = |text: &str| -> Vec<(String, Option<serde_json::Value>)> {
        text.lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                let row = serde_json::from_str::<serde_json::Value>(line)
                    .ok()
                    .filter(serde_json::Value::is_object);
                (line.to_string(), row)
            })
            .collect()
    };

    let union = parse(union);
    let fresh = parse(fresh);

    let subjects_before: HashSet<String> = union
        .iter()
        .filter_map(|(_, row)| row.as_ref().and_then(subject))
        .collect();

    // Which identities are vouched for by a timestamp. Gathered over BOTH
    // inputs, so tonight's stamped row collapses a bare one the union has been
    // carrying for months.
    let stamped: HashSet<String> = union
        .iter()
        .chain(fresh.iter())
        .filter_map(|(_, row)| row.as_ref())
        .filter(|row| row.get("at").is_some())
        .map(identity)
        .collect();

    let mut rows = BTreeSet::new();
    let mut collapsed = 0;
    let mut unparsed = 0;
    let mut subjects_after = HashSet::new();

    for (line, row) in union.into_iter().chain(fresh) {
        let Some(row) = row else {
            unparsed += 1;
            rows.insert(line);
            continue;
        };
        if row.get("at").is_none() && stamped.contains(&identity(&row)) {
            collapsed += 1;
            continue;
        }
        if let Some(subject) = subject(&row) {
            subjects_after.insert(subject);
        }
        rows.insert(line);
    }

    Merge {
        rows: rows.into_iter().collect(),
        collapsed,
        subjects_before: subjects_before.len(),
        subjects_after: subjects_after.len(),
        unparsed,
    }
}
