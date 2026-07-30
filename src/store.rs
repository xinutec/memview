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

#[derive(Debug, Deserialize, Default)]
struct FrontmatterMeta {
    #[serde(rename = "type")]
    mtype: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct Frontmatter {
    description: Option<String>,
    #[serde(default)]
    metadata: Option<FrontmatterMeta>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryMeta {
    /// Canonical id = filename stem; frontmatter `name` normally matches.
    pub name: String,
    pub description: String,
    /// user | feedback | project | reference (from metadata.type, falling
    /// back to the filename prefix).
    pub mtype: String,
    pub modified: Option<DateTime<Utc>>,
}

#[derive(Debug)]
pub struct MemoryDoc {
    pub meta: MemoryMeta,
    /// Markdown body (frontmatter stripped).
    pub body: String,
    /// The file exactly as written, frontmatter included. Kept because linting
    /// the corpus has to see what the frontmatter *says*, not only what parsing
    /// it produced — a `name:` that disagrees with the filename is invisible
    /// once the parse has already preferred the filename.
    pub raw: String,
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
            let mtype = fm
                .metadata
                .and_then(|m| m.mtype)
                .unwrap_or_else(|| name.split('_').next().unwrap_or("other").to_string());
            let modified = entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .map(DateTime::<Utc>::from);
            // Canonical id is the filename stem; frontmatter `name` normally
            // agrees and is not trusted to (a mismatch shouldn't hide a file).
            docs.insert(
                name.clone(),
                MemoryDoc {
                    meta: MemoryMeta {
                        name,
                        description: fm.description.unwrap_or_default(),
                        mtype,
                        modified,
                    },
                    body: body.to_string(),
                    raw: text.clone(),
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

    pub fn search(&self, query: &str) -> Vec<SearchHit> {
        let q = query.to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        let mut hits: Vec<SearchHit> = self
            .docs
            .values()
            .filter_map(|d| {
                let name_hit = d.meta.name.to_lowercase().contains(&q);
                let desc_hit = d.meta.description.to_lowercase().contains(&q);
                // Offset into the ORIGINAL body: `body.to_lowercase().find()` would
                // return an offset into the lowercased copy, which drifts wherever
                // lowercasing changes a char's byte length (e.g. 'İ' 2B → "i̇" 3B) —
                // then snippet_around would window the wrong place.
                let body_pos = find_ci(&d.body, &q);
                if !name_hit && !desc_hit && body_pos.is_none() {
                    return None;
                }
                let score =
                    (name_hit as u32) * 4 + (desc_hit as u32) * 2 + body_pos.is_some() as u32;
                Some(SearchHit {
                    meta: d.meta.clone(),
                    snippet: body_pos.map(|p| snippet_around(&d.body, p, q.len())),
                    score,
                })
            })
            .collect();
        hits.sort_by(|a, b| b.score.cmp(&a.score).then(a.meta.name.cmp(&b.meta.name)));
        hits
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
            for link in wikilinks(&doc.body) {
                // Mentioning a memory twice is still one relationship, and a
                // memory linking itself is not a relationship at all. A typed
                // mention beats an untyped one for the same pair, so a body that
                // says `[[x]]` in passing and `[[governs:x]]` where it means it
                // reports the claim rather than whichever came first.
                if link.target == doc.meta.name || !self.docs.contains_key(&link.target) {
                    continue;
                }
                if !seen.insert(link.target.clone()) {
                    if let Some(relation) = link.relation
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
                    target: link.target,
                    relation: link.relation,
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
    score: u32,
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
    s
}

fn markdown_options() -> Options<'static> {
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
