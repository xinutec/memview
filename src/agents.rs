//! Which named Claude session works on which part of the codebase.
//!
//! Several sessions run in parallel, each named for what it does — `recall`,
//! `health`, `tumor`, `observe`. That naming is a claim, and this is the
//! evidence for or against it: what each one actually opened and actually
//! changed, counted per project directory.
//!
//! **The signal is the file paths of tool calls, not `cwd` and not any text.**
//! `cwd` says where a session was started and barely moves; text is hopeless,
//! because MEMORY.md names every project and is injected into every session, so
//! grepping for a project name matches nearly everything. What a session
//! *opened* and *wrote* cannot be faked by injected context, and it is the
//! record of work rather than of intent.
//!
//! **Reads and writes are counted apart, and that distinction is the point.** A
//! session that reads a repository is consulting it; one that writes there is
//! responsible for it. `health` reads the `pippijn` monorepo more than anything
//! else while doing its writing in `health` — reporting one number would call it
//! a monorepo session, which is not what it is.
//!
//! Only names, project names and integers leave this module — the same rule the
//! rest of the mining follows, for the same reason
//! (see the module docs on [`crate::couse`]).
use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::couse::{field, find_at};

/// One named session, and where its work actually landed.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Agent {
    /// The name it goes by — "recall", "health" — or its session id when it was
    /// never named.
    pub name: String,
    /// Transcripts filed under this name. More than one when a session is
    /// resumed, or when the same name has been reused over time.
    pub transcripts: usize,
    /// Files opened, per project directory.
    pub reads: BTreeMap<String, usize>,
    /// Files written or edited, per project directory.
    pub writes: BTreeMap<String, usize>,
    /// First and last activity, ISO-8601.
    pub first: String,
    pub last: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Agents {
    /// When this was mined, ISO-8601 — the artefact's own account of its age,
    /// because an mtime records the last copy rather than the last derivation.
    #[serde(default)]
    pub generated: String,
    /// Named sessions, busiest first.
    pub agents: Vec<Agent>,
}

impl Agents {
    pub fn load(path: &Path) -> Option<Self> {
        serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        std::fs::write(path, serde_json::to_string_pretty(self)?)
            .with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }
}

/// The tools whose `file_path` counts as reading.
const READ_TOOLS: [&str; 1] = ["Read"];
/// ...and as writing. `NotebookEdit` carries `notebook_path`, not `file_path`,
/// so it is not counted here rather than counted wrongly.
const WRITE_TOOLS: [&str; 2] = ["Write", "Edit"];

/// The project a path belongs to: the first element under the code root.
///
/// `None` for anywhere else, which deliberately drops the two largest sources of
/// noise — the scratchpad under `/private/tmp`, where every session writes
/// throwaway scripts, and the memory corpus itself, which every session reads
/// and which says nothing about what any of them works on.
fn project_of(path: &str, code_root: &str) -> Option<String> {
    let root = code_root.trim_end_matches('/');
    let rest = path.strip_prefix(root)?.strip_prefix('/')?;
    let name = rest.split('/').next()?;
    (!name.is_empty()).then(|| name.to_string())
}

/// Session id → name, from the live registry at `~/.claude/sessions`.
///
/// Keyed by pid, and each entry carries `sessionId` and `name`. This is the
/// authority over the in-transcript "the user named this session" reminder,
/// which is written once and goes stale the moment a session is renamed.
pub fn registry_names(dir: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        if let (Some(id), Some(name)) = (json["sessionId"].as_str(), json["name"].as_str())
            && !name.is_empty()
        {
            out.insert(id.to_string(), name.to_string());
        }
    }
    out
}

/// The name a transcript records for itself, for sessions the registry has
/// forgotten. Stale after a rename, which is why it is only the fallback.
///
/// **The quotes are backslash-escaped**, because the reminder is prose inside a
/// JSON string: the bytes on disk read `named this session \"home\"`. Matching
/// the unescaped form finds nothing at all, and the failure is silent — every
/// session the registry has forgotten simply shows as a bare uuid, which looks
/// like an unnamed session rather than a broken parser.
///
/// First occurrence wins, which is right for the session's own reminder (it is
/// injected near the top) but is not airtight: a transcript can quote another
/// session's name later on, and this one does. Acceptable only because the
/// registry is the authority and this runs when the registry has nothing.
fn named_in_transcript(text: &[u8]) -> Option<String> {
    let needle = b"named this session ";
    let mut start = find_at(text, needle, 0)? + needle.len();
    // Skip the opening quote in whichever form it takes.
    while matches!(text.get(start), Some(b'\\') | Some(b'"')) {
        start += 1;
    }
    let end = (start..text.len()).find(|&i| text[i] == b'\\' || text[i] == b'"')?;
    let name = std::str::from_utf8(&text[start..end]).ok()?;
    (!name.is_empty() && name.len() <= 40).then(|| name.to_string())
}

/// Count one transcript's tool calls into `agent`.
fn scan_transcript(text: &[u8], code_root: &str, agent: &mut Agent) {
    for line in text.split(|&c| c == b'\n') {
        if line.is_empty() {
            continue;
        }
        if let Some(stamp) = field(line, "timestamp").and_then(|t| std::str::from_utf8(t).ok()) {
            if agent.first.is_empty() || stamp < agent.first.as_str() {
                agent.first = stamp.to_string();
            }
            if stamp > agent.last.as_str() {
                agent.last = stamp.to_string();
            }
        }
        // A line can carry more than one tool call, so every occurrence is
        // walked rather than only the first — a batched turn that opens six
        // files is six reads, and counting it as one would understate exactly
        // the sessions that work hardest.
        for (tools, counter) in [
            (READ_TOOLS.as_slice(), &mut agent.reads),
            (WRITE_TOOLS.as_slice(), &mut agent.writes),
        ] {
            for tool in tools {
                let needle = format!("\"name\":\"{tool}\",\"input\":{{\"file_path\":\"");
                let mut from = 0;
                while let Some(at) = find_at(line, needle.as_bytes(), from) {
                    let start = at + needle.len();
                    let Some(end) = find_at(line, b"\"", start) else {
                        break;
                    };
                    if let Ok(path) = std::str::from_utf8(&line[start..end])
                        && let Some(project) = project_of(path, code_root)
                    {
                        *counter.entry(project).or_insert(0) += 1;
                    }
                    from = end;
                }
            }
        }
    }
}

/// Mine every transcript under `projects_root` into per-agent directory counts.
///
/// Every project directory is walked, not just one: agents are named per
/// session and a session's transcripts live under whichever root it was started
/// in, so scoping to one root would silently lose whole agents.
pub fn scan(
    projects_root: &Path,
    sessions_dir: &Path,
    code_root: &str,
    generated: &str,
) -> Result<Agents> {
    let names = registry_names(sessions_dir);
    let mut by_name: BTreeMap<String, Agent> = BTreeMap::new();

    let roots = std::fs::read_dir(projects_root)
        .with_context(|| format!("reading {}", projects_root.display()))?;
    for root in roots.flatten() {
        if !root.path().is_dir() {
            continue;
        }
        let Ok(files) = std::fs::read_dir(root.path()) else {
            continue;
        };
        for file in files.flatten() {
            let path = file.path();
            if path.extension().is_none_or(|e| e != "jsonl") {
                continue;
            }
            let Ok(text) = std::fs::read(&path) else {
                continue;
            };
            let id = path.file_stem().unwrap_or_default().to_string_lossy();
            // Registry first, transcript second, id last. An unnamed session is
            // shown as its id rather than merged into an "unknown" bucket —
            // several distinct agents pooled under one label would be a claim
            // about the work that nothing supports.
            let name = names
                .get(id.as_ref())
                .cloned()
                .or_else(|| named_in_transcript(&text))
                .unwrap_or_else(|| id.to_string());
            let agent = by_name.entry(name.clone()).or_insert_with(|| Agent {
                name,
                ..Agent::default()
            });
            agent.transcripts += 1;
            scan_transcript(&text, code_root, agent);
        }
    }

    let mut agents: Vec<Agent> = by_name.into_values().collect();
    agents.sort_by_key(|a| {
        let total: usize = a.reads.values().sum::<usize>() + a.writes.values().sum::<usize>();
        (std::cmp::Reverse(total), a.name.clone())
    });
    Ok(Agents {
        generated: generated.to_string(),
        agents,
    })
}
