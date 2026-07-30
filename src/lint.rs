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
use crate::store::{Corpus, RELATIONS, index_links, split_relation, wikilinks_of};

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
        "not-in-index",
        Severity::Error,
        "MEMORY.md links every memory; one it does not link is one nothing browses to",
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
        "missing-why",
        Severity::Warning,
        "a feedback memory needs **Why:** — a rule without its reason gets misapplied",
    ),
    (
        "missing-how",
        Severity::Warning,
        "a feedback memory needs **How to apply:** — a rule you cannot act on is a note",
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
            if !doc.body.contains("**Why:**") {
                push("missing-why", name, "no **Why:** line".to_string());
            }
            if !doc.body.contains("**How to apply:**") {
                push("missing-how", name, "no **How to apply:** line".to_string());
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
        let listed: BTreeSet<&String> = targets.iter().collect();
        for target in &targets {
            if !corpus.docs.contains_key(target) {
                push("index-points-nowhere", "MEMORY.md", format!("{target}.md"));
            }
        }
        for name in corpus.docs.keys() {
            if !listed.contains(name) {
                push(
                    "not-in-index",
                    name,
                    "MEMORY.md does not link it".to_string(),
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
