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
    wikilinks_of,
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

/// Every rule, with the severity it currently carries.
///
/// Listed in one place rather than beside each check so that the promotion of a
/// rule from warning to error is a visible, reviewable edit — and so this table
/// can be read as the answer to "what does a good memory look like".
const RULES: &[(&str, Severity, &str)] = &[
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
        // Introduced 2026-08-14 directly at ERROR, which the two-tier design
        // allows only because the corpus was taken to zero first: 190 of 542
        // memories carried no stamp, and every one was backfilled from the
        // file's mtime — with mtime then restored, since completing the
        // frontmatter is not a modification of what the memory says.
        //
        // ⚠ **Presence is not accuracy, and this rule only checks presence.**
        // Measured the same day: `modified` is never EARLIER than mtime but is
        // later than it on 115 files, i.e. a third of the stamps that already
        // existed were stale — the stamp is maintained by the memory-writing
        // path and goes silently wrong whenever a file is edited by any other
        // route. A freshness rule wants mtime, which is a property of the
        // filesystem rather than the corpus, so it would misfire on any synced
        // or restored copy. That check belongs beside this one, as a warning.
        //
        // ⚠ **The message names a repair tool; do NOT let that become an
        // auto-fix.** `memory-stamp` must stay a thing a person runs, because
        // this rule failing is the only visible symptom of a write that skipped
        // the stamping path, and the perishable half is the AUTHOR — recoverable
        // from the transcripts only until Claude Code prunes them. Silence the
        // rule and the authorship loss continues unseen; see
        // `feedback_a_precondition_that_can_pass_wrongly`. What the pointer
        // removes is the forensics, not the failure: the session that trips this
        // is usually not the one that wrote the file (three authors in three
        // hours, each invisible to itself and each blocking somebody else).
        "missing-modified",
        Severity::Error,
        "no `modified` stamp — its age cannot be judged, so a stale claim reads as current; \
         `cargo run --bin memory-stamp` names the session that wrote it and repairs it",
    ),
    (
        // Introduced 2026-08-14 at ERROR; the corpus was already at zero. The
        // stem is what everything resolves by, so a missing `name:` breaks
        // nothing at runtime — which is the reason to check it. It is the
        // memory's own statement of its id, and a reader quoting frontmatter
        // that is not there cites nothing.
        "missing-name",
        Severity::Error,
        "no `name:` in frontmatter — the memory does not state its own id",
    ),
    (
        // Introduced 2026-08-14 at ERROR; the corpus was already at zero, and
        // all four declared types were in vocabulary.
        //
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
        // **Reachability, not membership** — corrected 2026-08-02 at Pippijn's
        // word: *"MEMORY.md doesn't need to index everything. things have to be
        // reachable, but don't need to all be in MEMORY.md"*. The rule used to
        // demand an index line for every memory, and it failed the gate on a
        // corpus that was perfectly navigable: three lares memories had just
        // been consolidated under one index entry that links them all. What
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
        "dangling-link",
        // Never promoted, however low the count goes. The memory instructions
        // say a link to a memory that does not exist yet "marks something worth
        // writing later, not an error" — so this rule reports a backlog, and
        // making it fail would punish exactly the habit it is there to track.
        Severity::Warning,
        "links a memory that was never written — an intent marker, so this is a backlog and never an error",
    ),
    (
        // Promoted 2026-07-30, at zero. Nine of the nineteen this rule was
        // reporting turned out to be the CHECK, not the corpus: it demanded the
        // literal bytes `**Why:**`, so `**Why (the nixos-repo caution):**` —
        // better writing, a scope the rule genuinely has — read as no reason at
        // all. Fixing that first mattered: promoting the old check would have
        // made "phrase it exactly this way" an error, and the corpus would have
        // been edited to satisfy a string match.
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
        // Introduced 2026-08-24 at ERROR with the corpus at zero — the point of
        // it is to be armed before the cliff, not to describe a fall.
        //
        // The Read tool returns the first 2,000 lines by default. Past that a
        // memory still opens, still looks whole, and its tail is silently not
        // there — the one failure mode a reader cannot detect from the output.
        // The corpus has hit it twice and both times found out afterwards:
        // `project_health_lean_port_roadmap` reached 2,403 lines (split
        // 2026-08-14, its last 403 lines outside an ordinary read), and the log
        // that came out of it was pushed to 2,233 by appending (split again
        // 2026-08-22).
        "past-read-limit",
        Severity::Error,
        "over 2,000 lines: the Read tool's default stops there, so the tail is silently unread",
    ),
    (
        // The same cliff with room to act. 1,000 lines is 2.4x the corpus's p99
        // (415) and half the hard limit, so it marks a genuine outlier while
        // leaving a full thousand lines of headroom — at the log's observed
        // ~138 lines/day that is about a week to split deliberately rather than
        // in a panic.
        //
        // ⚠ **Both previous splits were reactive**, which is why the warning
        // exists at all: a rule that only fires once the tail is already
        // invisible reports a loss instead of preventing one.
        "nearing-read-limit",
        Severity::Warning,
        "over 1,000 lines and growing toward the Read tool's 2,000-line default — split it deliberately, before the tail goes quiet",
    ),
    (
        // Introduced 2026-08-25 at ERROR, with the corpus already at zero — the
        // three memories holding `173/173` all link the retraction already.
        //
        // ⚠ **The failure it exists for is measured, not imagined.**
        // `project_health_verified_core_lean` retracted `compare-match 173/173`
        // and then went on quoting it about TWELVE times, including in its own
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
        "untyped-links",
        Severity::Warning,
        "no link declares a relation, so the structure says only \"related\"",
    ),
    (
        // Introduced 2026-08-15 at WARNING with 47 of 65 governs-edges in
        // violation, worked down the same day and promoted at zero.
        //
        // ⚠ The 35 that were fixed by adding a `**What governs this:**` footer
        // were fixed in bulk, by the session that wrote this rule, in memories
        // it does not own. If a hub's owner wants the link somewhere better,
        // MOVE it — the rule only asks that the governed work name its governor,
        // not where. Demoting this back to a warning is the one-word edit below.
        //
        // The failure is specific and was measured, not imagined: a session
        // rewrote `project_life_emotion_suggestions` without having read the two
        // rules that declare `governs:project_life_app` and constrain that exact
        // feature — because the hub named one governor out of five, and the
        // cluster had been mapped by looking OUTWARD from the hub. Nothing in
        // the corpus was dangling, unreachable or stale; every existing rule
        // passed. Reachability says a reader CAN arrive; this says the reader
        // who is working on X is TOLD which rules bind X.
        //
        // ⚠ Filed against the target, because the target is the file that has to
        // change. The rule already did its part by declaring what it governs.
        //
        // Deliberately only `governs`. `part-of` and `because` point from the
        // component to the whole and from a claim to its reason, and demanding
        // the parent enumerate every child would turn a map into an index. A
        // governs-edge is the one relation whose whole purpose is that the
        // governed work knows about it.
        "governs-unreciprocated",
        Severity::Error,
        "a rule declares it governs this, and this does not name the rule back — so the rule only fires for a reader who already knew it existed",
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
        // Introduced 2026-08-25 at WARNING with 15 findings of 369 sha-shaped
        // tokens — 4.1%. A warning and not an error because the rule cannot
        // separate "this sha is wrong" from "the repo holding it is not cloned
        // here", and both look identical from the code root.
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
pub fn check(corpus: &Corpus, couse: Option<&CoUse>) -> Vec<Finding> {
    let mut findings = Vec::new();
    let mut push = |rule: &'static str, memory: &str, detail: String| {
        findings.push(Finding {
            severity: severity_of(rule),
            rule,
            memory: memory.to_string(),
            detail,
        });
    };

    let mut linked: BTreeSet<String> = BTreeSet::new();
    // Unordered pairs, for the co-use comparison: a link in either direction
    // means the two memories already know about each other.
    let mut pairs: BTreeSet<(String, String)> = BTreeSet::new();
    let mut typed_links = 0usize;
    let mut total_links = 0usize;
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
        if lines > 2000 {
            push("past-read-limit", name, format!("{lines} lines"));
        } else if lines > 1000 {
            push(
                "nearing-read-limit",
                name,
                format!("{lines} lines, {} from the limit", 2000 - lines),
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

        for link in wikilinks_of(&doc.body) {
            total_links += 1;
            if link.relation.is_some() {
                typed_links += 1;
            }
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

    // A rule that says it governs some work, against work that never mentions
    // the rule. Sorted so the report is stable and a hub's governors arrive
    // together rather than scattered through the run.
    //
    // ⚠ A hub may DELEGATE its rule list to a memory that declares itself
    // `part-of` it — `project_dicom_scan` keeps twelve rules in
    // `project_dicom_scan_rules`, which the hub links twice. That is better
    // organisation than an inline list, and the first draft of this rule
    // reported all nine of them as violations: exactly the mistake `unreachable`
    // made when it demanded an index line from a corpus that was perfectly
    // navigable, and was corrected for. So a governor named by an acknowledged
    // child counts as named.
    //
    // ONE hop, and only through `part-of`. A child's own children do not count:
    // the claim being checked is "a reader who opens this work is told what
    // binds it", and a page it explicitly hands off to is still that page's
    // answer. Anything deeper is just reachability, which `unreachable` already
    // owns.
    let mut delegates: BTreeMap<&String, Vec<&String>> = BTreeMap::new();
    for (child, parent) in &part_of {
        delegates.entry(parent).or_default().push(child);
    }
    let names_it =
        |holder: &String, rule: &String| outbound.get(holder).is_some_and(|out| out.contains(rule));
    governs.sort();
    for (rule, governed) in &governs {
        if names_it(governed, rule) {
            continue;
        }
        if let Some(children) = delegates.get(governed)
            && children.iter().any(|c| names_it(c, rule))
        {
            continue;
        }
        push(
            "governs-unreciprocated",
            governed,
            format!("governed by {rule}, which neither it nor any part-of child names"),
        );
    }

    if let Some(index) = corpus.index_md.as_deref() {
        let targets = index_links(index);
        for target in &targets {
            if !corpus.docs.contains_key(target) {
                push("index-points-nowhere", "MEMORY.md", format!("{target}.md"));
            }
        }
        // Walk out from the index through the wikilinks, as a reader would —
        // with nothing struck out, which is the same question `memory-rank` asks
        // with its demotions struck out. One invariant, one implementation: the
        // second copy of this walk is what let that tool grow a one-at-a-time
        // signature and recommend a set that stranded a pair (#869).
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

    // A corpus-wide observation rather than a per-file one: it is about the
    // shape of the whole link set, and reporting it against every memory would
    // bury every other finding.
    if total_links > 0 && typed_links * 20 < total_links {
        push(
            "untyped-links",
            "(corpus)",
            format!("{typed_links} of {total_links} links declare a relation"),
        );
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
/// **Why this exists.** On 2026-08-01 an audit found that `lares` had been retired
/// to `~/Archive/lares` and recorded in exactly one memory, while fifteen others —
/// four of them feedback rules, the highest-authority documents here — still sent a
/// reader to `~/Code/lares`. Every rule in the table above passed the whole time,
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
/// the per-repo check cannot separate them — the banner cleared the file. Measured
/// on the real corpus 2026-08-12: `project_lares_recon` carried the banner in its
/// first paragraph and four live paths below it, and this rule was silent on all
/// four while the audit that found them read the file by hand. A retirement note
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
            if hexish && !session_prefixes.contains(&tok) {
                out.insert(tok);
            }
            i = j + 1;
        } else {
            i = j;
        }
    }
    out
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
fn git_batch_check(repo: &std::path::Path, input: &str) -> std::io::Result<String> {
    use std::io::Write;
    let mut cmd = std::process::Command::new("git");
    cmd.arg("-C").arg(repo).args(["cat-file", "--batch-check"]);
    for var in [
        "GIT_DIR",
        "GIT_INDEX_FILE",
        "GIT_WORK_TREE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_COMMON_DIR",
        "GIT_CEILING_DIRECTORIES",
        "GIT_PREFIX",
        "GIT_CONFIG_PARAMETERS",
    ] {
        cmd.env_remove(var);
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
/// That struct's `modified` is populated from the file's mtime (`store.rs`), so
/// it is `Some` for every memory that exists and a check against it can never
/// fire — which is exactly how `missing-modified` was first written, and it
/// passed a corpus with 190 missing stamps. `mtype` has the same hazard from
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
pub fn passed_for_session(corpus: &Corpus, findings: &[Finding], session: Option<&str>) -> bool {
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
                }
            }
    })
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
        for link in wikilinks_of(&doc.body) {
            let (relation, _) = split_relation(&link.target);
            let key = link
                .relation
                .or(relation)
                .unwrap_or_else(|| "(untyped)".to_string());
            *counts.entry(key).or_default() += 1;
        }
    }
    counts
}
