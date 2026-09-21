//! The memory corpus: one directory of markdown files with YAML frontmatter and
//! `[[name]]` cross-references, plus a MEMORY.md index whose links are
//! `[title](file.md)`. Loaded fresh from disk on every request — the corpus is
//! small and the writer is a live Claude session. Rendering rewrites both link
//! forms to the SPA route `/m/<name>`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use comrak::nodes::{NodeLink, NodeValue};
use comrak::{Arena, Options, format_html, parse_document};
use serde::{Deserialize, Serialize};

use crate::couse::Usage;
use crate::rank;

#[derive(Debug, Deserialize, Default)]
struct FrontmatterMeta {
    #[serde(rename = "type")]
    mtype: Option<String>,
    /// The session that wrote this memory. Camel-case on disk, so it is renamed.
    #[serde(rename = "originSessionId")]
    origin_session: Option<String>,
    /// When the memory itself says it last changed — not the file's mtime, which
    /// records a touch and is wrong by days (#1219).
    modified: Option<String>,
    /// When the memory was first written. Recovered from the transcripts by
    /// `memory-dated`, not observed. Absent on a memory no surviving transcript
    /// records — a DETECTION gap, never an mtime.
    created: Option<String>,
    /// Never read as the judgement: `role` belongs at the TOP level, beside
    /// `description`. Captured only so a misplacement can be REPORTED. Five
    /// declarations sat here being ignored, and serde drops an unknown key without
    /// a word, so an ignored declaration and an absent one looked identical — the
    /// memory read as unjudged while its author believed it judged (memview#1537).
    role: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct Frontmatter {
    description: Option<String>,
    /// The memory's own line in the index. See [`MemoryMeta::teaser`].
    teaser: Option<String>,
    /// `tripwire` or `pointer`, declared by the author. See [`MemoryMeta::role`].
    role: Option<String>,
    #[serde(default)]
    metadata: Option<FrontmatterMeta>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryMeta {
    /// Canonical id = filename stem; frontmatter `name` normally matches.
    pub name: String,
    /// The frontmatter description, held as markdown and sent as HTML: ranking
    /// tokenises it and linting measures it, and only serialising is for a reader.
    #[serde(serialize_with = "as_inline_html")]
    pub description: String,
    /// This memory's line in the index — the cue a reader meets in a list of three
    /// hundred. Deliberately NOT `description`, which runs long; generating the index
    /// from descriptions overruns [`crate::ceiling::INDEX_CEILING`] several times.
    /// Lives with the memory so it cannot rot apart from it. Absent means
    /// index-ineligible, not an error (memview#822, #1310).
    pub teaser: Option<String>,
    /// What the index line is FOR — `tripwire` or `pointer`. The raw string, since
    /// the vocabulary belongs to [`crate::study::Role`]; a typo reads as unjudged.
    /// Absent is UNEXAMINED, never "safe to demote": a judgement held only in
    /// `memory-roles.json` cannot grow with the corpus (memview#1537).
    pub role: Option<String>,
    /// Whether a `role:` was written under `metadata:`, where nothing reads it.
    /// Reported rather than honoured: guessing the author's intent would make the
    /// wrong spelling work and the schema meaningless.
    ///
    /// Off the wire. A frontmatter spelling mistake is a fact for `memory-lint`,
    /// not something a reader of the memory needs, and every serialized field here
    /// is one the TypeScript mirror must carry.
    #[serde(skip)]
    pub misplaced_role: bool,
    /// user | feedback | project | reference (from metadata.type, falling
    /// back to the filename prefix).
    pub mtype: String,
    pub modified: Option<DateTime<Utc>>,
    /// When it was first written, if a transcript still said so when
    /// `memory-dated` ran. See [`FrontmatterMeta::created`].
    pub created: Option<DateTime<Utc>>,
}

#[derive(Debug)]
pub struct MemoryDoc {
    pub meta: MemoryMeta,
    /// Markdown body (frontmatter stripped).
    pub body: String,
    /// This memory's outgoing wikilinks, parsed ONCE at load. Parsing them per
    /// query was the whole cost of the memory tools — ~446,000 parses in `homes_for`
    /// (memview#1274). The corpus is SMALL; anything slow here is slow by shape.
    pub links: Vec<Wikilink>,
    /// The file exactly as written, so linting can see what the frontmatter SAYS.
    pub raw: String,
    /// Why the frontmatter did not parse, when it did not. `Some` means every field
    /// below is a DEFAULT rather than what the file says.
    pub frontmatter_error: Option<String>,
    /// The session that wrote this memory, if it declares one. Deliberately NOT on
    /// [`MemoryMeta`], which is serialised into every list and graph node a
    /// share-link recipient may read; resolving a session to its agent is what
    /// `/api/agents` is owner-only to protect.
    pub origin_session: Option<String>,
}

pub struct Corpus {
    pub docs: BTreeMap<String, MemoryDoc>,
    /// Raw markdown of MEMORY.md (None if absent).
    pub index_md: Option<String>,
}

/// Split "---\n<yaml>\n---\n<body>". Files without frontmatter are all body.
fn split_frontmatter(text: &str) -> (Option<&str>, &str) {
    let Some(rest) = text.strip_prefix("---\n") else {
        return (None, text);
    };
    match rest.find("\n---") {
        Some(end) => {
            let yaml = &rest[..end];
            let after = &rest[end + 4..];
            (Some(yaml), after.strip_prefix('\n').unwrap_or(after))
        }
        None => (None, text),
    }
}

impl Corpus {
    pub fn load(dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref();
        let mut docs = BTreeMap::new();
        let mut index_md = None;
        let entries = std::fs::read_dir(dir)
            .with_context(|| format!("reading memory dir {}", dir.display()))?;
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            let Some(fname) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !fname.ends_with(".md") {
                continue;
            }
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            if fname == "MEMORY.md" {
                index_md = Some(text);
                continue;
            }
            let name = fname.trim_end_matches(".md").to_string();
            let (yaml, body) = split_frontmatter(&text);
            // A parse failure must not read as an ABSENT field. serde_yaml rejects a
            // duplicate key, so one stray second `modified:` used to blank the whole
            // frontmatter and `memory-lint` then reported "no description" — the wrong
            // field and the wrong layer. Keep the default so one bad file cannot hide a
            // memory, but carry the error so linting can name the real cause.
            let mut frontmatter_error = None;
            let fm: Frontmatter = match yaml {
                Some(y) => serde_yaml::from_str(y).unwrap_or_else(|e| {
                    frontmatter_error = Some(e.to_string());
                    Frontmatter::default()
                }),
                None => Frontmatter::default(),
            };
            let meta = fm.metadata.unwrap_or_default();
            // Checked before `meta` is consumed below. A `role:` written here is not
            // an alternative spelling to honour — it is a mistake to name.
            let misplaced = meta.role.as_deref().is_some_and(|r| !r.trim().is_empty());
            let mtype = meta
                .mtype
                .unwrap_or_else(|| name.split('_').next().unwrap_or("other").to_string());
            // An empty value is absent: an origin that resolves to nothing renders as an
            // agent that never was.
            let origin_session = meta.origin_session.filter(|s| !s.trim().is_empty());
            // The stamp the memory keeps, falling back to mtime only when it has none —
            // which `memory-lint` reports as an error.
            let modified = meta
                .modified
                .as_deref()
                .and_then(|stamp| DateTime::parse_from_rfc3339(stamp).ok())
                .map(|stamp| stamp.with_timezone(&Utc))
                .or_else(|| {
                    entry
                        .metadata()
                        .ok()
                        .and_then(|m| m.modified().ok())
                        .map(DateTime::<Utc>::from)
                });
            // No mtime fallback here: an mtime is the last touch, a different fact from a
            // creation date, and a wrong date that looks present is worse than an absent one.
            let created = meta
                .created
                .as_deref()
                .and_then(|stamp| DateTime::parse_from_rfc3339(stamp).ok())
                .map(|stamp| stamp.with_timezone(&Utc));
            // Canonical id is the filename stem; a frontmatter mismatch should not hide a file.
            docs.insert(
                name.clone(),
                MemoryDoc {
                    meta: MemoryMeta {
                        name,
                        description: fm.description.unwrap_or_default(),
                        teaser: fm
                            .teaser
                            .map(|t| t.trim().to_string())
                            .filter(|t| !t.is_empty()),
                        role: fm
                            .role
                            .map(|r| r.trim().to_lowercase())
                            .filter(|r| !r.is_empty()),
                        misplaced_role: misplaced,
                        mtype,
                        modified,
                        created,
                    },
                    links: wikilinks(body),
                    body: body.to_string(),
                    raw: text.clone(),
                    frontmatter_error,
                    origin_session,
                },
            );
        }
        Ok(Self { docs, index_md })
    }

    pub fn get(&self, name: &str) -> Option<&MemoryDoc> {
        self.docs.get(name)
    }

    pub fn list(&self) -> Vec<MemoryMeta> {
        self.docs.values().map(|d| d.meta.clone()).collect()
    }

    /// Names of memories whose body wikilinks to `name`. Shares `wikilink_targets`
    /// with `outlinks` so the two directions cannot disagree.
    pub fn backlinks(&self, name: &str) -> Vec<MemoryMeta> {
        self.docs
            .values()
            .filter(|d| d.meta.name != name && wikilink_targets(&d.body).iter().any(|t| t == name))
            .map(|d| d.meta.clone())
            .collect()
    }

    /// `[[targets]]` referenced by this doc, split into existing and dangling; a
    /// dangling wikilink marks something worth writing.
    pub fn outlinks(&self, doc: &MemoryDoc) -> (Vec<MemoryMeta>, Vec<String>) {
        let mut existing = Vec::new();
        let mut dangling = Vec::new();
        for target in wikilink_targets(&doc.body) {
            if target == doc.meta.name {
                continue;
            }
            match self.docs.get(&target) {
                Some(d) => {
                    if !existing.iter().any(|m: &MemoryMeta| m.name == target) {
                        existing.push(d.meta.clone());
                    }
                }
                None => {
                    if !dangling.contains(&target) {
                        dangling.push(target);
                    }
                }
            }
        }
        (existing, dangling)
    }

    /// Memories matching `query`, best first, and whether the query had to be
    /// relaxed. `usage` is the co-use artefact, a mild prior. Every term is
    /// required first; only if that finds nothing is any term accepted — and the
    /// fact is returned, never swallowed.
    pub fn search(&self, query: &str, usage: &BTreeMap<String, Usage>) -> SearchResult {
        if query.trim().is_empty() {
            return SearchResult::default();
        }
        let docs: Vec<rank::Doc<'_>> = self
            .docs
            .values()
            .map(|d| rank::Doc {
                name: &d.meta.name,
                description: &d.meta.description,
                body: &d.body,
                usage: usage.get(&d.meta.name),
            })
            .collect();

        let mut relaxed = false;
        let mut scored = rank::rank(&docs, query, true);
        if scored.is_empty() {
            scored = rank::rank(&docs, query, false);
            relaxed = !scored.is_empty();
        }

        let values: Vec<&MemoryDoc> = self.docs.values().collect();
        let hits = scored
            .into_iter()
            .map(|s| {
                let d = values[s.index];
                // Snippet anchored on the RAREST term the memory holds: the query as typed
                // usually appears nowhere, and offset zero would show the frontmatter.
                let pos = rank::tokenize(query)
                    .iter()
                    .filter_map(|t| find_ci(&d.body, t).map(|p| (t.len(), p)))
                    .max_by_key(|(len, _)| *len)
                    .map(|(_, p)| p);
                SearchHit {
                    meta: d.meta.clone(),
                    snippet: pos.map(|p| snippet_around(&d.body, p, query.len())),
                    score: (s.score * 100.0).round() / 100.0,
                }
            })
            .collect();
        SearchResult { hits, relaxed }
    }

    /// The whole corpus as a link graph. Shares `wikilink_targets` with
    /// `backlinks`/`outlinks`. Dangling wikilinks are absent; `outlinks` keeps them visible.
    pub fn graph(&self) -> Graph {
        let (section_of, sections) = self
            .index_md
            .as_deref()
            .map(index_sections)
            .unwrap_or_default();

        let mut edges = Vec::new();
        let mut in_degree: BTreeMap<String, usize> = BTreeMap::new();
        let mut out_degree: BTreeMap<String, usize> = BTreeMap::new();
        for doc in self.docs.values() {
            let mut seen = BTreeSet::new();
            for link in &doc.links {
                // Twice is one relationship, self is none, and a typed mention beats an untyped
                // one for the same pair.
                if link.target == doc.meta.name || !self.docs.contains_key(&link.target) {
                    continue;
                }
                if !seen.insert(link.target.clone()) {
                    if let Some(relation) = link.relation.clone()
                        && let Some(existing) = edges.iter_mut().find(|e: &&mut GraphEdge| {
                            e.source == doc.meta.name && e.target == link.target
                        })
                    {
                        existing.relation.get_or_insert(relation);
                    }
                    continue;
                }
                *out_degree.entry(doc.meta.name.clone()).or_default() += 1;
                *in_degree.entry(link.target.clone()).or_default() += 1;
                edges.push(GraphEdge {
                    source: doc.meta.name.clone(),
                    target: link.target.clone(),
                    relation: link.relation.clone(),
                });
            }
        }

        let nodes = self
            .docs
            .values()
            .map(|d| GraphNode {
                meta: d.meta.clone(),
                section: section_of.get(&d.meta.name).cloned(),
                size: d.body.len(),
                in_degree: in_degree.get(&d.meta.name).copied().unwrap_or(0),
                out_degree: out_degree.get(&d.meta.name).copied().unwrap_or(0),
            })
            .collect();

        Graph {
            nodes,
            edges,
            sections,
            // Filled by the route when a co-use artefact is loaded.
            as_of: None,
            usage: Default::default(),
            affinities: Default::default(),
        }
    }
}

/// Map each memory to the `## section` of MEMORY.md that indexes it, and list
/// the section titles in index order. One comrak walk for both callers: the
/// line scanner it replaced missed `setext` headings, misread links whose title
/// held `](`, and indexed links inside fenced code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexEntry {
    pub name: String,
    /// The link's visible label — the cue a reader actually meets.
    pub label: String,
    pub section: Option<String>,
}

/// Every link in the written index, in order. First mention wins per name; a
/// later mention is a cross-reference.
pub fn index_entries(index_md: &str) -> Vec<IndexEntry> {
    let options = markdown_options();
    let arena = Arena::new();
    let root = parse_document(&arena, index_md, &options);
    let mut out: Vec<IndexEntry> = Vec::new();
    let mut seen = BTreeSet::new();
    let mut current: Option<String> = None;
    for node in root.descendants() {
        let value = node.data.borrow().value.clone();
        match value {
            NodeValue::Heading(h) if h.level == 2 => {
                let title = node_text(node);
                if !title.is_empty() {
                    current = Some(title);
                }
            }
            NodeValue::Link(link) => {
                let Some(stem) = md_link_stem(&link.url) else {
                    continue;
                };
                if !seen.insert(stem.to_string()) {
                    continue;
                }
                out.push(IndexEntry {
                    name: stem.to_string(),
                    label: node_text(node).trim().to_string(),
                    section: current.clone(),
                });
            }
            _ => {}
        }
    }
    out
}

pub(crate) fn index_sections(index_md: &str) -> (BTreeMap<String, String>, Vec<String>) {
    let options = markdown_options();
    let arena = Arena::new();
    let root = parse_document(&arena, index_md, &options);
    let mut section_of = BTreeMap::new();
    let mut sections: Vec<String> = Vec::new();
    let mut current: Option<String> = None;
    // Pre-order, so a heading is always visited before the links beneath it.
    for node in root.descendants() {
        let value = node.data.borrow().value.clone();
        match value {
            NodeValue::Heading(h) if h.level == 2 => {
                let title = node_text(node);
                if !title.is_empty() {
                    if !sections.contains(&title) {
                        sections.push(title.clone());
                    }
                    current = Some(title);
                }
            }
            NodeValue::Link(link) => {
                let Some(section) = current.as_ref() else {
                    continue;
                };
                if let Some(stem) = md_link_stem(&link.url) {
                    section_of
                        .entry(stem.to_string())
                        .or_insert_with(|| section.clone());
                }
            }
            _ => {}
        }
    }
    (section_of, sections)
}

/// Everything a reader arrives at from the index, with `demoting` struck out. A
/// SET, never one name at a time: two memories linking only each other are each
/// other's home and both look safe until both lines go — `memory-rank` offered
/// exactly that pair (#869). One place, both callers.
pub fn reachable_without(
    docs: &BTreeMap<String, MemoryDoc>,
    index_md: &str,
    demoting: &BTreeSet<String>,
) -> BTreeSet<String> {
    depths_without(docs, index_md, demoting)
        .into_keys()
        .collect()
}

/// How many links a reader follows from the index to reach each memory. Depth 1
/// is DIRECTLY linked; absent is unreachable. The half of the root/traversal
/// question nothing measured: every other signal describes USE, this describes
/// POSITION. Breadth-first — `reachable_without` used a stack, which is right for
/// reachability and wrong for distance — and `demoting` is struck out first.
pub fn depths_without(
    docs: &BTreeMap<String, MemoryDoc>,
    index_md: &str,
    demoting: &BTreeSet<String>,
) -> BTreeMap<String, usize> {
    let mut depth: BTreeMap<String, usize> = BTreeMap::new();
    let mut queue: std::collections::VecDeque<(String, usize)> = index_links(index_md)
        .into_iter()
        .filter(|name| docs.contains_key(name) && !demoting.contains(name))
        .map(|name| (name, 1))
        .collect();
    while let Some((name, at)) = queue.pop_front() {
        let Some(doc) = docs.get(&name) else { continue };
        // First arrival wins: BFS reaches a name by its shortest path.
        if depth.contains_key(&name) {
            continue;
        }
        depth.insert(name, at);
        for link in &doc.links {
            if docs.contains_key(&link.target) && !depth.contains_key(&link.target) {
                queue.push_back((link.target.clone(), at + 1));
            }
        }
    }
    depth
}

/// Every `name.md` the index links, in order — including any written before the
/// first heading, which `index_sections` deliberately files under no section.
pub fn index_links(index_md: &str) -> Vec<String> {
    let options = markdown_options();
    let arena = Arena::new();
    let root = parse_document(&arena, index_md, &options);
    let mut out = Vec::new();
    for node in root.descendants() {
        if let NodeValue::Link(link) = &node.data.borrow().value
            && let Some(stem) = md_link_stem(&link.url)
        {
            out.push(stem.to_string());
        }
    }
    out
}

/// Every bold run in a document, as plain text. Parsed, not string-matched:
/// `**Why:**` inside a fenced example is a sample, not a rule.
pub fn bold_runs(body: &str) -> Vec<String> {
    let options = markdown_options();
    let arena = Arena::new();
    let root = parse_document(&arena, body, &options);
    let mut out = Vec::new();
    for node in root.descendants() {
        if matches!(node.data.borrow().value, NodeValue::Strong) {
            out.push(node_text(node));
        }
    }
    out
}

/// Whether some bold run opens a section named `heading`. Loose about what
/// follows: `**Why (the nixos-repo caution):**` is better writing, not an absent
/// section.
pub fn has_section(body: &str, heading: &str) -> bool {
    bold_runs(body).iter().any(|run| {
        let run = run.trim();
        let Some(rest) = run
            .to_ascii_lowercase()
            .strip_prefix(&heading.to_ascii_lowercase())
            .map(str::to_string)
        else {
            return false;
        };
        // A whole word opening the run: `**Why (2026-07-21):**` is this section,
        // `**Whyever**` is not.
        rest.is_empty() || rest.starts_with([':', '.', ' ', ',', '(', '—', '-'])
    })
}

/// The plain text of a node — its descendant text and code runs, concatenated.
fn node_text<'a>(
    node: &'a comrak::arena_tree::Node<'a, std::cell::RefCell<comrak::nodes::Ast>>,
) -> String {
    let mut out = String::new();
    for child in node.descendants() {
        match &child.data.borrow().value {
            NodeValue::Text(text) => out.push_str(text),
            NodeValue::Code(code) => out.push_str(&code.literal),
            _ => {}
        }
    }
    out.trim().to_string()
}

/// A memory as a node in the link graph: its metadata plus the structural
/// facts a layout needs.
#[derive(Debug, Clone, Serialize)]
pub struct GraphNode {
    #[serde(flatten)]
    pub meta: MemoryMeta,
    /// The `## section` of MEMORY.md that indexes this memory — the curated
    /// taxonomy. `None` when the index never links it under a heading.
    pub section: Option<String>,
    /// Body length in bytes; spans ~50x, so radii should scale log-wise.
    pub size: usize,
    pub in_degree: usize,
    pub out_degree: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphEdge {
    pub source: String,
    pub target: String,
    /// What the link claims, or `None` for a plain mention. See [`RELATIONS`].
    pub relation: Option<String>,
}

#[derive(Debug, Default, Serialize)]
pub struct Graph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    /// When the co-use mine that produced `usage` and `affinities` last ran. The
    /// defect this closes is SILENCE: a graph missing yesterday's work and one
    /// missing nothing looked identical (memview#1274).
    pub as_of: Option<String>,
    /// How much each memory is used, keyed by name. Empty when there is no co-use
    /// artefact; the picture degrades to structure.
    #[serde(default)]
    pub usage: std::collections::BTreeMap<String, crate::couse::Usage>,
    /// Pairs the work keeps using together — a second, weaker pull: 71% cross a
    /// region boundary drawn from the written links alone.
    #[serde(default)]
    pub affinities: Vec<crate::couse::Pair>,
    /// Section titles in MEMORY.md order, so a legend reads in the order the
    /// index was written rather than alphabetically.
    pub sections: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct SearchHit {
    #[serde(flatten)]
    pub meta: MemoryMeta,
    pub snippet: Option<String>,
    /// BM25 score, exposed so a ranking regression shows in the response.
    pub score: f64,
}

/// What a search found, and whether it had to widen the question to find it.
#[derive(Debug, Default, Serialize)]
pub struct SearchResult {
    pub hits: Vec<SearchHit>,
    /// True when nothing matched every term and the query fell back to "any
    /// term". The page says so — see the note in [`Corpus::search`].
    pub relaxed: bool,
}

/// The relations a link may declare. A closed vocabulary: 856 links all said
/// "related" while doing five different jobs. Unknown prefixes are NOT
/// tolerated — `[[superseeds:x]]` stays in the target and shows as dangling.
pub const MEMORY_TYPES: [&str; 4] = ["user", "feedback", "project", "reference"];

pub const RELATIONS: [&str; 6] = [
    // This memory is a component of that one.
    "part-of",
    // This rule applies to that work.
    "governs",
    // That memory is the reason for what this one says.
    "because",
    // This narrows or extends that one.
    "refines",
    // This replaces that one, which is now history.
    "supersedes",
    // A known, unresolved tension between the two.
    "contradicts",
];

/// One `[[link]]`: who it points at, and what it claims about the relationship.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Wikilink {
    /// `None` for a plain `[[name]]` — a mention, with nothing asserted.
    pub relation: Option<String>,
    pub target: String,
}

/// Split a wikilink's inner text into an optional relation and a target. Only a
/// prefix in [`RELATIONS`] counts; anything else fails loudly as dangling.
pub fn split_relation(inner: &str) -> (Option<String>, String) {
    if let Some((prefix, rest)) = inner.split_once(':')
        && RELATIONS.contains(&prefix)
        && !rest.is_empty()
    {
        return (Some(prefix.to_string()), rest.to_string());
    }
    (None, inner.to_string())
}

/// Extract every `[[wikilink]]`, in order. Parsed with comrak so this and the
/// rendered HTML cannot disagree: a hand-rolled scanner reported `${x[[-n
/// "$target"]]}` and a `[[la,lo,ts]]` tuple as links. A wikilink can straddle a
/// wrapped source line.
fn wikilink_targets(body: &str) -> Vec<String> {
    wikilinks(body).into_iter().map(|l| l.target).collect()
}

/// Every `[[link]]` in the body with its relation. Public so the linter sees
/// exactly the links the viewer does.
pub fn wikilinks_of(body: &str) -> Vec<Wikilink> {
    wikilinks(body)
}

fn wikilinks(body: &str) -> Vec<Wikilink> {
    let options = markdown_options();
    let arena = Arena::new();
    let root = parse_document(&arena, body, &options);
    let mut out = Vec::new();
    for node in root.descendants() {
        if let NodeValue::WikiLink(wl) = &node.data.borrow().value {
            let target = wl.url.split('|').next().unwrap_or(&wl.url);
            let target = target.split_whitespace().collect::<Vec<_>>().join(" ");
            let (relation, target) = split_relation(&target);
            if !target.is_empty() {
                out.push(Wikilink { relation, target });
            }
        }
    }
    out
}

/// First byte offset in `body` where the text, lowercased, begins with `needle`.
/// The offset is valid in `body` itself: case folding can change byte lengths.
fn find_ci(body: &str, needle: &str) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }
    body.char_indices().find_map(|(i, _)| {
        // Lowercase the tail lazily, char by char (one char can fold to several),
        // and compare against the needle — matching when the needle runs out.
        let mut folded = body[i..].chars().flat_map(char::to_lowercase);
        let mut want = needle.chars();
        loop {
            match (folded.next(), want.next()) {
                (_, None) => return Some(i),
                (Some(a), Some(b)) if a == b => {}
                _ => return None,
            }
        }
    })
}

/// ~160-char window around a byte position, clamped to char boundaries.
fn snippet_around(body: &str, pos: usize, match_len: usize) -> String {
    let mut start = pos.saturating_sub(80);
    while start > 0 && !body.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = (pos + match_len + 80).min(body.len());
    while end < body.len() && !body.is_char_boundary(end) {
        end += 1;
    }
    let mut s = body[start..end].replace('\n', " ");
    if start > 0 {
        s = format!("…{s}");
    }
    if end < body.len() {
        s.push('…');
    }
    // Rendered here rather than on the wire like a description: a snippet has no
    // other use.
    render_inline(&s)
}

/// Serialize a markdown field as inline HTML — see [`MemoryMeta::description`].
fn as_inline_html<S: serde::Serializer>(md: &str, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&render_inline(md))
}

/// Render a fragment of corpus markdown as INLINE HTML: block structure
/// contributes no markup, and links are unwrapped to their text — a second link
/// inside a two-line preview is an ambiguous tap target. Parsed by comrak:
/// `project_kubes_dhall_model` must not become emphasis at its underscores.
pub fn render_inline(md: &str) -> String {
    let options = markdown_options();
    let arena = Arena::new();
    let root = parse_document(&arena, md, &options);
    let mut out = String::new();
    inline_html(root, &options, &mut out);
    out.trim().to_string()
}

/// Walk blocks and links transparently; render everything else as it is.
fn inline_html<'a>(node: &'a comrak::nodes::AstNode<'a>, options: &Options, out: &mut String) {
    for child in node.children() {
        let (block, transparent) = {
            let value = &child.data.borrow().value;
            (
                value.block(),
                value.block()
                    || matches!(
                        value,
                        NodeValue::Link(_) | NodeValue::WikiLink(_) | NodeValue::Image(_)
                    ),
            )
        };
        if !transparent {
            let _ = format_html(child, options, out);
            continue;
        }
        let before = out.len();
        inline_html(child, options, out);
        // One block running into the next would join two sentences into one word.
        if block && out.len() > before && !out.ends_with(' ') {
            out.push(' ');
        }
    }
}

pub(crate) fn markdown_options() -> Options<'static> {
    let mut options = Options::default();
    options.extension.table = true;
    options.extension.strikethrough = true;
    options.extension.tasklist = true;
    options.extension.autolink = true;
    options.extension.wikilinks_title_after_pipe = true;
    // Raw HTML in bodies renders escaped, not omitted.
    options.render.escape = true;
    options
}

/// If `url` is a plain relative link to a corpus file (`foo.md`), its stem.
fn md_link_stem(url: &str) -> Option<&str> {
    if url.contains(':') || url.contains('/') {
        return None;
    }
    url.strip_suffix(".md")
}

/// Render corpus markdown to HTML, rewriting `[[name]]` wikilinks and
/// relative `(file.md)` links (MEMORY.md style) to the SPA route `/m/<name>`.
pub fn render_markdown(md: &str) -> Result<String> {
    let options = markdown_options();
    let arena = Arena::new();
    let root = parse_document(&arena, md, &options);
    for node in root.descendants() {
        let mut data = node.data.borrow_mut();
        match &mut data.value {
            NodeValue::WikiLink(wl) => {
                let (relation, target) = split_relation(&wl.url);
                match relation {
                    None => wl.url = format!("/m/{target}"),
                    // A typed link becomes an ordinary link with the relation as its `title`:
                    // "governs:project_x" mid-sentence puts structure into the wording.
                    Some(relation) => {
                        data.value = NodeValue::Link(Box::new(NodeLink {
                            url: format!("/m/{target}"),
                            title: relation,
                        }));
                        // The label still carries the prefix comrak parsed.
                        drop(data);
                        for child in node.children() {
                            let mut cd = child.data.borrow_mut();
                            if let NodeValue::Text(text) = &mut cd.value {
                                *text = target.clone().into();
                                break;
                            }
                        }
                        continue;
                    }
                }
            }
            NodeValue::Link(link) => {
                if let Some(stem) = md_link_stem(&link.url) {
                    link.url = format!("/m/{stem}");
                }
            }
            _ => {}
        }
    }
    let mut out = String::new();
    format_html(root, &options, &mut out).context("rendering markdown")?;
    Ok(out)
}

/// Who links to each memory, over the whole corpus, computed once — [`homes_for`]
/// re-derived it per target and a small corpus took a minute. The fix is the
/// shape, not a cache.
pub fn incoming_links(docs: &BTreeMap<String, MemoryDoc>) -> BTreeMap<String, BTreeSet<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (name, doc) in docs {
        for link in &doc.links {
            out.entry(link.target.clone())
                .or_default()
                .insert(name.clone());
        }
    }
    out
}

/// The memories that link to `target` and are themselves reachable. Takes the
/// map from [`incoming_links`], built once and asked many times.
pub fn homes_for(
    incoming: &BTreeMap<String, BTreeSet<String>>,
    target: &str,
    reached: &BTreeSet<String>,
) -> Vec<String> {
    incoming
        .get(target)
        .into_iter()
        .flatten()
        .filter(|name| name.as_str() != target && reached.contains(*name))
        .cloned()
        .collect()
}

/// The bytes a memory's entry spends in the index — what demoting it recovers.
/// The entry, not the line: a line lists dozens of memories, and charging each
/// the whole line claimed a saving of 20,266 bytes from a 20,411-byte file.
pub fn index_entry_cost(index_md: &str, name: &str) -> usize {
    let Some(link) = index_md.find(&format!("]({name}.md)")) else {
        return 0;
    };
    // Back to the `[` that opens this entry's teaser.
    let Some(open) = index_md[..link].rfind('[') else {
        return 0;
    };
    let close = link + format!("]({name}.md)").len();
    // Plus the `, ` that joins it to its neighbour.
    (close - open) + 2
}
