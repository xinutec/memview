//! The `MEMORY.md` the corpus itself declares, beside the one a session wrote.
//!
//! Every other check here reports violations, candidates or an ordering — each
//! answers a question somebody thought to ask. This assembles a WHOLE FILE, and
//! the difference between two whole files shows what no per-line rule can: a
//! line that has drifted from the memory it describes, a section that should
//! not exist, an entry missing entirely.
//!
//! Pippijn, 2026-09-01, deciding how the corpus and its tooling relate:
//! *"You write and maintain MEMORY.md, and we'll have an algorithm that
//! generates the MEMORY.md we WOULD generate given the algorithmic and historic
//! data, but it's guiding you, not replacing you."*
//!
//! ⚠ **It never writes `MEMORY.md`.** It writes its own copy elsewhere and a
//! session reads the diff and decides. When the two disagree the ALGORITHM is
//! the first suspect — see `feedback_check_tokens_against_a_system_of_record`.
//!
//! ⚠ **Do not let it become authoritative by accident** (memview#1310). The
//! moment its output is pasted rather than read, the corpus is capped at this
//! algorithm's quality and nothing will ever read as wrong.
//!
//! ## What it decides, and what it deliberately does NOT
//!
//! **It assembles; it never writes prose.** Each memory carries its own index
//! line in its `teaser:` frontmatter, which is what that field exists for:
//! *"Let's make the teaser text part of the doc itself. The automation will be
//! structural, not linguistic."* A memory with no teaser cannot be generated,
//! and that is a signal rather than a failure.
//!
//! ⚠ **It does NOT decide ADMISSION, and that is a measured refusal rather than
//! an unfinished edge.** Which memories deserve a line is memview#822's open
//! question, and the study built to answer it (memview#884) harvested on
//! 2026-09-11 to a bounded null: every arm inside its own null band, the design
//! failing its own placebo by 2-4x the bands. Measured the same day, the
//! retirement route is closed too — 15 of 16 indexed tripwires sampled are
//! general claims about durable behaviour whose subjects never disappear, so
//! nothing expires and capacity is about one slot. Admission therefore rests on
//! a comparative value judgement that no measurement here can make, which is
//! why the cut stays Pippijn's. A generator that proposed one would be
//! inventing the answer this repo has twice failed to measure.
//!
//! ⚠ **It does NOT decide ORDER either, and inherits it.** Section order and
//! within-section order come from the file as written. Two reasons, and the
//! second is the load-bearing one: the sections are a curated taxonomy that
//! beats anything clustering would infer (the authored `##` headings and the
//! link clusters agree only 56%), and **a generated ordering would make every
//! line a diff line** — three hundred of them — burying the membership and
//! drift findings that are the point. So a matching order is NOT the algorithm
//! agreeing with the file; it is the algorithm declining to have an opinion.
//!
//! What that leaves it deciding is narrow and real: **which declared lines
//! exist, what each says, and whether they fit.**

use std::collections::{BTreeMap, BTreeSet};

use crate::store::Corpus;

/// One assembled index line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub name: String,
    /// The memory's own `teaser:` — never this module's words.
    pub teaser: String,
    /// What the line in the file says today, when the file carries one and it
    /// differs from the teaser. `None` when they agree or the memory is absent
    /// from the index.
    pub written: Option<String>,
}

impl Line {
    /// The line as the assembled file spells it.
    pub fn render(&self) -> String {
        format!("- [{}]({}.md)", self.teaser, self.name)
    }
}

/// One `##` section of the assembled file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub title: String,
    pub lines: Vec<Line>,
}

/// The assembled file, and every way it and the written one disagree.
#[derive(Debug, Clone, Default)]
pub struct Shadow {
    pub sections: Vec<Section>,
    /// Indexed today, but the memory declares no teaser.
    ///
    /// ⚠ **A DATA gap, never a proposal to drop.** The generator cannot spell a
    /// line the memory does not carry, so these are absent from the assembled
    /// file for a reason that says nothing about whether they belong. Reporting
    /// them as removals would be the generator asserting a judgement it did not
    /// make — measured 2026-09-11: 53 of 346 indexed memories.
    pub no_teaser: Vec<String>,
    /// Declares a teaser, and the written index does not carry it.
    ///
    /// ⚠ **Not an admission proposal either** — see the module note. A memory
    /// can declare a line and still not belong in the root; what this says is
    /// only that somebody wrote a line for it and the file does not have one.
    pub declared_not_carried: Vec<String>,
    /// The memory's teaser and the written line say different things.
    ///
    /// ⚠ **The finding this whole artefact exists to surface.** Pippijn's
    /// 2026-09-01 reason for moving the teaser into the doc was that a line in
    /// `MEMORY.md` could rot separately from the memory it describes, and
    /// nothing connected the two. This is the connection.
    pub drifted: Vec<Line>,
    /// Assembled lines the ceiling cuts, in the order they fall.
    pub over_ceiling: Vec<String>,
    pub bytes: usize,
}

/// Assemble the index the corpus declares.
///
/// `sections` is the written file's heading order and `section_of` its
/// placement, both read from `MEMORY.md` — the generator inherits the taxonomy
/// rather than inferring one, for the reason the module note gives.
pub fn assemble(corpus: &Corpus) -> Shadow {
    let Some(index) = corpus.index_md.as_deref() else {
        return Shadow::default();
    };
    // ⚠ One parsed reading of the file, shared by every question below —
    // placement, label and order. See `store::index_entries` for why this is
    // not pattern-matched.
    let entries = crate::store::index_entries(index);
    let (_, titles) = crate::store::index_sections(index);
    let written: BTreeMap<&str, &str> = entries
        .iter()
        .map(|e| (e.name.as_str(), e.label.as_str()))
        .collect();
    let section_of: BTreeMap<&str, &str> = entries
        .iter()
        .filter_map(|e| e.section.as_deref().map(|s| (e.name.as_str(), s)))
        .collect();
    let order: BTreeMap<&str, usize> = entries
        .iter()
        .enumerate()
        .map(|(i, e)| (e.name.as_str(), i))
        .collect();
    let linked: BTreeSet<&str> = entries.iter().map(|e| e.name.as_str()).collect();

    let mut shadow = Shadow::default();
    let mut placed: BTreeMap<&str, Vec<Line>> = BTreeMap::new();

    for (name, doc) in &corpus.docs {
        let Some(teaser) = doc.meta.teaser.as_deref() else {
            if linked.contains(name.as_str()) {
                shadow.no_teaser.push(name.clone());
            }
            continue;
        };
        let Some(section) = section_of.get(name.as_str()).copied() else {
            shadow.declared_not_carried.push(name.clone());
            continue;
        };
        // ⚠ Compared as TRIMMED text and nothing cleverer. A teaser that
        // differs only in surrounding whitespace is the same cue, and
        // reporting it as drift would bury the real cases in noise; anything
        // beyond that — punctuation, emphasis — is a real difference in what a
        // reader meets, so it is drift.
        let says = written.get(name.as_str()).copied();
        let line = Line {
            name: name.clone(),
            teaser: teaser.to_string(),
            written: says
                .filter(|w| w.trim() != teaser.trim())
                .map(str::to_string),
        };
        if line.written.is_some() {
            shadow.drifted.push(line.clone());
        }
        placed.entry(section).or_default().push(line);
    }

    // ⚠ **Within a section, the WRITTEN order — not alphabetical.** The file is
    // read top to bottom by a person and its order carries intent no field
    // records. Sorting here would rewrite every line of the diff.
    for title in &titles {
        let Some(mut lines) = placed.remove(title.as_str()) else {
            continue;
        };
        lines.sort_by_key(|l| order.get(l.name.as_str()).copied().unwrap_or(usize::MAX));
        shadow.sections.push(Section {
            title: title.clone(),
            lines,
        });
    }
    shadow.no_teaser.sort();
    shadow.declared_not_carried.sort();
    shadow.drifted.sort_by(|a, b| a.name.cmp(&b.name));

    let rendered = render(&shadow);
    shadow.bytes = rendered.len();
    // ⚠ **The ceiling is part of the artefact, not a warning beside it.** An
    // assembled index that ignores the limit is not the file we would have
    // written; it is a wish. `ceiling` owns the number and the cut model.
    let seen = crate::ceiling::cut(&rendered, crate::lint::INDEX_CEILING);
    if !seen.is_whole() {
        shadow.over_ceiling = crate::store::index_links(seen.dropped);
    }
    shadow
}

/// The assembled file.
pub fn render(shadow: &Shadow) -> String {
    let mut out = String::from("# Memory index\n");
    for section in &shadow.sections {
        out.push_str(&format!("\n## {}\n", section.title));
        for line in &section.lines {
            out.push_str(&line.render());
            out.push('\n');
        }
    }
    out
}
