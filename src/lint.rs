//! Static analysis for the memory corpus itself.
//!
//! The corpus is a document set with rules, and until now nothing checked them:
//! three links were written `[[name.md]]` and had been silently dead for weeks,
//! twelve memories had no links in either direction and could never surface
//! during work, and a misspelt relation would have joined them unnoticed. None
//! of that is visible by reading — each file is individually fine.
//!
//! Severity is deliberately two-tier and deliberately movable. A rule starts as
//! a WARNING while the existing violations are worked through, and is promoted
//! to an ERROR once the count reaches zero — so the corpus ratchets forward and
//! cannot regress on anything already fixed. Promoting a rule is a one-word
//! edit here; that is the whole design.
//!
//! Most of the rules are the corpus's own conventions, checked against itself:
//! lowercase filenames, a description on every memory, `**Why:**` on a feedback
//! memory. Those conventions are written down as memories, and a convention
//! nothing enforces is a convention that decays.

use std::collections::{BTreeMap, BTreeSet};

use crate::couse::CoUse;
use crate::store::{
    Corpus, MEMORY_TYPES, RELATIONS, has_section, index_links, reachable_without, split_relation,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Known violations exist; the rule is documented and being worked down.
    Warning,
    /// Zero violations remain, so any new one is a regression.
    Error,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Severity::Warning => "warning",
            Severity::Error => "error",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Finding {
    pub severity: Severity,
    /// Stable rule id, so a finding can be talked about without quoting it.
    pub rule: &'static str,
    /// The memory it is about — filename stem, or `MEMORY.md` for the index.
    pub memory: String,
    pub detail: String,
}

/// The most a single index teaser may take from the ceiling.
///
/// Not tuned to the longest line in the corpus — a cap that tight would argue
/// with wording. It sits between the length of a teaser and that of a
/// description, so what it catches is a description pasted into the wrong
/// field.
pub const TEASER_MAX: usize = 140;

/// The injection ceiling, owned by [`crate::ceiling`] and re-exported here.
///
/// ⚠ **One number, one owner.** The cut model and the rule that reports it have
/// to agree by construction, not by two constants that happen to match — a
/// corpus with two ceilings warns at one size and truncates at another.
pub use crate::ceiling::INDEX_CEILING;

/// Rules a corpus caught mid-write can fail through no fault of its own.
///
/// Writing a memory is two edits — the file, then its line in MEMORY.md — and
/// anything loading the corpus between them sees a memory that exists and is
/// not reachable. Both are ERRORS and the pre-commit gate runs this, so a
/// concurrent session writing a memory can fail an unrelated commit for a
/// reason that was never the committer's and that evaporates on retry.
///
/// ⚠ **Here rather than in `bin/memory-lint.rs`, because there it named a rule
/// that no longer existed.** A renamed rule leaves behind a string that still
/// compiles, still reads as deliberate, and matches nothing — so the retry
/// covered one of its two rules, and not the likelier one (memview#1456). Beside [`RULES`] it is one grep from its subject, and
/// `every_racy_rule_is_a_real_rule` in `tests/suite/lint.rs` now fails on a rename
/// instead of silently disarming the retry.
/// How much of a file the Read tool returns by default.
///
/// A memory past this still opens and still looks whole; only its tail is
/// missing, which is the one failure a reader cannot see in the output.
pub const READ_LIMIT: usize = 2000;

pub const RACY: [&str; 2] = ["unreachable", "index-points-nowhere"];

/// Every rule, with the severity it currently carries.
///
/// Listed in one place rather than beside each check so that the promotion of a
/// rule from warning to error is a visible, reviewable edit — and so this table
/// can be read as the answer to "what does a good memory look like".
const RULES: &[(&str, Severity, &str)] = &[
    (
        "teaser-shape",
        Severity::Error,
        "a `teaser:` is one short line for the index, not a second description — \
         one line, at most TEASER_MAX bytes",
    ),
    (
        // A WARNING although the corpus is at zero, which the two-tier design
        // would otherwise send straight to ERROR: the ratchet's condition is
        // "worked down to zero", and this reached zero in the very pass that
        // judged the backlog. A rule never observed stable should not decide
        // whether the other sessions can commit. Promote it once it has held —
        // the one-word edit the design is built around.
        "unjudged-role",
        Severity::Warning,
        "an indexed memory with no `role:` and no entry in the judgement record \
         — an unjudged memory is held from demotion forever, so the index can \
         only grow (memview#1537)",
    ),
    (
        // A WARNING while violations remain. ⚠ It may NEVER be promoted by
        // writing longer index lines: the index is near its size ceiling, and
        // stating a claim on every one of these costs far more than the headroom
        // left. Zero comes from demoting them or from the corpus getting
        // smaller, never from editing toward the rule (memview#822).
        "mute-tripwire",
        Severity::Warning,
        "a memory judged TRIPWIRE whose index line states no claim — it reminds \
         a reader who already knows it and warns nobody else",
    ),
    (
        "link-extension",
        Severity::Error,
        "a `[[name.md]]` wikilink can never resolve — the canonical id is the filename stem",
    ),
    (
        "unknown-relation",
        Severity::Error,
        "a typed link whose relation is not in the vocabulary; usually a typo",
    ),
    (
        "self-link",
        Severity::Error,
        "a memory linking to itself asserts nothing",
    ),
    (
        "name-mismatch",
        Severity::Error,
        "frontmatter `name` disagrees with the filename, which is the canonical id",
    ),
    (
        "uppercase-filename",
        Severity::Error,
        "memory filenames are lowercase",
    ),
    (
        "missing-description",
        Severity::Error,
        "the description is what recall reads to decide relevance; without one the memory is invisible",
    ),
    (
        // ⚠ **Presence is not accuracy, and this rule only checks presence.**
        // The stamp is maintained by the memory-writing path and goes silently
        // wrong whenever a file is edited by any other route.
        //
        // ⚠ **Do not rebuild this on mtime.** Most files disagree with their
        // own stamp by days: a synced, restored or merely touched copy carries
        // an mtime that says nothing about when the memory was written, and
        // `modified` is not even reliably later than it (#1219).
        //
        // ⚠ **The message names a repair tool; do NOT let that become an
        // auto-fix.** `memory-stamp` must stay a thing a person runs, because
        // this rule failing is the only visible symptom of a write that skipped
        // the stamping path, and the harder half is the AUTHOR — recoverable only
        // from the transcripts the archive still holds. Silence the rule and
        // the authorship loss continues unseen; see
        // `feedback_a_precondition_that_can_pass_wrongly`. What the pointer
        // removes is the forensics, not the failure: the session that trips this
        // is usually not the one that wrote the file.
        "missing-modified",
        Severity::Error,
        "no `modified` stamp — its age cannot be judged, so a stale claim reads as current; \
         `cargo run --bin memory-stamp` names the session that wrote it and repairs it",
    ),
    (
        // ⚠ **What this catches is a BACKFILL, not a typo.** A `created` stamp
        // written later than the file's own first commit invents a birthday the
        // memory never had. Nothing downstream reads the pair together, so one
        // sits unremarked: it still recalls, still renders, and quietly poisons
        // anything that later asks how old the corpus is.
        //
        // ⚠ **Ordering only.** Whether either stamp is TRUE is not decidable
        // here — see `missing-modified` above on why mtime cannot referee it.
        // This rule states the one thing that is true by construction: a memory
        // cannot have been changed before it existed.
        "created-after-modified",
        Severity::Error,
        "`created` is later than `modified` — a memory cannot have been changed before it existed, \
         so one of the two stamps was written by something that did not check",
    ),
    (
        // The stem is what everything resolves by, so a missing `name:` breaks
        // nothing at runtime — which is the reason to check it. It is the
        // memory's own statement of its id, and a reader quoting frontmatter
        // that is not there cites nothing.
        "missing-name",
        Severity::Error,
        "no `name:` in frontmatter — the memory does not state its own id",
    ),
    (
        // ⚠ Covers absent AND out-of-vocabulary in one rule, because they fail
        // identically downstream: `mtype` falls back to the filename prefix, so
        // `type: refrence` and no type at all both parse as a valid memory and
        // neither is visible by reading the file.
        "unknown-type",
        Severity::Error,
        "metadata `type` missing or outside the vocabulary; it silently falls back to the filename prefix",
    ),
    (
        "stranded",
        Severity::Error,
        "no links in either direction — can only be found by already knowing its name",
    ),
    (
        // **Reachability, not membership**, at Pippijn's word: *"MEMORY.md
        // doesn't need to index everything. things have to be reachable, but
        // don't need to all be in MEMORY.md"*. Demanding an index line for every
        // memory fails the gate on a corpus that is perfectly navigable —
        // several memories consolidated under one entry that links them all.
        // What
        // matters is that a reader starting at MEMORY.md can get there, by any
        // number of hops.
        "unreachable",
        Severity::Error,
        "nothing browses to it: no path of links from MEMORY.md reaches it",
    ),
    (
        "index-points-nowhere",
        Severity::Error,
        "MEMORY.md links a file that does not exist",
    ),
    (
        "index-over-ceiling",
        // ⚠ **A WARNING, and it must stay one until the corpus reaches zero.**
        // The root is over the line today, so shipping this at error would make
        // the nightly memory commit unpassable — `claude-sync` sets
        // `corpus_ok=false` on any lint error and withholds the whole corpus
        // from its history. A gate that can never go green is not a signal; it
        // is how memview's own gate became unpassable (#1062). Promote it in a
        // one-word edit once the root is under, and not before.
        //
        // ⚠ **Why this is a LINT and not a line in `MEMORY.md`'s header.** The
        // rule — a new line is paid for by demoting a finished one — has been
        // written in that header for weeks, injected into every session, and the
        // root has grown past the ceiling anyway. Prose asking a writer to
        // budget does not budget; it spends the scarcest bytes in the corpus
        // restating a constraint nothing enforces. This is the same constraint
        // where it can actually bind (#822).
        Severity::Warning,
        "MEMORY.md is past the injection ceiling — the bottom is silently truncated and no session can tell",
    ),
    (
        "dangling-link",
        // Never promoted, however low the count goes. The memory instructions
        // say a link to a memory that does not exist yet "marks something worth
        // writing later, not an error" — so this rule reports a backlog, and
        // making it fail would punish exactly the habit it is there to track.
        Severity::Warning,
        "links a memory that was never written — an intent marker, so this is a backlog and never an error",
    ),
    (
        // ⚠ **Match the shape, not the literal bytes.** An earlier version
        // demanded exactly `**Why:**`, so `**Why (the nixos-repo caution):**` —
        // better writing, and a scope the rule genuinely has — read as no reason
        // at all. Promoting a check that strict would make "phrase it exactly
        // this way" an error, and the corpus would be edited to satisfy a string
        // match.
        "missing-why",
        Severity::Error,
        "a feedback memory needs a bold **Why…** section — a rule without its reason gets misapplied",
    ),
    (
        "missing-how",
        Severity::Error,
        "a feedback memory needs a bold **How to apply…** section — a rule you cannot act on is a note",
    ),
    (
        // Armed at zero so it reports a REGRESSION rather than a backlog, which
        // is the corpus convention and the only moment it is cheap.
        //
        // ⚠ **WARNING, and it must NOT be promoted without somewhere to route
        // it.** A memory with no `originSessionId` cannot be attributed, so
        // [`passed_for_session`] can never charge it to anybody: at ERROR it
        // would fail the nightly and no session — precisely the #1047 pathology
        // that function exists to remove, arriving by a new door. `memory-blame`
        // cannot route it either, since it files to the author it does not have.
        // Catching this needs the WRITER at write time (#1498), not a louder
        // corpus rule.
        //
        // ⚠ **`missing-modified` does not cover this path.** These files HAVE a
        // stamp. A heredoc creates one with neither field; a later Edit stamps
        // `modified:` because the body changed, and never adds the origin
        // because an edit is not a creation. The stamp heals itself and the
        // author is lost for good.
        "missing-origin",
        Severity::Warning,
        "no `originSessionId:` — nobody can be asked about it, and a rule that FAILED on it would block the nightly and no session",
    ),
    (
        // Armed before the cliff rather than after one: past [`READ_LIMIT`] a
        // memory still opens and still looks whole, and its tail is silently not
        // there — the one failure a reader cannot detect from the output. The
        // corpus has hit it twice, and both times found out afterwards.
        "past-read-limit",
        Severity::Error,
        "past the Read tool's default line limit, so the tail is silently unread",
    ),
    (
        // The same cliff with room to act: half of [`READ_LIMIT`] is well past
        // the corpus's ordinary spread, so it marks a real outlier while leaving
        // enough headroom to split deliberately rather than in a panic.
        //
        // ⚠ **Every split so far has been reactive**, which is why the warning
        // exists at all: a rule that fires only once the tail is invisible
        // reports a loss instead of preventing one.
        "nearing-read-limit",
        Severity::Warning,
        "over half the Read tool's default line limit and growing — split it deliberately, before the tail goes quiet",
    ),
    (
        // ⚠ **The failure it exists for has happened.** A memory retracted a
        // figure and then went on quoting it throughout, including in its own
        // description. Its defence was a hand-written CORRECTION banner, applied
        // afterwards and checkable by nobody, because a retraction was prose.
        //
        // ⚠ **Linking the retraction is the whole requirement, and the reason is
        // that prose cannot be read.** A grep for the banner in a file that HAS
        // one returned nothing, because it reads "no 173/173 figure IS
        // comparable" rather than "not comparable" — negation split across a
        // sentence. `missing-why` made the same mistake with literal `**Why:**`
        // bytes and 9 of its 19 findings were the check rather than the corpus.
        // So this asks for a link, which is structure, and never for a phrasing.
        "quotes-a-retracted-figure",
        Severity::Error,
        "quotes a figure another memory declares retracted, without linking the memory that retracted it",
    ),
    (
        "unlinked-co-use",
        // Advisory, and never promoted. This is evidence, not a rule: two
        // memories used together may still have nothing to say about each
        // other, and a gate that forced a link for every correlation would fill
        // the corpus with links nobody meant.
        Severity::Warning,
        "used together in separate turns but neither links the other — a link the corpus is missing",
    ),
    (
        "dead-repo-path",
        Severity::Error,
        "names a `~/Code/<repo>` that does not exist, and does not say where it went",
    ),
    (
        "unresolvable-code-root",
        Severity::Error,
        "the checkout root could not be read, so no path claim was actually verified",
    ),
    (
        // A warning and not an error because the rule cannot separate "this sha
        // is wrong" from "the repo holding it is not cloned here", and both look
        // identical from the code root.
        //
        // ⚠ **Three filters, and each was measured rather than guessed.**
        // Checking every `[0-9a-f]{7,10}` token against the memory's OWN repo,
        // guessed from its name, reported 65 dead of 237 — and five of the first
        // six checked existed in a DIFFERENT repo. The memory does not say which
        // repo a sha belongs to and the name does not imply it, so the question
        // has to be asked of every repo, not one. That correction took the rate
        // from 27% to 7.6%. The remaining noise was two shapes that are not
        // commits at all: decimal numbers that happen to be valid hex (`1048575`,
        // `1234567`) and 8-character session-id prefixes (`296dae53`), which the
        // corpus writes in backticks the same way. Excluding both: 4.1%.
        "unresolvable-commit",
        Severity::Warning,
        "cites a commit hash that exists in no repository here — a mistyped sha, a rebased-away commit, or a repo that is not cloned on this machine",
    ),
    (
        // ⚠ **This is `created-after-modified`'s blind half.** That rule states
        // what is decidable inside the corpus — a memory cannot have been
        // changed before it existed — and it caught exactly one of the six,
        // only because that one's `modified` happened to land before its
        // invented `created`. The other five had both stamps in order and were
        // simply wrong about when they began.
        //
        // ⚠ **One-sided ON PURPOSE.** A `created` EARLIER than the mined first
        // write is expected and correct: the transcript archive does not reach
        // back forever, so a memory older than it shows its first RE-write
        // instead of its creation. Only the other direction is impossible.
        "created-after-first-write",
        Severity::Error,
        "`created` is later than the earliest write the transcripts record — a birthday typed \
         rather than looked up; `cargo run --bin memory-created` names the real one",
    ),
    (
        // The `unresolvable-code-root` argument, one artefact over: this rule's
        // whole input is mined by a separate run, so "no findings" and "never
        // ran" are the same output unless one of them says so. WARNING and not
        // ERROR because the record is absent on every machine but the Mac, and
        // a lint that refuses a fresh checkout is a lint nobody runs.
        "unreadable-created-record",
        Severity::Warning,
        "the mined creation record could not be read, so birthdays went unchecked this run — \
         `cargo run --bin memory-created` rebuilds it",
    ),
];

fn severity_of(rule: &str) -> Severity {
    RULES
        .iter()
        .find(|(id, _, _)| *id == rule)
        .map(|(_, sev, _)| *sev)
        .unwrap_or(Severity::Warning)
}

/// What each rule is for, keyed by id — printed alongside a run's findings so
/// the output explains itself rather than needing this file open beside it.
pub fn rule_reasons() -> BTreeMap<&'static str, (Severity, &'static str)> {
    RULES
        .iter()
        .map(|(id, sev, why)| (*id, (*sev, *why)))
        .collect()
}

/// Run every rule over the corpus.
///
/// Findings come back sorted by severity then memory, so the output reads as a
/// worklist and a run with nothing to say prints nothing.
/// `roles` is `memory-roles.json`, or `None` where the caller has no reason to
/// hold it — the `unjudged-role` rule is skipped then rather than reporting a
/// gap it cannot see. Passing it is what lets the rule tell an unjudged memory
/// from one judged in the record but not in its own frontmatter.
/// Whether an index line asserts something a reader could be wrong about.
///
/// ⚠ **Deliberately crude, and a FLOOR rather than a judgement.** Emphasis or a
/// clause of four words or more; anything shorter is a bare label. It cannot
/// tell a good claim from a bad one and does not try — what it separates is
/// `cd` and `TDD` from a sentence, which is the distinction that decides
/// whether a line can act on anybody at all.
///
/// ⚠ **A bare label is not USELESS**, and the rule's wording says so. `TDD`
/// recalls the rule to a reader who has already read the file: it is a
/// mnemonic. What it cannot do is warn a reader who has not, which is the one
/// thing a tripwire exists for.
fn states_a_claim(label: &str) -> bool {
    label.contains("**") || label.split_whitespace().count() >= 4
}

pub fn check(
    corpus: &Corpus,
    couse: Option<&CoUse>,
    roles: Option<&serde_json::Value>,
) -> Vec<Finding> {
    let mut findings = Vec::new();
    let mut push = |rule: &'static str, memory: &str, detail: String| {
        findings.push(Finding {
            severity: severity_of(rule),
            rule,
            memory: memory.to_string(),
            detail,
        });
    };

    // ⚠ **A bound, not a style rule.** Every teaser is copied into an injected
    // file with a hard 24,400-byte ceiling, so length here is spent from a fixed
    // budget rather than from taste. The longest line in the index today is 123
    // bytes and the median is 8; a description's median is 193, so the cap is
    // set to catch a description pasted into the wrong field rather than to
    // argue with anyone's phrasing.
    for doc in corpus.docs.values() {
        let Some(teaser) = doc.meta.teaser.as_deref() else {
            continue;
        };
        if teaser.contains('\n') {
            push(
                "teaser-shape",
                &doc.meta.name,
                "spans more than one line; it becomes a single line of the index".to_string(),
            );
        } else if teaser.len() > TEASER_MAX {
            push(
                "teaser-shape",
                &doc.meta.name,
                format!(
                    "{} bytes, over the {TEASER_MAX} allowed — that is description length, and \
                     the index has {} bytes for every entry it carries",
                    teaser.len(),
                    crate::ceiling::INDEX_CEILING
                ),
            );
        }
    }

    let mut linked: BTreeSet<String> = BTreeSet::new();
    // Unordered pairs, for the co-use comparison: a link in either direction
    // means the two memories already know about each other.
    let mut pairs: BTreeSet<(String, String)> = BTreeSet::new();
    // Every resolved outbound target, per memory, and every governs-edge — both
    // collected here because the reciprocity question can only be asked once the
    // whole corpus has been walked.
    let mut outbound: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut governs: Vec<(String, String)> = Vec::new();
    let mut part_of: Vec<(String, String)> = Vec::new();

    for (name, doc) in &corpus.docs {
        if name.chars().any(char::is_uppercase) {
            push("uppercase-filename", name, format!("{name}.md"));
        }
        if doc.meta.description.trim().is_empty() {
            push("missing-description", name, "no description".to_string());
        }
        if frontmatter_value(&doc.raw, "modified").is_none() {
            push(
                "missing-modified",
                name,
                "no `modified:` in frontmatter".to_string(),
            );
        }
        // ⚠ **Compared as instants, not as the text they were written in.** A
        // rule on the strings would read `2026-08-19T09:00:00Z` as later than
        // `2026-08-21T08:00:00+03:00`, and both shapes are in the corpus.
        if let (Some(created), Some(modified)) = (doc.meta.created, doc.meta.modified)
            && created > modified
        {
            push(
                "created-after-modified",
                name,
                format!(
                    "created {} is after modified {}",
                    created.to_rfc3339(),
                    modified.to_rfc3339()
                ),
            );
        }
        if frontmatter_value(&doc.raw, "name").is_none() {
            push(
                "missing-name",
                name,
                "no `name:` in frontmatter".to_string(),
            );
        }
        match frontmatter_value(&doc.raw, "type") {
            None => push("unknown-type", name, "no `type:` in metadata".to_string()),
            Some(t) if !MEMORY_TYPES.contains(&t.as_str()) => push(
                "unknown-type",
                name,
                format!("`type: {t}` is not one of {}", MEMORY_TYPES.join(" | ")),
            ),
            Some(_) => {}
        }
        // The frontmatter `name` is not trusted for lookup — the stem is — but a
        // disagreement means one of the two is wrong, and a reader quoting the
        // frontmatter would cite an id nothing resolves.
        if let Some(declared) = frontmatter_name(&doc.raw)
            && declared != *name
        {
            push(
                "name-mismatch",
                name,
                format!("frontmatter says `{declared}`"),
            );
        }

        // Counted on `raw`, because the Read tool's limit applies to the file
        // on disk — frontmatter included — and not to the parsed body.
        let lines = doc.raw.lines().count();
        if lines > READ_LIMIT {
            push("past-read-limit", name, format!("{lines} lines"));
        } else if lines > READ_LIMIT / 2 {
            push(
                "nearing-read-limit",
                name,
                format!("{lines} lines, {} from the limit", READ_LIMIT - lines),
            );
        }

        // Every type, not just feedback: authorship is who to ask, and that
        // question is asked of a `reference` as often as of a rule.
        if frontmatter_value(&doc.raw, "originSessionId").is_none() {
            push(
                "missing-origin",
                name,
                "no `originSessionId:` — `memory-stamp` recovers it from the transcripts"
                    .to_string(),
            );
        }

        if doc.meta.mtype == "feedback" {
            if !has_section(&doc.body, "Why") {
                push("missing-why", name, "says no why".to_string());
            }
            if !has_section(&doc.body, "How to apply") {
                push("missing-how", name, "nothing to act on".to_string());
            }
        }

        for link in &doc.links {
            if link.target == *name {
                push("self-link", name, format!("[[{}]]", link.target));
                continue;
            }
            if link.target.ends_with(".md") {
                push("link-extension", name, format!("[[{}]]", link.target));
                continue;
            }
            // A colon-prefixed target that split_relation refused is either a
            // misspelt relation or an invented one. Either way the link points
            // at a memory that cannot exist, so say which of the two it is
            // rather than reporting it as merely dangling.
            if let Some((prefix, rest)) = link.target.split_once(':')
                && !rest.is_empty()
                && prefix.chars().all(|c| c.is_ascii_lowercase() || c == '-')
            {
                push(
                    "unknown-relation",
                    name,
                    format!("`{prefix}:` is not one of {}", RELATIONS.join(", ")),
                );
                continue;
            }
            if corpus.docs.contains_key(&link.target) {
                outbound
                    .entry(name.clone())
                    .or_default()
                    .insert(link.target.clone());
                match link.relation.as_deref() {
                    Some("governs") => governs.push((name.clone(), link.target.clone())),
                    Some("part-of") => part_of.push((name.clone(), link.target.clone())),
                    _ => {}
                }
                linked.insert(link.target.clone());
                linked.insert(name.clone());
                let (x, y) = if *name < link.target {
                    (name.clone(), link.target.clone())
                } else {
                    (link.target.clone(), name.clone())
                };
                pairs.insert((x, y));
            } else {
                push("dangling-link", name, format!("[[{}]]", link.target));
            }
        }
    }

    for name in corpus.docs.keys() {
        if !linked.contains(name) {
            push("stranded", name, "no inbound or outbound links".to_string());
        }
    }

    if let Some(index) = corpus.index_md.as_deref() {
        let targets = index_links(index);
        for target in &targets {
            if !corpus.docs.contains_key(target) {
                push("index-points-nowhere", "MEMORY.md", format!("{target}.md"));
            }
        }
        // ⚠ **Only the INDEXED set, because only it can reach a proposal.**
        // `memory-tiers` and `memory-rank` both hold an unjudged memory — an
        // absent judgement is not a pointer — so an unjudged INDEXED entry is
        // exempt from demotion forever and the root can only grow. An unjudged
        // unindexed one costs nothing and is not this rule's business.
        if let Some(roles) = roles {
            // One parsed reading of the file — see `store::index_entries`.
            for entry in crate::store::index_entries(index) {
                let Some(doc) = corpus.docs.get(&entry.name) else {
                    continue; // already reported as index-points-nowhere
                };
                let Some(role) =
                    crate::study::role_for(doc.meta.role.as_deref(), roles, &entry.name)
                else {
                    push(
                        "unjudged-role",
                        &entry.name,
                        "no `role: tripwire|pointer` in its frontmatter and none in the record"
                            .to_string(),
                    );
                    continue;
                };
                // ⚠ **Only a TRIPWIRE, because a bare label is CORRECT for a
                // pointer.** Most indexed pointers state no claim, and that is
                // the control: a tripwire's whole job is to act on a reader who
                // did not come looking, so silence means something there and
                // nothing on a pointer.
                if role == crate::study::Role::Tripwire && !states_a_claim(&entry.label) {
                    push(
                        "mute-tripwire",
                        &entry.name,
                        format!("line reads {:?}", entry.label),
                    );
                }
            }
        }
        // Walk out from the index through the wikilinks, as a reader would —
        // with nothing struck out, which is the same question `memory-rank` asks
        // with its demotions struck out. One invariant, one implementation: the
        // second copy of this walk is what let that tool grow a one-at-a-time
        // signature and recommend a set that stranded a pair (#869).
        // ⚠ **Bytes, not entries, and measured on the file as injected.** The
        // truncation is a byte limit, so an index of few long lines fails where
        // one of many short lines passes. `crate::ceiling` owns both the number
        // and the cut model, and records how the number was measured.
        let size = index.len();
        let seen = crate::ceiling::cut(index, INDEX_CEILING);
        if !seen.is_whole() {
            // ⚠ **Name the casualties, do not state an overage.** "969 over" is
            // a number a reader can carry for weeks without acting; it says
            // nothing about which memories stopped arriving, and the file gives
            // no other sign — a truncated index reads as a complete one. The
            // whole point of the rule is that no session can tell, so the lint
            // has to be the thing that tells.
            // ⚠ **Order-preserving, and NOT `Vec::dedup`** — that drops only
            // ADJACENT repeats, so one name listed in two sections would be
            // counted twice and the tally would overstate the loss.
            let mut seen_once = std::collections::BTreeSet::new();
            let lost: Vec<String> = crate::store::index_links(seen.dropped)
                .into_iter()
                .filter(|name| seen_once.insert(name.clone()))
                .collect();
            push(
                "index-over-ceiling",
                "MEMORY.md",
                format!(
                    "{size} bytes, {} over the {INDEX_CEILING} b ceiling — {} memories a new session is NOT given",
                    size - INDEX_CEILING,
                    lost.len()
                ),
            );
            // ⚠ **These are the casualties, not an estimate.** [`INDEX_CEILING`]
            // is the edge rather than a warning line, so every name here is a
            // memory a new session is not given. The one remaining slack is the
            // cut
            // model: whole lines, which can over-report by at most one partial
            // line at the boundary.
            for name in lost {
                push(
                    "index-over-ceiling",
                    "MEMORY.md",
                    format!("  below the line: {name}"),
                );
            }
        }
        let reached = reachable_without(&corpus.docs, index, &BTreeSet::new());
        for name in corpus.docs.keys() {
            if !reached.contains(name) {
                push(
                    "unreachable",
                    name,
                    "no path of links from MEMORY.md".to_string(),
                );
            }
        }
    }

    // A figure somebody retracted, still being quoted by a memory that does not
    // link the retraction. Asked after the whole corpus is walked, because the
    // declaration can live in any memory and be quoted by any other.
    let mut retracted: Vec<(String, String)> = Vec::new();
    for (name, doc) in &corpus.docs {
        for figure in retracted_figures(&doc.raw) {
            retracted.push((figure, name.clone()));
        }
    }
    for (figure, declarer) in &retracted {
        for (name, doc) in &corpus.docs {
            if name == declarer || !doc.body.contains(figure.as_str()) {
                continue;
            }
            // The link is the requirement. A reader who lands on the figure is
            // one hop from what retracts it, whatever words surround it.
            if outbound
                .get(name)
                .is_some_and(|out| out.contains(declarer.as_str()))
            {
                continue;
            }
            push(
                "quotes-a-retracted-figure",
                name,
                format!("quotes `{figure}` — retracted by {declarer}, which it does not link"),
            );
        }
    }

    // What the transcripts say belongs together and the corpus does not.
    if let Some(couse) = couse {
        // Undirected, and built from the linked pairs rather than `outbound`, so
        // "some memory links both of these" is asked in the direction a reader
        // actually travels — a backlink walks as well as a link.
        let mut adjacency: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for (a, b) in &pairs {
            adjacency.entry(a.clone()).or_default().insert(b.clone());
            adjacency.entry(b.clone()).or_default().insert(a.clone());
        }
        let (missing, connected) = couse.unlinked(&adjacency);
        // Capped, and the cap is reported rather than silently applied: 500
        // suggestions is a wall, not a worklist, and a list that quietly stops
        // reads as "that is all of them".
        const SHOWN: usize = 20;
        for pair in missing.iter().take(SHOWN) {
            push(
                "unlinked-co-use",
                &pair.a,
                format!(
                    "{} with {} ({} sessions, {} turns, npmi {:.2})",
                    "used together", pair.b, pair.sessions, pair.turns, pair.npmi
                ),
            );
        }
        if missing.len() > SHOWN {
            push(
                "unlinked-co-use",
                "(corpus)",
                format!(
                    "{} more unlinked pairs seen in >= {} sessions; showing the {SHOWN} strongest \
                     ({connected} further pairs are held back: some memory already links both)",
                    missing.len() - SHOWN,
                    crate::couse::MIN_SESSIONS
                ),
            );
        }
    }

    findings.sort_by(|a, b| {
        b.severity
            .cmp(&a.severity)
            .then(a.rule.cmp(b.rule))
            .then(a.memory.cmp(&b.memory))
    });
    findings
}

/// Repo names that legitimately appear under `~/Code/` without being checkouts.
///
/// Enumerated rather than pattern-matched on purpose: an explicit list is
/// auditable and fails safe — a missing entry is a visible finding, where a
/// clever rule would silently swallow the case it did not anticipate.
const NOT_A_REPO: &[&str] = &[
    // The fleet-consistency conductor: a bare script, not a checkout.
    "check",
];

/// Words that name an 8-hex identifier as something other than a commit.
///
/// Read in the [`KIND_WINDOW`] characters immediately before the opening
/// backtick, because that is where the writer says what the token is. Each of
/// these is in the corpus for a real reason: `snapshot` and `restic` for the
/// archive's own ids, `magic` and `bytecode` for the Hermes header, `blob` and
/// `hash-object` for a git object that is not a commit.
///
/// Enumerated for the same reason [`NOT_A_REPO`] is — a missing entry shows up
/// as a visible finding, where a pattern would swallow whatever it did not
/// anticipate.
///
/// ⚠ **`session` is deliberately NOT here.** Session ids are already excluded by
/// `originSessionId`, which is exact where a word is a guess.
const NOT_A_COMMIT_KIND: &[&str] = &[
    "restic",
    "snapshot",
    "magic",
    "bytecode",
    "hash-object",
    "blob",
    "checksum",
    "digest",
    "inode",
];

/// How far back of the text before a token is read for [`NOT_A_COMMIT_KIND`].
///
/// 20 characters, measured against the corpus — see [`commit_shas`] for the
/// cost of every wider window that was tried.
const KIND_WINDOW: usize = 20;

/// The text of a memory a path claim can live in — prose, inline code and
/// fenced blocks — with link destinations left out.
///
/// Parsed with comrak, like everything else that reads corpus markdown. A raw
/// substring scan is what this replaced, and the reason not to keep one is
/// written into `store.rs::index_sections`: the line scanner there mis-read any
/// link whose title contained `](`. The same hazard applies here — a URL that
/// happens to contain the root's path would read as a claim about a repo.
///
/// Code spans and fenced blocks are deliberately KEPT, which is the opposite
/// call from the index parser. There a link inside a fence was noise; here a
/// command in a fence is the most actionable kind of claim a memory can make —
/// `run ~/Code/x/deploy.sh` is an instruction whether or not it is fenced.
/// Returned one entry per TOP-LEVEL block, because the archive exemption is
/// scoped to the block a claim is written in — see [`check_world`]. Flattening
/// the document to a single string is what let one retirement banner clear every
/// stale path below it.
fn claimable_blocks(doc: &crate::store::MemoryDoc) -> Vec<String> {
    let options = crate::store::markdown_options();
    let arena = comrak::Arena::new();
    let root = comrak::parse_document(&arena, &doc.body, &options);
    let mut blocks: Vec<String> = Vec::new();
    for block in root.children() {
        let mut out = String::new();
        for node in block.descendants() {
            match &node.data.borrow().value {
                comrak::nodes::NodeValue::Text(t) => out.push_str(t),
                comrak::nodes::NodeValue::Code(c) => out.push_str(&c.literal),
                comrak::nodes::NodeValue::CodeBlock(c) => out.push_str(&c.literal),
                _ => {}
            }
            out.push('\n');
        }
        blocks.push(out);
    }
    // The description is frontmatter, so no markdown node covers it — and it is
    // load-bearing: `project_lares_recon` named the dead path there as well as in
    // its body, and the description is the half a reader sees first. It is its own
    // block: a retirement recorded in the body does not reach the line a reader
    // sees first, and vice versa.
    blocks.push(doc.meta.description.clone());
    blocks
}

/// Every `~/Code/<segment>` a memory names.
///
/// Memories write the checkout root both ways — `~/Code/x` and the absolute
/// `/path/to/Code/x` — and they mean the same place, so both are read. The
/// absolute form is derived from `code_root` rather than written down, so no
/// personal home path is baked into the source and a different root checks
/// correctly instead of silently matching nothing.
///
/// Only the first segment is taken: a repo either exists or it does not, whereas
/// a file inside one moves for ordinary reasons and checking those would report
/// churn as rot.
fn code_repos_named(text: &str, code_root: &std::path::Path) -> BTreeSet<String> {
    let absolute = format!("{}/", code_root.display());
    let prefixes = ["~/Code/", absolute.as_str()];
    let mut found = BTreeSet::new();
    for prefix in prefixes {
        let mut rest = text;
        while let Some(at) = rest.find(prefix) {
            rest = &rest[at + prefix.len()..];
            let seg: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
                .collect();
            let seg = seg.trim_end_matches('.').to_string();
            if !seg.is_empty() && !NOT_A_REPO.contains(&seg.as_str()) {
                found.insert(seg);
            }
        }
    }
    found
}

/// Checks that reach outside the corpus, to the checkout root the memories describe.
///
/// Kept separate from [`check`] because that function is pure over the corpus and
/// worth keeping that way; this one is the only part that touches a filesystem.
///
/// **Why this exists.** A repo retired to `~/Archive` was recorded in exactly one
/// memory while many others — including feedback rules, the highest-authority
/// documents here — still sent a reader to `~/Code`. Every rule in the table
/// above passed the whole time,
/// because all of them ask whether the document graph is well-formed and none of
/// them ask whether it is true. A corpus can be perfectly consistent with itself
/// and still be describing a machine that no longer exists.
///
/// The escape hatch is naming the new location: a claim that says `~/Archive/<repo>`
/// has recorded the retirement and is exempt. That makes the fix for a true positive
/// either update the path or state where it went — never just silence the check.
///
/// **The exemption is scoped to the BLOCK, not the document, and that distinction
/// is the whole rule.** Scoped per document it under-fires exactly where it matters:
/// a long project memory opens with "retired to `~/Archive/lares`" and forty lines
/// later still instructs "captures live at `~/Code/lares/captures`". Same repo, so
/// the per-repo check cannot separate them — the banner clears the file. That has
/// happened: a banner in the first paragraph and live paths below it, with this
/// rule silent on every one while the audit that found them read by hand.
/// A retirement note
/// records the retirement where it is written; it is not a document-wide waiver.
/// Sha-shaped tokens a memory writes in backticks, minus the two shapes that
/// look identical and are not commits.
///
/// ⚠ **A decimal number is valid hex.** `1048575` and `1234567` are both written
/// in backticks in this corpus and neither is a commit, so a token with no
/// `a`-`f` in it is not treated as one. Costs the rare all-digit sha, which is a
/// 1-in-16^7 shape and worth losing.
///
/// ⚠ **A session id is written the same way.** `296dae53` is the health
/// session's, not a commit, and the corpus cites session-id prefixes in prose. So
/// every `originSessionId` the corpus declares is excluded by its first eight
/// characters. Together these two filters took the finding rate from 7.6% to
/// 4.1%.
///
/// ⚠ **A third shape: an 8-hex identifier that is not a git object at all.** A
/// restic snapshot id, a Hermes bytecode magic number and a `git hash-object`
/// blob are all written in backticks exactly like a sha, and all three sat in
/// the findings as permanent noise — a warning that cannot be made true is one
/// a reader learns to skip. The discriminator is the word the writer put
/// IMMEDIATELY before the token, because that is where the kind is named:
/// `snapshot \`7d747cd5\``, `(magic \`c61fbc03\`)`.
///
/// ⚠ **The window is 20 characters and that number was measured, not chosen.**
/// Against the whole corpus — 440 citations that resolve as real commits, 27
/// that do not — a 20-char left window excuses 0 of the 440. Widening it costs
/// real detection immediately: 30 chars loses 5, 40 loses 6, 80 loses 14. The
/// enclosing BLOCK, which is the scope `dead-repo-path` uses, loses 43 — one
/// `restic` in a paragraph excuses every sha in it.
///
/// ⚠ **The inverse rule was measured and REJECTED.** #1249 proposed requiring a
/// nearby word claiming the token IS a commit. Only 51.6% of the 440 real
/// citations name one in their block, so it would have halved detection while
/// still keeping 8 of the 27 non-commits. Do not re-propose it.
fn commit_shas(body: &str, session_prefixes: &BTreeSet<String>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let bytes: Vec<char> = body.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != '`' {
            i += 1;
            continue;
        }
        let start = i + 1;
        let mut j = start;
        while j < bytes.len() && bytes[j] != '`' && bytes[j] != '\n' {
            j += 1;
        }
        if j < bytes.len() && bytes[j] == '`' {
            let tok: String = bytes[start..j].iter().collect();
            let hexish = (7..=10).contains(&tok.len())
                && tok
                    .chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
                && tok.chars().any(|c| c.is_ascii_lowercase());
            let named_kind = {
                let from = start.saturating_sub(1 + KIND_WINDOW);
                let before: String = bytes[from..start.saturating_sub(1)]
                    .iter()
                    .collect::<String>()
                    .to_ascii_lowercase();
                NOT_A_COMMIT_KIND.iter().any(|w| before.contains(w))
            };
            if hexish && !session_prefixes.contains(&tok) && !named_kind {
                out.insert(tok);
            }
            i = j + 1;
        } else {
            i = j;
        }
    }
    out
}

/// Whether a block records `repo` as living on a remote it names.
///
/// The counterpart of the `~/Archive/<repo>` exemption in [`check_world`]: both
/// ask "does this block account for the absence", and both answer it from what
/// the writer actually wrote rather than from a list kept somewhere else.
///
/// Matched as the LAST segment after an org, so `github.com/xinutec/phonos`
/// excuses `phonos` and not `xinutec` — an org name that collided with a repo
/// name would otherwise waive a real finding. Both spellings, because the corpus
/// writes browse URLs and clone URLs alike.
fn names_a_remote(block: &str, repo: &str) -> bool {
    for host in ["github.com/", "github.com:"] {
        let mut rest = block;
        while let Some(at) = rest.find(host) {
            rest = &rest[at + host.len()..];
            let mut segs =
                rest.split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')));
            let org = segs.next().unwrap_or("");
            // The separator that ended the org has to be a path separator; a
            // bare `github.com/xinutec` names an org and locates no repository.
            let after = &rest[org.len()..];
            if !org.is_empty() && after.starts_with('/') {
                let name: String = after[1..]
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
                    .collect();
                if name.trim_end_matches(".git") == repo {
                    return true;
                }
            }
        }
    }
    false
}

/// Every git repository directly under the code root, plus the root itself, plus
/// the archive beside it.
///
/// The root is included because `~/Code` is a repository too, and leaving it out
/// reported its own HEAD as unresolvable — measured, on `4a10271`.
///
/// ⚠ **A retired repository still holds its commits, and `dead-repo-path`
/// already says so.** That rule accepts `~/Archive/<repo>` as the retirement
/// record, so a memory may legitimately cite a sha from a repo that has left
/// `~/Code`. Searching only the code root reported two of those as unresolvable
/// — `lares` and `scanner-frozen` — which is the rule contradicting its
/// neighbour about where a retired repo lives.
///
/// The archive is found as a SIBLING of the code root rather than hard-coded, so
/// a test pointing at a temporary root does not reach the real `~/Archive` and a
/// root with no sibling archive simply finds nothing.
fn repos_under(code_root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut repos = vec![code_root.to_path_buf()];
    let mut collect = |dir: std::path::PathBuf| {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for e in entries.flatten() {
                let p = e.path();
                // `.git` for a working clone; a bare mirror is named `<name>.git`.
                if p.join(".git").exists()
                    || p.extension().is_some_and(|x| x == "git") && p.is_dir()
                {
                    repos.push(p);
                }
            }
        }
    };
    collect(code_root.to_path_buf());
    if let Some(parent) = code_root.parent() {
        collect(parent.join("Archive"));
        // ⚠ **Not every repository the fleet uses lives under the code root.**
        // `~/.config/home-manager` is one, cited by `project_mac_home_manager`
        // and `reference_mac_agents_run_from_store` among others, and searching
        // only `~/Code` reported five of its commits as existing nowhere — which
        // the rule's own text would have read as "not cloned on this machine"
        // about a repo that is right there.
        //
        // The whole directory rather than that one name: it is the same question
        // for whatever else is checked out beside it, and a hard-coded repo name
        // is a maintenance trap. Relative to the root's parent, like the archive
        // above, so a test root reaches nothing real.
        collect(parent.join(".config"));
        // ⚠ **`~/.claude` IS a repository, not a directory holding some** — which
        // is why `collect` cannot reach it: that walks a directory's children.
        //
        // Without it, every `unresolvable-commit` warning about a claude-config
        // commit is wrong, and the rule's own text accuses a "mistyped sha" or
        // "a repo that is not cloned on this machine" about the repository the
        // corpus itself lives in — the shape that teaches a reader to skim it.
        //
        // Same defect as `~/.config` above, recorded separately because the SHAPE
        // differs: anyone adding a third location has to know which kind it is.
        let claude = parent.join(".claude");
        if claude.join(".git").exists() {
            repos.push(claude);
        }
    }
    repos
}

/// Which of `shas` no repository holds, asked one repo at a time.
///
/// ⚠ **`--batch-check` echoes the RESOLVED oid for a hit, not the input**, so a
/// hit cannot be matched back to the short sha that produced it by reading the
/// line. It answers one line per input in input ORDER, which is what this pairs
/// on — and if the counts ever disagree the repo is skipped rather than paired
/// wrongly. Getting this wrong reported all 394 tokens as dead, including ones
/// verified by hand a minute earlier.
fn unresolved_in_any(shas: &BTreeSet<String>, repos: &[std::path::PathBuf]) -> BTreeSet<String> {
    let mut left: Vec<String> = shas.iter().cloned().collect();
    for repo in repos {
        if left.is_empty() {
            break;
        }
        let input = left
            .iter()
            .map(|s| format!("{s}^{{commit}}"))
            .collect::<Vec<_>>()
            .join("\n");
        let Ok(out) = git_batch_check(repo, &input) else {
            continue;
        };
        let lines: Vec<&str> = out.lines().collect();
        if lines.len() != left.len() {
            continue;
        }
        left = left
            .iter()
            .zip(lines)
            .filter(|(_, l)| l.contains("missing") || l.contains("ambiguous"))
            .map(|(s, _)| s.clone())
            .collect();
    }
    // ⚠ **Everything above asked `^{commit}`, so a token that IS a git object of
    // some other type is still sitting in `left`.** `reference_a_git_hook_...`
    // cites `c1b0730e`, which is `printf 'x' | git hash-object` — a real blob in
    // a real repo, correct as written, and reported for a year as a sha that
    // exists nowhere. Git already knows the answer; the rule just never asked.
    //
    // Asked bare and only of the leftovers, which is a handful rather than the
    // whole corpus. A token resolving to a blob, tree or tag is not a mistyped
    // commit, so it is dropped rather than re-worded into a second finding: the
    // memory is right and there is nothing for a reader to do.
    for repo in repos {
        if left.is_empty() {
            break;
        }
        let input = left.join("\n");
        let Ok(out) = git_batch_check(repo, &input) else {
            continue;
        };
        let lines: Vec<&str> = out.lines().collect();
        if lines.len() != left.len() {
            continue;
        }
        left = left
            .iter()
            .zip(lines)
            .filter(|(_, l)| l.contains("missing") || l.contains("ambiguous"))
            .map(|(s, _)| s.clone())
            .collect();
    }
    left.into_iter().collect()
}

/// ⚠ **`-C` sets the DIRECTORY and loses to `GIT_DIR`, which wins.** This lint
/// runs inside the gate, the gate runs from `git commit`'s pre-commit hook, and
/// that hook exports `GIT_DIR` and `GIT_INDEX_FILE` to everything it spawns. Left
/// inherited, every `cat-file` below would ask the COMMITTING repository whether
/// it holds the sha instead of asking the repo named by `-C` — so the rule would
/// answer wrongly in the one place it actually runs, and correctly by hand.
///
/// Found because the same inheritance made the test helper write into memview's
/// index; that cost two failed commits (`error: Error building trees`). The
/// production path had the identical bug one function away.
///
/// ⚠ **Strip by PREFIX, never by a list.** An enumerated set silently misses
/// whatever git adds next; `src/commits.rs` carries the same guard and the same
/// reason, after a subset that missed one variable let a fresh repo bind to the
/// committing repo's dirs.
///
/// ⚠ **No live defect was demonstrated, and that is stated rather than implied.**
/// Every off-list variable tried against this exact call resolved the sha
/// correctly — `GIT_CONFIG_COUNT`/`KEY`/`VALUE`, `GIT_CONFIG_GLOBAL`,
/// `GIT_CONFIG_SYSTEM`, `GIT_NAMESPACE`, `GIT_LITERAL_PATHSPECS`. Object lookup
/// turns on `GIT_DIR`, `GIT_OBJECT_DIRECTORY` and `GIT_ALTERNATE_OBJECT_DIRECTORIES`,
/// and the old list did name all three. So this is drift-proofing and consistency
/// between two guards, NOT a bug that was silently answering from the wrong
/// repository — do not cite it as one.
fn git_batch_check(repo: &std::path::Path, input: &str) -> std::io::Result<String> {
    use std::io::Write;
    let mut cmd = std::process::Command::new("git");
    cmd.arg("-C").arg(repo).args(["cat-file", "--batch-check"]);
    for (key, _) in std::env::vars() {
        if key.starts_with("GIT_") {
            cmd.env_remove(key);
        }
    }
    let mut child = cmd
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    child
        .stdin
        .take()
        .expect("piped")
        .write_all(input.as_bytes())?;
    let out = child.wait_with_output()?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn check_world(corpus: &Corpus, code_root: &std::path::Path) -> Vec<Finding> {
    let mut findings = Vec::new();

    // A root that cannot be read must report that, not pass. A check that answers
    // "no findings" because it could not look is worse than no check: it reads as
    // a clean bill and there is nothing in the output to say otherwise.
    if !code_root.is_dir() {
        findings.push(Finding {
            severity: severity_of("unresolvable-code-root"),
            rule: "unresolvable-code-root",
            memory: "(corpus)".to_string(),
            detail: format!("{} is not a readable directory", code_root.display()),
        });
        return findings;
    }

    for (name, doc) in &corpus.docs {
        // Reported once per (memory, repo) however many blocks name it: the
        // finding is "this memory sends a reader to a repo that is gone", and one
        // line per stale mention would bury that under repetition.
        let mut reported: BTreeSet<String> = BTreeSet::new();
        for block in claimable_blocks(doc) {
            for repo in code_repos_named(&block, code_root) {
                if code_root.join(&repo).exists() || reported.contains(&repo) {
                    continue;
                }
                // Naming the archive location IS the retirement record. Checked
                // per repo so a memory that retires one repo cannot excuse a
                // stale reference to another, and per BLOCK so a banner at the
                // top cannot excuse an instruction further down.
                if block.contains(&format!("~/Archive/{repo}")) {
                    continue;
                }
                // ⚠ **The third state: alive, pushed, and simply not cloned
                // here.** Retirement is not the only honest reason a `~/Code`
                // path is absent: a repo can be pushed with no working copy on
                // this Mac, and the honest sentence — "there is no clone here,
                // it is at github.com/xinutec/<repo>" — trips this rule for
                // saying where the repo was expected to be.
                //
                // Deleting the path from the prose is strictly worse: the reader
                // loses the location and the rule learns nothing. Naming the
                // remote beside
                // the path is the same shape of record as naming the archive,
                // and is accepted on the same terms — per repo, and per BLOCK,
                // so a header cannot excuse an instruction forty lines down.
                if names_a_remote(&block, &repo) {
                    continue;
                }
                reported.insert(repo.clone());
                findings.push(Finding {
                    severity: severity_of("dead-repo-path"),
                    rule: "dead-repo-path",
                    memory: name.clone(),
                    detail: format!("~/Code/{repo} does not exist"),
                });
            }
        }
    }

    // Commit claims, asked of every repo at once rather than per memory: one
    // `cat-file` per repository answers the whole corpus, where a call per
    // citation would be ~370 spawns in a pre-commit gate.
    let session_prefixes: BTreeSet<String> = corpus
        .docs
        .values()
        .filter_map(|d| frontmatter_value(&d.raw, "originSessionId"))
        .map(|id| id.chars().take(8).collect())
        .collect();
    let mut cited: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (name, doc) in &corpus.docs {
        for sha in commit_shas(&doc.body, &session_prefixes) {
            cited.entry(sha).or_default().insert(name.clone());
        }
    }
    let all: BTreeSet<String> = cited.keys().cloned().collect();
    if !all.is_empty() {
        let repos = repos_under(code_root);
        for sha in unresolved_in_any(&all, &repos) {
            for memory in cited.get(&sha).into_iter().flatten() {
                findings.push(Finding {
                    severity: severity_of("unresolvable-commit"),
                    rule: "unresolvable-commit",
                    memory: memory.clone(),
                    detail: format!("`{sha}` is in no repository under {}", code_root.display()),
                });
            }
        }
    }

    findings.sort_by(|a, b| a.memory.cmp(&b.memory).then(a.detail.cmp(&b.detail)));
    findings
}

/// The `name:` line of a memory's frontmatter, if it declares one.
/// A frontmatter field's value, by key, at any indent.
///
/// Indent-insensitive so one helper serves both the top-level keys (`name:`)
/// and the ones nested under `metadata:` (`type:`, `modified:`). Keys are
/// matched with their colon, so `type:` does not also match `node_type:`.
///
/// ⚠ **Deliberately reads `raw` rather than the parsed [`crate::store::MemoryMeta`].**
/// That struct's `modified` is `Some` for every memory that exists, so a check
/// against it can never fire — which is exactly how `missing-modified` was first
/// written, and it passed a corpus full of missing stamps.
///
/// ⚠ **The reason has changed once and the conclusion did not.** It was always
/// `Some` when it came from the file's mtime; it now prefers the frontmatter
/// stamp and falls back to mtime, so it is still always `Some` and this rule
/// must still read `raw`. A note that survives the change it describes is the
/// dangerous kind. `mtype` has the same hazard from
/// the other direction: it falls back to the filename prefix, so a memory
/// declaring no type at all parses as a valid one. What the frontmatter *says*
/// is the only thing that travels with the file, and it is what these rules are
/// about.
fn frontmatter_value(raw: &str, key: &str) -> Option<String> {
    let rest = raw.strip_prefix("---\n")?;
    let end = rest.find("\n---")?;
    let needle = format!("{key}:");
    rest[..end].lines().find_map(|line| {
        let line = line.trim_start();
        let value = line.strip_prefix(&needle)?;
        let value = value.trim().trim_matches(['"', '\'']);
        (!value.is_empty()).then(|| value.to_string())
    })
}

/// The figures a memory declares retracted, from a `retracts:` frontmatter list.
///
/// ⚠ **A frontmatter FIELD, not a typed link, and the difference is the target.**
/// All six relations in `feedback_typed_memory_links` point memory-to-memory;
/// a retraction is a claim about a TOKEN — `173/173` — which is not a document
/// and cannot be the far end of a `[[link]]`. `supersedes` was the near miss and
/// it says the wrong thing: the superseded memory is history, whereas the memory
/// holding a retracted figure is usually current and correct apart from that
/// number.
///
/// The token is chosen by whoever retracts it, so it can be made as specific as
/// the case needs; a figure short enough to collide is a figure too short to
/// retract usefully.
///
/// ```text
/// retracts:
///   - "compare-match 173/173"
/// ```
fn retracted_figures(raw: &str) -> Vec<String> {
    let Some(rest) = raw.strip_prefix("---\n") else {
        return Vec::new();
    };
    let Some(end) = rest.find("\n---") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut inside = false;
    for line in rest[..end].lines() {
        if line.trim_start().starts_with("retracts:") {
            inside = true;
            continue;
        }
        if inside {
            let t = line.trim_start();
            // A list item belonging to `retracts:`; anything else ends the block,
            // including the next key at any indentation.
            if let Some(item) = t.strip_prefix("- ") {
                let item = item.trim().trim_matches(['"', '\'']);
                if !item.is_empty() {
                    out.push(item.to_string());
                }
            } else {
                inside = false;
            }
        }
    }
    out
}

fn frontmatter_name(raw: &str) -> Option<String> {
    let rest = raw.strip_prefix("---\n")?;
    let end = rest.find("\n---")?;
    for line in rest[..end].lines() {
        if let Some(value) = line.strip_prefix("name:") {
            let value = value.trim().trim_matches(['"', '\'']);
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// True when nothing at ERROR severity was found — the exit-code question.
pub fn passed(findings: &[Finding]) -> bool {
    !findings.iter().any(|f| f.severity == Severity::Error)
}

/// True when nothing at ERROR severity is **this session's to fix**.
///
/// `session` is `CLAUDE_CODE_SESSION_ID` when the linter runs inside a session,
/// and `None` when it does not — the nightly `claude-sync.sh` under launchd,
/// or a hand run. **`None` means strict**, so the job that gates the corpus
/// commit still refuses on any error at all; nothing about the corpus's own
/// standard has moved.
///
/// ⚠ **Why a session is treated differently.** `memory-lint` runs over the
/// SHARED corpus, so before this a session's commit failed on a memory some
/// other session had written minutes earlier, in a repo it had never touched —
/// five times in a week (memview #1047), each costing a full gate run to a
/// session that could not have caused it and could not tell who had. The corpus
/// is now alarmed where it belongs: `mem_check.py`'s `delivery` section reports
/// the nightly's verdict to fleetwatch, so a shared error is seen the same day
/// without blocking anybody.
///
/// ⚠ **`MEMORY.md` stays everybody's.** It carries no `originSessionId` to
/// attribute, it is the one document every session reads, and it was not the
/// source of any of the five — so an error in the index fails the gate for
/// whoever is standing there, deliberately.
///
/// ⚠ **An unstamped memory is nobody's**, which is exactly the #1047 class: it
/// has no `originSessionId`, so it matches no session and fails no gate. That
/// is the intended routing and not an oversight — it is unattributable by
/// construction, and the dashboard is the answer for it. `missing-modified`
/// stays an ERROR so the nightly still refuses to commit it.
///
/// ⚠ **`originSessionId` alone answers "whose memory is this", never "who broke
/// it"** — and those are different acts with the same signature (memview#1553).
/// Reproduced by accident: a bad `re.sub` wrote a literal `\g<1>` over another
/// session's `modified:` key, the frontmatter stopped parsing, and the two
/// errors that resulted — thirty seconds old and mine — printed as *"none of
/// them this session's"*. Creation is the wrong question, because most memories
/// a session edits it did not write.
///
/// So `wrote` is consulted as well: the recorded LAST WRITER of that memory's
/// file, which is what `last-writer.json` folds out of the transcripts.
///
/// ⚠ **[`Wrote::Unrecorded`] keeps the old behaviour, and that residual is
/// real.** Only about half the corpus has a recorded writer, which sounds thin
/// until the population is narrowed to the one that matters: nearly every memory
/// edited in the last week is recorded. A memory nobody has touched is not one
/// this session damaged, so the corpus-wide rate is diluted and the recent one
/// is honest. This closes most of the class, not all of it; do not describe it
/// as closing the class.
pub fn passed_for_session(
    corpus: &Corpus,
    findings: &[Finding],
    session: Option<&str>,
    wrote: impl Fn(&str) -> Wrote,
) -> bool {
    let Some(session) = session else {
        return passed(findings);
    };
    !findings.iter().any(|finding| {
        finding.severity == Severity::Error
            && match corpus.docs.get(&finding.memory) {
                // The index, and anything else with no document to ask.
                None => true,
                Some(doc) => {
                    frontmatter_value(&doc.raw, "originSessionId").as_deref() == Some(session)
                        // Not `||` over one expression on purpose: the two
                        // reasons a finding is yours are worth telling apart in
                        // a debugger, and the second is the newer claim.
                        || wrote(&finding.memory) == Wrote::Mine
                }
            }
    })
}

/// What the write record says about who last touched a memory.
///
/// ⚠ **Three answers, not a `bool`.** "Somebody else wrote it" and "nothing is
/// recorded" both mean *not this session*, and collapsing them would hide the
/// residual above behind a value that reads as a measurement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wrote {
    /// The record names this session as the last writer.
    Mine,
    /// The record names somebody else.
    Another,
    /// The record has nothing for this memory.
    Unrecorded,
}

/// Read a recorded writer as this session sees it.
///
/// ⚠ **In the library rather than in `memory-lint`, so a test can reach it.**
/// The three-way answer is the whole point of this change and a bin cannot be
/// tested — the same argument `stamped::missing` already makes about itself.
pub fn wrote_by(recorded: Option<&str>, me: &str) -> Wrote {
    match recorded {
        None => Wrote::Unrecorded,
        Some(who) if who == me => Wrote::Mine,
        Some(_) => Wrote::Another,
    }
}

/// How many findings each rule produced, for the summary line.
pub fn tally(findings: &[Finding]) -> BTreeMap<&'static str, usize> {
    let mut counts = BTreeMap::new();
    for f in findings {
        *counts.entry(f.rule).or_default() += 1;
    }
    counts
}

/// Relations actually used across the corpus, and how often — so the vocabulary
/// can be judged against what it is being asked to express.
pub fn relation_usage(corpus: &Corpus) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for doc in corpus.docs.values() {
        for link in &doc.links {
            let (relation, _) = split_relation(&link.target);
            let key = link
                .relation
                .clone()
                .or(relation)
                .unwrap_or_else(|| "(untyped)".to_string());
            *counts.entry(key).or_default() += 1;
        }
    }
    counts
}

/// Tolerance between a frontmatter `created` and the transcript entry for the
/// same write.
///
/// ⚠ **Measured, not chosen.** A stamp written by the Write tool lands a hair
/// AFTER the transcript entry it belongs to, while the smallest real defect is
/// minutes out. The two populations are an order of magnitude apart and this
/// sits in the gap, rather than on either edge where a jitter outlier or a lazy
/// round-minute would decide the rule.
const BIRTHDAY_SLACK_SECS: i64 = 60;

/// Referee each memory's stated birthday against the mined creation record.
///
/// Separate from [`check`] because the record is not part of the corpus: it is
/// `memory-created.json`, rebuilt from the transcripts by a run that reads
/// gigabytes. Passing the path rather than the parsed value keeps the
/// "could not look" case inside the rule, where it becomes a finding instead of
/// a silence.
pub fn check_created(corpus: &Corpus, record: &std::path::Path) -> Vec<Finding> {
    let mut findings = Vec::new();

    let Some(mined) = std::fs::read_to_string(record)
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
    else {
        findings.push(Finding {
            severity: severity_of("unreadable-created-record"),
            rule: "unreadable-created-record",
            memory: "(corpus)".to_string(),
            detail: format!("{} is missing or unparseable", record.display()),
        });
        return findings;
    };

    for (name, doc) in &corpus.docs {
        let Some(created) = doc.meta.created else {
            continue;
        };
        // ⚠ A memory the record does not name is a DETECTION GAP, not a memory
        // with no beginning — `memory-dated` draws the same line. The miner
        // misses a write it cannot parse, and a finding from that would accuse
        // the corpus of the miner's blind spot.
        let Some(first) = mined
            .get(name)
            .and_then(|v| v["first"].as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        else {
            continue;
        };
        let first = first.with_timezone(&chrono::Utc);
        if (created - first).num_seconds() > BIRTHDAY_SLACK_SECS {
            findings.push(Finding {
                severity: severity_of("created-after-first-write"),
                rule: "created-after-first-write",
                memory: name.clone(),
                detail: format!(
                    "created {} is after the first write the transcripts show, {}",
                    created.to_rfc3339(),
                    first.to_rfc3339()
                ),
            });
        }
    }

    findings
}
