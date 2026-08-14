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
    Corpus, RELATIONS, has_section, index_links, reachable_without, split_relation, wikilinks_of,
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
        "dead-repo-path",
        Severity::Error,
        "names a `~/Code/<repo>` that does not exist, and does not say where it went",
    ),
    (
        "unresolvable-code-root",
        Severity::Error,
        "the checkout root could not be read, so no path claim was actually verified",
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

    for (name, doc) in &corpus.docs {
        if name.chars().any(char::is_uppercase) {
            push("uppercase-filename", name, format!("{name}.md"));
        }
        if doc.meta.description.trim().is_empty() {
            push("missing-description", name, "no description".to_string());
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

    // What the transcripts say belongs together and the corpus does not.
    if let Some(couse) = couse {
        let missing = couse.unlinked(&pairs);
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
                    "{} more unlinked pairs seen in >= {} sessions; showing the {SHOWN} strongest",
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

    findings.sort_by(|a, b| a.memory.cmp(&b.memory).then(a.detail.cmp(&b.detail)));
    findings
}

/// The `name:` line of a memory's frontmatter, if it declares one.
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
