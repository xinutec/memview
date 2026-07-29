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
use comrak::nodes::NodeValue;
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
            for target in wikilink_targets(&doc.body) {
                // Mentioning a memory twice is still one relationship, and a
                // memory linking itself is not a relationship at all.
                if target == doc.meta.name
                    || !self.docs.contains_key(&target)
                    || !seen.insert(target.clone())
                {
                    continue;
                }
                *out_degree.entry(doc.meta.name.clone()).or_default() += 1;
                *in_degree.entry(target.clone()).or_default() += 1;
                edges.push(GraphEdge {
                    source: doc.meta.name.clone(),
                    target,
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
        }
    }
}

/// MEMORY.md's curated taxonomy: which `## section` indexes each memory, plus
/// the section titles in document order.
///
/// A memory linked from more than one section keeps the first — the index is
/// ordered, so the first mention is the one that classifies it. Links above the
/// first heading (the index's preamble) belong to no section.
fn index_sections(index_md: &str) -> (BTreeMap<String, String>, Vec<String>) {
    let mut section_of = BTreeMap::new();
    let mut sections: Vec<String> = Vec::new();
    let mut current: Option<String> = None;
    for line in index_md.lines() {
        if let Some(title) = line.strip_prefix("## ") {
            let title = title.trim().to_string();
            if !sections.contains(&title) {
                sections.push(title.clone());
            }
            current = Some(title);
            continue;
        }
        let Some(section) = current.as_ref() else {
            continue;
        };
        for stem in md_link_stems(line) {
            section_of.entry(stem).or_insert_with(|| section.clone());
        }
    }
    (section_of, sections)
}

/// Stems of relative `](name.md)` links on one line — the index's link form.
fn md_link_stems(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(start) = rest.find("](") {
        rest = &rest[start + 2..];
        let Some(end) = rest.find(')') else { break };
        let url = &rest[..end];
        rest = &rest[end + 1..];
        if let Some(stem) = md_link_stem(url) {
            out.push(stem.to_string());
        }
    }
    out
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
}

#[derive(Debug, Default, Serialize)]
pub struct Graph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
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

/// Extract `[[target]]` / `[[target|title]]` targets in order of appearance.
///
/// Bodies are hand-wrapped, so a wikilink can straddle a source line. comrak
/// renders that as one link, so the graph must see it as one too — internal
/// whitespace is collapsed rather than the link being skipped. A link can't
/// span a paragraph break, which also stops an unclosed `[[` from swallowing
/// the rest of the file.
fn wikilink_targets(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(start) = rest.find("[[") {
        rest = &rest[start + 2..];
        let Some(end) = rest.find("]]") else { break };
        let inner = &rest[..end];
        rest = &rest[end + 2..];
        if inner.contains("\n\n") {
            continue;
        }
        let target = inner.split('|').next().unwrap_or(inner);
        let target = target.split_whitespace().collect::<Vec<_>>().join(" ");
        if !target.is_empty() {
            out.push(target);
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
                wl.url = format!("/m/{}", wl.url);
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
