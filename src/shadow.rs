//! The `MEMORY.md` the corpus itself declares, beside the one a session wrote:
//! the difference between two whole files shows what no per-line rule can.
//!
//! It guides, never replaces: it never writes `MEMORY.md`, and when the two
//! disagree it is the first suspect. It assembles each memory's `teaser:`; it does
//! not decide admission or order.
//!
//! All three are inherited from the written file (memview#1310):
//! - Membership: `memory-tiers` admits and demotes; a second rule here could only
//!   repeat it or contradict it.
//! - Sections: the authored `##` headings. Link clusters have no names and move
//!   whenever links do.
//! - Order: as written, which places hubs and warnings deliberately.
//!
//! What is left to it is drift: a line and its memory's teaser come apart.

use std::collections::{BTreeMap, BTreeSet};

use crate::store::Corpus;

/// One assembled index line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub name: String,
    /// The memory's own `teaser:` — never this module's words.
    pub teaser: String,
    /// What the file's line says, when it differs from the teaser.
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
    /// Indexed today, but the memory declares no teaser. A DATA gap, never a
    /// proposal to drop.
    pub no_teaser: Vec<String>,
    /// Declares a teaser, and the written index does not carry it. Not an
    /// admission proposal either.
    pub declared_not_carried: Vec<String>,
    /// The memory's teaser and the written line say different things — the finding
    /// this artefact exists to surface.
    pub drifted: Vec<Line>,
    /// Assembled lines the ceiling cuts, in the order they fall.
    pub over_ceiling: Vec<String>,
    pub bytes: usize,
}

/// Assemble the index the corpus declares. `sections` and `section_of` are read
/// from `MEMORY.md`: the generator inherits the taxonomy.
pub fn assemble(corpus: &Corpus) -> Shadow {
    let Some(index) = corpus.index_md.as_deref() else {
        return Shadow::default();
    };
    // One parsed reading of the file — see `store::index_entries`.
    let entries = crate::store::index_entries(index);
    let (_, titles) = crate::store::index_sections(index);
    let written: BTreeMap<&str, &str> = entries
        .iter()
        .map(|e| (e.name.as_str(), e.source.as_str()))
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
        // Compared as written and TRIMMED: whitespace is the same cue, anything
        // beyond, emphasis included, is a real difference in what a reader meets.
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

    // Within a section, the WRITTEN order: it carries intent no field records.
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
    // The ceiling is part of the artefact: an assembled index that ignores the
    // limit is a wish.
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
