//! The memory corpus: one directory of markdown files, each with YAML
//! frontmatter (name/description/metadata.type) and a body that
//! cross-references other memories as `[[name]]`, plus a MEMORY.md index
//! whose links are `[title](file.md)`.
//!
//! Loaded fresh from disk on every request — the corpus is small (hundreds
//! of small files) and the writer is a live Claude session, so staleness
//! would be worse than the read cost. Rendering rewrites both link forms to
//! the SPA route `/m/<name>`.

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
    /// The session that wrote this memory, as the memory instructions record
    /// it. Camel-case on disk, so it is renamed rather than relied on.
    #[serde(rename = "originSessionId")]
    origin_session: Option<String>,
    /// When the memory itself says it last changed.
    ///
    /// ⚠ **Not the file's mtime, which is what this used and which is wrong by
    /// a median of 9.9 days.** mtime records a touch; measured over the whole
    /// corpus on 2026-08-27, only 129 of 647 files agreed with their own stamp
    /// within an hour, the worst was 34 days out, and 11 had an mtime EARLIER
    /// than the stamp. `memory-lint` makes an absent stamp an error and
    /// `memory-stamp` exists to maintain it, so it is the corpus's own record
    /// and the viewer had no business preferring the filesystem's (#1219).
    modified: Option<String>,
    /// When the memory was first written.
    ///
    /// ⚠ **Recovered, not observed.** It exists nowhere but the transcripts —
    /// this repo's history begins 2026-08-14 — so `memory-dated` mines it and
    /// writes it here, where it is versioned. This once said the recovery "gets
    /// less complete every day"; measured 2026-08-29 against odin's snapshots,
    /// it does not (memview#1240). Absent on a memory no surviving transcript
    /// records, which is a DETECTION gap and not a memory without a beginning;
    /// nothing falls back to an mtime, which records a touch and is wrong by a
    /// median of 9.9 days across this corpus.
    created: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct Frontmatter {
    description: Option<String>,
    /// The memory's own line in the index, written here rather than in
    /// MEMORY.md. See [`MemoryMeta::teaser`].
    teaser: Option<String>,
    #[serde(default)]
    metadata: Option<FrontmatterMeta>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryMeta {
    /// Canonical id = filename stem; frontmatter `name` normally matches.
    pub name: String,
    /// The frontmatter description, held as the markdown it is and **sent as
    /// HTML**.
    ///
    /// Rendered at the wire rather than at construction because ranking
    /// tokenises this field and linting measures it: both want the words, and
    /// neither wants `<code>` among them. Serialising is the only moment the
    /// value is for a reader, so it is the only place it is rendered — which
    /// also means every view that shows a description shows it the same way,
    /// with no second field to fall out of step.
    #[serde(serialize_with = "as_inline_html")]
    pub description: String,
    /// This memory's line in the index — the cue a reader meets in a list of
    /// three hundred, not a summary read on its own.
    ///
    /// ⚠ **Deliberately NOT `description`, which answers a different question.**
    /// A description decides relevance when it is read alone and runs to a
    /// median of 193 characters; an index teaser is read among hundreds and runs
    /// to a median of 8. Generating the index from descriptions would be ~64 KB
    /// against a 24,400-byte ceiling. Measured over the corpus 2026-09-01.
    ///
    /// **It lives with the memory so it cannot rot apart from it.** Held in
    /// MEMORY.md, a teaser described a memory that had since changed and nothing
    /// connected the two. Pippijn, 2026-09-01: "Let's make the teaser text part
    /// of the doc itself. The automation will be structural, not linguistic."
    ///
    /// Absent is meaningful, not an error: a memory with no teaser cannot be
    /// assembled into the index, which is the first signal the corpus has had
    /// about what is index-eligible (memview#822, #1310).
    pub teaser: Option<String>,
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
    /// This memory's outgoing wikilinks, parsed ONCE at load.
    ///
    /// ⚠ **Because parsing them per query was the whole cost of the memory
    /// tools.** Every graph walk — `depths_without`, `reachable_without`,
    /// `incoming_links`, the lint's link rules — used to call `wikilinks_of`,
    /// which is a full markdown parse with a fresh arena. `memory-tiers` runs
    /// four whole-corpus walks, so it paid ~2,700 parses of a few megabytes per
    /// run; `homes_for` asked per memory and paid ~446,000 (memview#1274).
    ///
    /// The corpus is SMALL. Anything here that is slow is slow because of its
    /// shape, and the fix is the shape rather than a cache.
    pub links: Vec<Wikilink>,
    /// The file exactly as written, frontmatter included. Kept because linting
    /// the corpus has to see what the frontmatter *says*, not only what parsing
    /// it produced — a `name:` that disagrees with the filename is invisible
    /// once the parse has already preferred the filename.
    pub raw: String,
    /// The session that wrote this memory (`metadata.originSessionId`), if it
    /// declares one.
    ///
    /// Deliberately NOT part of [`MemoryMeta`]. That struct is serialised into
    /// every list, backlink, outlink, search hit and graph node, all of which a
    /// share-link recipient may read — and resolving a session to the agent
    /// that owns it is exactly the roster `/api/agents` is owner-only to
    /// protect. Keeping it on the doc means the leak cannot happen by
    /// forgetting, only by writing a handler that opts in.
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
            let fm: Frontmatter = match yaml {
                Some(y) => serde_yaml::from_str(y).unwrap_or_default(),
                None => Frontmatter::default(),
            };
            let meta = fm.metadata.unwrap_or_default();
            let mtype = meta
                .mtype
                .unwrap_or_else(|| name.split('_').next().unwrap_or("other").to_string());
            // An empty value is absent: an origin that resolves to nothing is
            // worse than none, because it renders as an agent that never was.
            let origin_session = meta.origin_session.filter(|s| !s.trim().is_empty());
            // ⚠ **The stamp the memory keeps, falling back to mtime only when
            // it has none** — which `memory-lint` reports as an error, so the
            // fallback is a stopgap for a corpus mid-repair, not a second
            // opinion. See `FrontmatterMeta::modified`.
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
            // ⚠ **No mtime fallback here, unlike `modified` above.** An mtime
            // is the last touch, which for a creation date is not a worse
            // answer but a different fact — and a wrong date that looks present
            // is worse than an absent one, because nothing goes looking for it.
            let created = meta
                .created
                .as_deref()
                .and_then(|stamp| DateTime::parse_from_rfc3339(stamp).ok())
                .map(|stamp| stamp.with_timezone(&Utc));
            // Canonical id is the filename stem; frontmatter `name` normally
            // agrees and is not trusted to (a mismatch shouldn't hide a file).
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
                        mtype,
                        modified,
                        created,
                    },
                    links: wikilinks(body),
                    body: body.to_string(),
                    raw: text.clone(),
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

    /// Names of memories whose body wikilinks to `name`. Shares
    /// `wikilink_targets` with `outlinks` so the two directions of the graph
    /// can't disagree about what counts as a link.
    pub fn backlinks(&self, name: &str) -> Vec<MemoryMeta> {
        self.docs
            .values()
            .filter(|d| d.meta.name != name && wikilink_targets(&d.body).iter().any(|t| t == name))
            .map(|d| d.meta.clone())
            .collect()
    }

    /// `[[targets]]` referenced by this doc, split into existing and dangling
    /// (a dangling wikilink marks something worth writing — not an error).
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
    /// relaxed to find them.
    ///
    /// `usage` is the mined co-use artefact when there is one; it supplies a mild
    /// prior so that among comparable answers the ones the work actually leans on
    /// come first. Empty is fine and changes only the ordering.
    ///
    /// **Every term is required first, and only if that finds nothing is the
    /// query relaxed to any term** — with the fact returned, never swallowed. A
    /// search that quietly widens its own query presents loose matches as though
    /// they were what was asked for, and the reader has no way to tell.
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
                // Snippet anchored on the RAREST term the memory actually holds,
                // not on the whole query: the query as typed usually appears
                // nowhere, which is the fault this ranking exists to fix, and a
                // snippet from offset zero would show the frontmatter every time.
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

    /// The whole corpus as a link graph: one node per memory, one edge per
    /// distinct `[[wikilink]]` between two memories that both exist.
    ///
    /// Shares `wikilink_targets` with `backlinks`/`outlinks`, so the graph view
    /// and the per-memory link panels cannot disagree about what a link is.
    /// Dangling wikilinks are deliberately absent — they have no node to point
    /// at; `outlinks` is where they stay visible.
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
                // Mentioning a memory twice is still one relationship, and a
                // memory linking itself is not a relationship at all. A typed
                // mention beats an untyped one for the same pair, so a body that
                // says `[[x]]` in passing and `[[governs:x]]` where it means it
                // reports the claim rather than whichever came first.
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
            // Filled by the route when a co-use artefact is loaded; the corpus
            // alone cannot say when the mine last ran.
            as_of: None,
            usage: Default::default(),
            affinities: Default::default(),
        }
    }
}

/// Map each memory to the `## section` of MEMORY.md that indexes it, and list
/// the section titles in the order the index writes them.
///
/// Parsed with comrak, like everything else that reads corpus markdown. The
/// line-by-line scanner this replaced could not see a heading written with
/// `setext` underlining, mis-read any link whose title contained `](`, and
/// happily indexed links inside fenced code — three ways for the legend to
/// disagree with the page it is a legend for.
fn index_sections(index_md: &str) -> (BTreeMap<String, String>, Vec<String>) {
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

/// Everything a reader arrives at from the index, with `demoting` struck out of
/// it — which is the question a demotion actually asks.
///
/// ⚠ **A SET, never one name at a time, and that is the whole point.** Asking
/// per candidate answers "is this one housed *today*", and today includes every
/// other candidate's index line. Two memories that link only each other are then
/// each other's home and both look safe — until both lines go and neither is
/// reachable from anything. That is not hypothetical: `memory-rank` offered
/// exactly that pair on 2026-08-14, summed as `→ 1818 bytes if all 25 were
/// demoted` (#869), and it is the 2026-08-07 stranding of 24 memories
/// (`feedback_memory_index_is_the_working_set`) with a number attached.
///
/// Reachability is the corpus's one invariant, so it is checked in one place and
/// both callers ask it the same way: `lint` with nothing struck out, `memory-rank`
/// with the demotions it is about to recommend.
pub fn reachable_without(
    docs: &BTreeMap<String, MemoryDoc>,
    index_md: &str,
    demoting: &BTreeSet<String>,
) -> BTreeSet<String> {
    depths_without(docs, index_md, demoting)
        .into_keys()
        .collect()
}

/// How many links a reader follows from the index to arrive at each memory.
///
/// **Depth 1 is a root line's own target — DIRECTLY linked.** Depth 2 is one hop
/// beyond it, and a name absent from the map is not reachable at all.
///
/// ⚠ **This is the half of the root/traversal question nothing measured.**
/// `docs/memory.md` splits the corpus into what is present without being asked
/// for and what is reached by following a link, and every signal built for that
/// decision so far describes USE — breadth, days, roles. None describes
/// POSITION. So "consulted by fifteen agents from four hops out" and "consulted
/// by fifteen agents from one" were the same reading, though one argues for a
/// root line and the other says the traversal is already short.
///
/// ⚠ **Breadth-first, and the queue is why.** `reachable_without` used
/// `Vec::pop`, which is a STACK — correct for reachability, where any order
/// visits the same set, and wrong for distance, where a depth-first walk records
/// whichever path it wandered down first rather than the shortest. The two share
/// this walk now so they cannot disagree about what is reachable.
///
/// `demoting` is struck out first, so the depths are the ones a reader would
/// face AFTER the demotion — which is the question a trade actually asks: does
/// this line's target fall one hop, or out of the graph entirely?
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
        // First arrival wins: BFS reaches a name by its shortest path, so a
        // later, longer route must not overwrite it.
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

/// Every bold run in a document, as plain text.
///
/// Parsed, not string-matched. `**Why:**` inside a fenced example is a sample
/// of a rule, not a rule stating its reason, and a `contains()` cannot tell the
/// difference — the same class of mistake as the three hand-rolled link parsers
/// this file replaced.
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

/// Whether some bold run opens a section named `heading`.
///
/// Deliberately loose about what follows the name. The corpus writes
/// `**Why (the nixos-repo caution):**` and `**How to apply, generally.**`, and
/// both are the section this asks about — carrying a scope qualifier or ending
/// in a full stop makes them better writing, not absent ones. A checker that
/// demanded one exact byte sequence would be asking the corpus to write worse
/// prose to satisfy it, which is the wrong way round.
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
        // The name has to be a whole word: `**Why:**`, `**Why (2026-07-21):**`
        // and `**Why I was wrong.**` are all this section, while `**Whyever**`
        // is not the word at all. A bold run *opening* with the word is taken as
        // the section — anything bold that begins "Why" is announcing a reason,
        // and demanding more structure than that would only push the corpus back
        // toward one rigid phrasing.
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
    /// taxonomy, which beats anything clustering would infer. `None` when the
    /// index never links it under a heading; that is a real corpus fact, so it
    /// is reported rather than folded into a catch-all bucket.
    pub section: Option<String>,
    /// Body length in bytes. Spans ~50x across the real corpus (median ~1.9 KB,
    /// max ~97 KB), so a renderer wanting node radii should scale it log-wise.
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
    /// When the co-use mine that produced `usage` and `affinities` last ran, as
    /// the artefact itself records it. `None` when there is no artefact.
    ///
    /// ⚠ **The defect this closes is SILENCE, not staleness.** The viewer serves
    /// whatever the last mine left, and that is the right trade for a page: a
    /// per-request re-scan costs far more than a CLI run, and a reader
    /// mid-scroll should not have the picture move under them. What was wrong is
    /// that nothing said how old it was, so a graph missing yesterday's work and
    /// a graph missing nothing looked identical. Nightly mining means this is
    /// routinely hours behind — which is fine, and now legible (memview#1274).
    pub as_of: Option<String>,
    /// How much each memory is actually used, keyed by name. Empty when no
    /// co-use artefact is available — the picture degrades to structure only,
    /// rather than to nothing.
    #[serde(default)]
    pub usage: std::collections::BTreeMap<String, crate::couse::Usage>,
    /// Pairs the work keeps using together, whether or not either links the
    /// other. A second, weaker pull in the layout: 71% of these cross a region
    /// boundary drawn from the written links alone, so they move the picture
    /// rather than merely confirming it.
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
    /// BM25 score. Exposed so a ranking regression shows up in the response
    /// rather than only in how the page happens to feel.
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

/// The relations a link may declare, and what each one asserts.
///
/// A closed vocabulary on purpose. The corpus had 856 links and every one of
/// them said only "related", while doing at least five different jobs — a rule
/// governing a project, a project citing a fact, a sub-project belonging to a
/// parent. An open vocabulary would record those distinctions once each and
/// never let anything be asked of them; a fixed one can be checked, filtered and
/// counted.
///
/// Unknown prefixes are deliberately NOT tolerated: `[[superseeds:x]]` keeps the
/// typo in the target, finds no memory of that name, and shows up as a dangling
/// link. A misspelt relation silently downgrading to an untyped link would hide
/// exactly the mistake this vocabulary exists to make visible.
/// The closed vocabulary for `metadata.type`.
///
/// Closed for the same reason [`RELATIONS`] is: [`MemoryMeta::mtype`] falls back
/// to the filename prefix when the frontmatter declares nothing, so a missing or
/// misspelt type parses as a perfectly good one and nothing downstream can tell.
/// Lint checks the declared value against this list rather than the parsed one.
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

/// Split a wikilink's inner text into an optional relation and a target.
///
/// Only a prefix in [`RELATIONS`] counts. Anything else stays part of the
/// target, so it fails loudly as a dangling link rather than quietly becoming
/// an untyped one.
pub fn split_relation(inner: &str) -> (Option<String>, String) {
    if let Some((prefix, rest)) = inner.split_once(':')
        && RELATIONS.contains(&prefix)
        && !rest.is_empty()
    {
        return (Some(prefix.to_string()), rest.to_string());
    }
    (None, inner.to_string())
}

/// Extract every `[[wikilink]]`, in order of appearance.
///
/// Parsed with comrak rather than scanned for `[[`, so this and the rendered
/// HTML can never disagree about what a link is. The hand-rolled scanner this
/// replaced could not see code: a shell snippet containing `rm -rf "${x[[-n
/// "$target"]]}"`, a Lean type, a `[[la,lo,ts]]` tuple in a fenced block — all
/// three were being reported as links to memories that had never been written,
/// while comrak had correctly refused to make links of them. The bug was not
/// that the guesses were bad; it was that there were two parsers.
///
/// Bodies are hand-wrapped, so a wikilink can straddle a source line. comrak
/// renders that as one link, and now so does this.
fn wikilink_targets(body: &str) -> Vec<String> {
    wikilinks(body).into_iter().map(|l| l.target).collect()
}

/// Every `[[link]]` in the body, in order, with the relation each declares.
///
/// Public so the corpus linter sees exactly the links the viewer does — the two
/// disagreeing about what a link is would make every finding suspect.
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

/// First byte offset in `body` (original casing) where the text, lowercased,
/// begins with `needle` (already lowercased). Unlike `body.to_lowercase().find`,
/// the returned offset is valid in `body` itself — case folding can change a
/// char's byte length, so an offset into the lowercased copy can't be used to
/// slice the original.
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
    // Rendered here rather than on the wire like a description, because a
    // snippet has no other use: it is built for a reader and read once.
    render_inline(&s)
}

/// Serialize a markdown field as inline HTML — see [`MemoryMeta::description`].
fn as_inline_html<S: serde::Serializer>(md: &str, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&render_inline(md))
}

/// Render a fragment of corpus markdown as *inline* HTML.
///
/// Descriptions and search snippets are markdown like everything else — a tenth
/// of the descriptions and every body contain a code span, a bold run or a
/// wikilink — and shown raw they read as punctuation: `` `code/kubes/dhall/` ``
/// and `**How to apply:**` with the asterisks visible.
///
/// **Inline only**: block structure is walked through and contributes no markup,
/// so a snippet cut out of a list or a heading yields its words rather than a
/// stray `<li>`. And **links are unwrapped to their text**, deliberately: a
/// search hit is a navigation surface with exactly one destination, and a second
/// link inside a two-line preview is an ambiguous tap target on a phone, which
/// is what this is mostly read on.
///
/// Parsed by comrak rather than pattern-matched, because CommonMark's rules are
/// the ones that matter here: `project_kubes_dhall_model` must not turn into
/// emphasis at its underscores, and this corpus is made of such names. A marker
/// left unclosed by the truncation renders as the literal text it is.
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
        // One block running into the next would join two sentences into one
        // word. Links need no separator — they sit inside a sentence.
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
    // Raw HTML in bodies (e.g. quoted <tags> in prose) renders escaped, not
    // omitted, so the text stays visible.
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
                    // A typed link becomes an ordinary link, because a wikilink
                    // node has nowhere to put the relation — and it has to go
                    // somewhere the reader can see. As a `title` it surfaces on
                    // hover; left in the label the prose would read
                    // "governs:project_x" mid-sentence, which puts structure
                    // into the wording where it does not belong.
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

/// The reachable memories that already link `target`.
///
/// ⚠ **This is the step the 2026-08-07 pass skipped**, and skipping it is what
/// stranded memories that still existed. A demotion is only safe once something
/// live already points at the memory; a candidate with no home is not a
/// candidate, it is a deletion wearing a demotion's clothes.
/// Who links to each memory, over the whole corpus, computed once.
///
/// ⚠ **This exists because [`homes_for`] used to re-derive it per target, and a
/// small corpus was taking a minute.** Each `wikilinks_of` is a full markdown
/// parse with a fresh arena; asking it inside a per-memory loop made the cost
/// 668 x 668 = ~446,000 parses of a few megabytes. Building the reverse map once
/// is 668 parses — the same answer, three orders of magnitude less work.
///
/// The corpus is SMALL. Anything here that is slow is slow because of its shape,
/// not its size, and the fix is the shape rather than a cache.
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

/// The memories that link to `target` and are themselves reachable.
///
/// Takes the map from [`incoming_links`] rather than the corpus: the caller
/// builds it once and asks this many times.
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

/// The bytes a memory's entry spends in the index, which is what demoting it
/// recovers — and the only reason any of this is a question.
///
/// ⚠ **The entry, not the line.** A line here is a section listing dozens of
/// memories — `[cite](a.md), [not chat](b.md), …` — so charging each of them the
/// whole line overstates every one of them and, summed, claimed a saving of
/// 20,266 bytes from a 20,411-byte file. What a demotion actually recovers is
/// one `[teaser](name.md)` fragment and the `, ` that joins it to its neighbour.
pub fn index_entry_cost(index_md: &str, name: &str) -> usize {
    let Some(link) = index_md.find(&format!("]({name}.md)")) else {
        return 0;
    };
    // Back to the `[` that opens this entry's teaser; without it the label is
    // free, which is the same overstatement in the other direction.
    let Some(open) = index_md[..link].rfind('[') else {
        return 0;
    };
    let close = link + format!("]({name}.md)").len();
    // Plus the separator that goes with it — a comma and a space between
    // entries, which is what is actually reclaimed when one is removed.
    (close - open) + 2
}
