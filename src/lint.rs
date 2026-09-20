//! Static analysis for the memory corpus itself: a document set with rules that
//! nothing checked — three links dead for weeks, twelve memories unreachable.
//!
//! Severity is two-tier and movable: a rule starts as a WARNING while the
//! backlog is worked down and is promoted to ERROR at zero, so the corpus
//! ratchets and cannot regress. Promoting is a one-word edit here.

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

/// The most a single index teaser may take from the ceiling — between a teaser's
/// length and a description's, so it catches a description pasted into the
/// wrong field.
pub const TEASER_MAX: usize = 140;

/// The injection ceiling, owned by [`crate::ceiling`]: one number, one owner, so
/// the cut model and the rule agree by construction.
pub use crate::ceiling::INDEX_CEILING;

/// Rules a corpus caught mid-write can fail through no fault of its own: writing
/// a memory is two edits, and the pre-commit gate can run between them. Here
/// rather than in `bin/memory-lint.rs`, where it named a rule that no longer
/// existed (memview#1456); `every_racy_rule_is_a_real_rule` fails on a rename.
pub const READ_LIMIT: usize = 2000;

pub const RACY: [&str; 2] = ["unreachable", "index-points-nowhere"];

/// Every rule, with the severity it currently carries — in one place, so a
/// promotion is a visible edit and the table reads as what a good memory looks like.
const RULES: &[(&str, Severity, &str)] = &[
    (
        "teaser-shape",
        Severity::Error,
        "a `teaser:` is one short line for the index, not a second description — \
         one line, at most TEASER_MAX bytes",
    ),
    (
        // A WARNING although the corpus is at zero: it reached zero in the very pass
        // that judged the backlog. Promote it once it has held.
        "unjudged-role",
        Severity::Warning,
        "an indexed memory with no `role:` and no entry in the judgement record \
         — an unjudged memory is held from demotion forever, so the index can \
         only grow (memview#1537)",
    ),
    (
        // A WARNING while violations remain. It may NEVER be promoted by writing
        // longer index lines: the index is near its ceiling. Zero comes from demoting
        // (memview#822).
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
        // Reported INSTEAD OF the field rules, not alongside them: when the frontmatter
        // did not parse, every field is a default and every field rule would accuse the
        // wrong thing. A duplicate `modified:` is the way this happens in practice.
        "unparsable-frontmatter",
        Severity::Error,
        "the frontmatter did not parse, so every field below it reads as ABSENT — \
         recall sees no description and no type, whatever the file says",
    ),
    (
        // Presence only, not accuracy. Do not rebuild this on mtime: most files
        // disagree with their own stamp by days (#1219). The message names a repair
        // tool; do NOT let that become an auto-fix — this failing is the only visible
        // symptom of a write that skipped the stamping path, and the author is the
        // harder half to recover.
        "missing-modified",
        Severity::Error,
        "no `modified` stamp — its age cannot be judged, so a stale claim reads as current; \
         `cargo run --bin memory-stamp` names the session that wrote it and repairs it",
    ),
    (
        // What this catches is a BACKFILL: a `created` later than the file's first
        // commit. Ordering only — whether either stamp is TRUE is not decidable here.
        "created-after-modified",
        Severity::Error,
        "`created` is later than `modified` — a memory cannot have been changed before it existed, \
         so one of the two stamps was written by something that did not check",
    ),
    (
        // A missing `name:` breaks nothing at runtime, which is the reason to check it.
        "missing-name",
        Severity::Error,
        "no `name:` in frontmatter — the memory does not state its own id",
    ),
    (
        // Absent AND out-of-vocabulary in one rule: `mtype` falls back to the filename
        // prefix, so both parse as a valid memory.
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
        // Reachability, not membership, at Pippijn's word: things have to be reachable,
        // but need not all be in MEMORY.md.
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
        // A WARNING until the corpus reaches zero: the root is over the line today, and
        // `claude-sync` withholds the whole corpus on any lint error (#1062). A LINT and
        // not a line in `MEMORY.md`'s header: that line has been there for weeks and the
        // root grew past the ceiling anyway (#822).
        Severity::Warning,
        "MEMORY.md is past the injection ceiling — the bottom is silently truncated and no session can tell",
    ),
    (
        "dangling-link",
        // Never promoted: a link to a memory that does not exist yet marks something
        // worth writing later.
        Severity::Warning,
        "links a memory that was never written — an intent marker, so this is a backlog and never an error",
    ),
    (
        // Match the shape, not the literal bytes: `**Why (the nixos-repo caution):**` is
        // better writing, and a strict match would have the corpus edited to satisfy it.
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
        // Armed at zero. WARNING, and NOT promoted without somewhere to route it: a
        // memory with no `originSessionId` can never be charged to anybody, so at ERROR
        // it would fail the nightly and no session (#1047). Catching this needs the
        // WRITER at write time (#1498). `missing-modified` does not cover it: a later
        // Edit stamps `modified:` and never adds the origin.
        "missing-origin",
        Severity::Warning,
        "no `originSessionId:` — nobody can be asked about it, and a rule that FAILED on it would block the nightly and no session",
    ),
    (
        // Armed before the cliff: past [`READ_LIMIT`] a memory still opens and looks
        // whole. The corpus has hit it twice.
        "past-read-limit",
        Severity::Error,
        "past the Read tool's default line limit, so the tail is silently unread",
    ),
    (
        // The same cliff with room to act: every split so far has been reactive.
        "nearing-read-limit",
        Severity::Warning,
        "over half the Read tool's default line limit and growing — split it deliberately, before the tail goes quiet",
    ),
    (
        // The failure it exists for has happened: a memory retracted a figure and went
        // on quoting it, defended by a prose banner nobody could check. Linking the
        // retraction is the requirement, because a phrasing cannot be read — a grep for
        // the banner found nothing.
        "quotes-a-retracted-figure",
        Severity::Error,
        "quotes a figure another memory declares retracted, without linking the memory that retracted it",
    ),
    (
        "unlinked-co-use",
        // Advisory, and never promoted: evidence, not a rule.
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
        // A warning, because a wrong sha and a repo not cloned here look identical.
        // Three filters, each measured: asking only the memory's own repo reported 65
        // dead of 237 (five of six existed elsewhere); decimal numbers that are valid
        // hex and 8-character session-id prefixes were the rest. 27% → 7.6% → 4.1%.
        "unresolvable-commit",
        Severity::Warning,
        "cites a commit hash that exists in no repository here — a mistyped sha, a rebased-away commit, or a repo that is not cloned on this machine",
    ),
    (
        // `created-after-modified`'s blind half: that rule caught one of six, the
        // other five having both stamps in order and both wrong. One-sided ON PURPOSE:
        // a `created` EARLIER than the mined first write is expected, since the archive
        // does not reach back forever.
        "created-after-first-write",
        Severity::Error,
        "`created` is later than the earliest write the transcripts record — a birthday typed \
         rather than looked up; `cargo run --bin memory-created` names the real one",
    ),
    (
        // This rule's whole input is mined by a separate run, so "no findings" and
        // "never ran" must not be the same output. WARNING: the record is absent on
        // every machine but the Mac.
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

/// What each rule is for, keyed by id — printed with a run's findings.
pub fn rule_reasons() -> BTreeMap<&'static str, (Severity, &'static str)> {
    RULES
        .iter()
        .map(|(id, sev, why)| (*id, (*sev, *why)))
        .collect()
}

/// Run every rule over the corpus. Findings sorted by severity then memory, so
/// the output reads as a worklist. `roles` is `memory-roles.json`, or `None`
/// where the caller has no reason to hold it; the `unjudged-role` rule is then
/// skipped rather than reporting a gap it cannot see.
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

    // A bound, not a style rule: every teaser is copied into a file with a hard
    // 24,400-byte ceiling. The longest index line is 123 bytes and a description's
    // median is 193, so this catches a description in the wrong field.
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
    // Every resolved outbound target and every governs-edge, collected here because
    // reciprocity can only be asked once the whole corpus has been walked.
    let mut outbound: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut governs: Vec<(String, String)> = Vec::new();
    let mut part_of: Vec<(String, String)> = Vec::new();

    for (name, doc) in &corpus.docs {
        if name.chars().any(char::is_uppercase) {
            push("uppercase-filename", name, format!("{name}.md"));
        }
        if let Some(err) = &doc.frontmatter_error {
            push("unparsable-frontmatter", name, err.clone());
        } else if doc.meta.description.trim().is_empty() {
            push("missing-description", name, "no description".to_string());
        }
        if frontmatter_value(&doc.raw, "modified").is_none() {
            push(
                "missing-modified",
                name,
                "no `modified:` in frontmatter".to_string(),
            );
        }
        // Compared as instants, not as text: `2026-08-19T09:00:00Z` and
        // `2026-08-21T08:00:00+03:00` are both in the corpus.
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
        // The frontmatter `name` is not trusted for lookup, but a disagreement means one
        // of the two is wrong.
        if let Some(declared) = frontmatter_name(&doc.raw)
            && declared != *name
        {
            push(
                "name-mismatch",
                name,
                format!("frontmatter says `{declared}`"),
            );
        }

        // Counted on `raw`: the Read tool's limit applies to the file on disk.
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

        // Every type, not just feedback: authorship is who to ask.
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
            // A colon-prefixed target `split_relation` refused is a misspelt or invented
            // relation; say which rather than reporting it as dangling.
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
        // Only the INDEXED set, because only it can reach a proposal: an unjudged
        // indexed entry is exempt from demotion forever, and the root can only grow.
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
                // Only a TRIPWIRE: a bare label is CORRECT for a pointer, and a tripwire's job
                // is to act on a reader who did not come looking.
                if role == crate::study::Role::Tripwire && !states_a_claim(&entry.label) {
                    push(
                        "mute-tripwire",
                        &entry.name,
                        format!("line reads {:?}", entry.label),
                    );
                }
            }
        }
        // Walk out from the index through the wikilinks, as a reader would — the same
        // question `memory-rank` asks with its demotions struck out. One implementation:
        // a second copy of this walk let that tool recommend a set that stranded a pair
        // (#869). Bytes, not entries, on the file as injected; `crate::ceiling` owns
        // the number and the cut model.
        let size = index.len();
        let seen = crate::ceiling::cut(index, INDEX_CEILING);
        if !seen.is_whole() {
            // Name the casualties, not an overage: "969 over" says nothing about which
            // memories stopped arriving. Order-preserving, and NOT `Vec::dedup`, which
            // drops only ADJACENT repeats.
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
            // These are the casualties: every name here is a memory a new session is not
            // given. The cut model can over-report by at most one partial line.
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

    // A retracted figure still quoted by a memory that does not link the
    // retraction. Asked after the whole corpus is walked.
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
            // The link is the requirement: a reader who lands on the figure is one hop
            // from what retracts it.
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
        // Undirected, built from the linked pairs: a backlink walks as well as a link.
        let mut adjacency: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for (a, b) in &pairs {
            adjacency.entry(a.clone()).or_default().insert(b.clone());
            adjacency.entry(b.clone()).or_default().insert(a.clone());
        }
        let (missing, connected) = couse.unlinked(&adjacency);
        // Capped, and the cap is reported: 500 suggestions is a wall, and a list that
        // quietly stops reads as "that is all".
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

/// Repo names that legitimately sit under `~/Code/` without being checkouts.
/// Enumerated: a missing entry is a visible finding.
const NOT_A_REPO: &[&str] = &[
    // The fleet-consistency conductor: a bare script, not a checkout.
    "check",
];

/// Words that name an 8-hex identifier as something other than a commit, read
/// in the [`KIND_WINDOW`] characters before the opening backtick: `snapshot`,
/// `restic`, `magic`, `bytecode`, `blob`, `hash-object`. Enumerated. `session`
/// is deliberately NOT here — session ids are excluded by `originSessionId`.
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

/// How far back the text before a token is read for [`NOT_A_COMMIT_KIND`]: 20
/// characters, measured — see [`commit_shas`].
const KIND_WINDOW: usize = 20;

/// The text of a memory a path claim can live in — prose, code and fences — with
/// link destinations left out. Parsed with comrak, since a substring scan misread
/// any link whose title contained `](`. Fences are KEPT here, unlike the index
/// parser: `run ~/Code/x/deploy.sh` is an instruction whether or not fenced. One
/// entry per TOP-LEVEL block, because the archive exemption is scoped to the
/// block — flattened, one retirement banner cleared every stale path below it.
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
    // The description is frontmatter, so no markdown node covers it, and a reader
    // sees it first. Its own block.
    blocks.push(doc.meta.description.clone());
    blocks
}

/// Every `~/Code/<segment>` a memory names, in either spelling; the absolute
/// form is derived from `code_root`, so no home path is baked in. Only the first
/// segment: a repo either exists or it does not.
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

/// Checks that reach outside the corpus, to the checkout root the memories
/// describe. Separate from [`check`], which is pure over the corpus.
///
/// A repo retired to `~/Archive` was recorded in one memory while many others
/// still sent a reader to `~/Code`, and every graph rule passed: none asks
/// whether the corpus is TRUE. Naming the new location, `~/Archive/<repo>`,
/// records the retirement and is exempt — per BLOCK, not per document, or a
/// banner in the first paragraph clears live paths forty lines down.
///
/// Sha-shaped tokens in backticks, minus the shapes that look identical and are
/// not commits: a decimal number is valid hex (`1048575`); a session id is
/// written the same way, so every declared `originSessionId` is excluded by its
/// first eight characters; and a restic snapshot, a Hermes magic number or a
/// `hash-object` blob is excused by the word IMMEDIATELY before it. The window
/// is 20 characters, measured: against 440 real citations it excuses none, and
/// 30 loses 5, 80 loses 14, the enclosing block 43. The inverse rule — require a
/// nearby word claiming the token IS a commit — was measured and REJECTED
/// (#1249): only 51.6% of real citations name one. Do not re-propose it.
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

/// Whether a block records `repo` as living on a remote it names — the
/// counterpart of the `~/Archive` exemption. Matched as the LAST segment after an
/// org, so `github.com/xinutec/phonos` excuses `phonos` and not `xinutec`.
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

/// Every git repository directly under the code root, plus the root itself (a
/// repository too) and the archive beside it: a retired repo still holds its
/// commits, and searching only the code root contradicted `dead-repo-path`. The
/// archive is a SIBLING of the root, so a test root reaches nothing real.
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
        // Not every repository the fleet uses lives under the code root:
        // `~/.config/home-manager` is cited, and its commits reported as existing
        // nowhere. The whole directory, relative to the root's parent.
        collect(parent.join(".config"));
        // `~/.claude` IS a repository, not a directory holding some, so `collect`
        // cannot reach it — and it is the repository the corpus itself lives in.
        let claude = parent.join(".claude");
        if claude.join(".git").exists() {
            repos.push(claude);
        }
    }
    repos
}

/// Which of `shas` no repository holds. `--batch-check` echoes the RESOLVED oid
/// for a hit, not the input, so answers are paired by ORDER; a count mismatch
/// skips the repo. Wrong, this reported all 394 tokens as dead.
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
    // Everything above asked `^{commit}`, so a git object of another type is still
    // in `left`: a `hash-object` blob was reported for a year as a sha that exists
    // nowhere. Asked bare, only of the leftovers, and dropped when it resolves.
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

/// `-C` sets the directory and loses to `GIT_DIR`, which the pre-commit hook
/// exports to everything it spawns — so inside the gate every `cat-file` would
/// ask the COMMITTING repository. Strip by PREFIX, never a list; `src/commits.rs`
/// carries the same guard. No live defect was demonstrated: object lookup turns
/// on `GIT_DIR`, `GIT_OBJECT_DIRECTORY` and `GIT_ALTERNATE_OBJECT_DIRECTORIES`,
/// and the old list named all three — this is drift-proofing, not a fix.
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

    // A root that cannot be read must report that, not pass.
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
        // Once per (memory, repo), however many blocks name it.
        let mut reported: BTreeSet<String> = BTreeSet::new();
        for block in claimable_blocks(doc) {
            for repo in code_repos_named(&block, code_root) {
                if code_root.join(&repo).exists() || reported.contains(&repo) {
                    continue;
                }
                // Naming the archive location IS the retirement record — per repo, and per BLOCK.
                if block.contains(&format!("~/Archive/{repo}")) {
                    continue;
                }
                // The third state: alive, pushed, and simply not cloned here. Naming the remote
                // beside the path is the same shape of record as naming the archive, accepted
                // on the same terms.
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

    // Commit claims, asked of every repo at once: one `cat-file` per repository,
    // where a call per citation would be ~370 spawns in a pre-commit gate.
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

/// A frontmatter field's value, by key, at any indent; keys are matched with
/// their colon. Deliberately reads `raw` rather than [`crate::store::MemoryMeta`],
/// whose `modified` is `Some` for every memory that exists — `missing-modified`
/// was first written against it and passed a corpus full of missing stamps.
/// `mtype` has the same hazard the other way.
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
/// A field, not a typed link: a retraction is a claim about a TOKEN, which cannot
/// be the far end of a `[[link]]`.
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

/// True when nothing at ERROR severity is THIS SESSION'S to fix.
///
/// `session` is `CLAUDE_CODE_SESSION_ID`, and `None` means strict — the nightly
/// still refuses on any error. A session is treated differently because
/// `memory-lint` runs over the SHARED corpus, and a session's commit failed five
/// times in a week on a memory another session had just written (memview #1047);
/// the nightly's verdict reaches fleetwatch instead. `MEMORY.md` stays
/// everybody's. An unstamped memory is nobody's, by construction.
///
/// `originSessionId` answers "whose memory", never "who broke it" (memview#1553),
/// so `wrote` — the recorded LAST WRITER from `last-writer.json` — is consulted
/// too. [`Wrote::Unrecorded`] keeps the old behaviour: about half the corpus has
/// a recorded writer, nearly all of it the recently edited half. This closes most
/// of the class, not all of it.
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
                        // Two conditions, not `||`: the two reasons a finding is yours are worth
                        // telling apart in a debugger.
                        || wrote(&finding.memory) == Wrote::Mine
                }
            }
    })
}

/// What the write record says about who last touched a memory. Three answers,
/// not a `bool`: "somebody else" and "nothing recorded" both mean not this
/// session, and collapsing them hides the residual.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wrote {
    /// The record names this session as the last writer.
    Mine,
    /// The record names somebody else.
    Another,
    /// The record has nothing for this memory.
    Unrecorded,
}

/// Read a recorded writer as this session sees it. In the library so a test can
/// reach it.
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
/// same write. Measured: a Write-tool stamp lands a hair AFTER its entry, the
/// smallest real defect is minutes out, and this sits in the gap.
const BIRTHDAY_SLACK_SECS: i64 = 60;

/// Referee each memory's stated birthday against the mined creation record —
/// `memory-created.json`, rebuilt from gigabytes of transcripts. The path is
/// passed so "could not look" becomes a finding, not a silence.
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
        // A memory the record does not name is a DETECTION GAP, not a memory with no
        // beginning; `memory-dated` draws the same line.
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
