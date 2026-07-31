//! What every Claude session actually worked on, mined from the transcripts.
//!
//! The question this answers is "who made heatcam, and when" — and the reason
//! it needs mining at all is that the obvious method gives a wrong answer.
//! `MEMORY.md` names every project, and the index is injected into every
//! session's context, so grepping the transcripts for a project name matches
//! essentially every session ever run. The signal here is `cwd`: where a
//! session actually *was*. No amount of injected context can fake it.
//!
//! Sibling of [`crate::couse`], and deliberately a separate artefact. Co-use
//! answers "which memories travel together"; this answers "which work happened
//! where, when, and at whose hands". They share a scanning technique and
//! nothing else.
//!
//! **Only the reader's own prompts are indexed** — never assistant replies,
//! never tool results. Chiefly because that is where the signal is: what was
//! ASKED is what a person searches for, while tool output is three gigabytes of
//! file contents and command noise that would bury every real hit and take the
//! artefact from ~13 MB to the size of the transcripts themselves.
//!
//! It has a second, smaller benefit — tool output is where a credential can
//! surface (a `cat` of a config, an env dump) and a person's own words rarely
//! carry one. That is a bonus, not the reason: isis is a TRUSTED host. The
//! threat model for this fleet is losing data, not disclosing it, which is what
//! the one-way VPN and the Mac-side backup exist for.
//!
//! The API is nonetheless owner-only, and that is a separate argument: a share
//! token is a deliberate public surface, so handing someone a link to one
//! memory must not also hand them fourteen thousand prompts.
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// A named Claude session, as the live registry knows it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// The transcript's UUID — its filename stem.
    pub id: String,
    /// The name the reader gave it ("home", "recall"), or the id when unnamed.
    ///
    /// Read from `~/.claude/sessions/<pid>.json`, which is the live registry
    /// and therefore current. The `The user named this session "…"` reminder in
    /// the transcript is written once at naming time, so a later rename leaves
    /// it stale. The registry only covers sessions the machine still knows
    /// about, hence the fallback.
    pub name: String,
    pub first: String,
    pub last: String,
    pub turns: usize,
}

/// One session's share of one project.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hand {
    /// Index into [`History::sessions`].
    pub session: usize,
    pub turns: usize,
    pub first: String,
    pub last: String,
}

/// A directory under `~/Code` that sessions worked in.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
    pub turns: usize,
    pub first: String,
    pub last: String,
    /// Who worked on it, most turns first — the answer to "who made this".
    pub hands: Vec<Hand>,
    /// Distinct files touched by a tool call, capped; see [`MAX_FILES`].
    pub files: Vec<String>,
    /// Days with at least one turn, for a timeline.
    pub days: Vec<String>,
}

/// One request and what it was about.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Turn {
    /// Index into [`History::sessions`].
    pub session: usize,
    /// Index into [`History::projects`], absent when the cwd was outside `~/Code`.
    pub project: Option<usize>,
    pub at: String,
    /// The reader's own words, cleaned. Empty when the turn had no prompt text
    /// (a resumed tool loop, or a message predating `promptId`).
    pub prompt: String,
    /// What Claude SAID in this turn — its prose replies, concatenated.
    ///
    /// Indexed because it is the harder recall problem and the more common one:
    /// "I remember it saying something about the shutter, but not which session
    /// said it" cannot be answered from prompts at all. Excludes `thinking`
    /// (reasoning the reader never saw, so never what they remember) and
    /// `tool_use` (commands, not speech).
    #[serde(default)]
    pub reply: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct History {
    pub generated: String,
    pub sessions: Vec<Session>,
    pub projects: Vec<Project>,
    pub turns: Vec<Turn>,
}

/// Longest reply kept per turn, in bytes.
///
/// Larger than a prompt's cap because a reply legitimately IS long — an
/// explanation runs to thousands of characters where a request runs to ninety.
/// Still bounded: a turn that produced twenty messages is a session's worth of
/// prose, and the tail of it is not what anyone half-remembers.
const MAX_REPLY: usize = 12_000;

/// Longest prompt kept, in bytes.
///
/// Almost every prompt is a sentence or two; a handful are enormous pastes of
/// logs or a whole file. Those are exactly the ones whose text is least worth
/// searching and most expensive to carry, and truncating them keeps one bad day
/// from doubling the artefact.
const MAX_PROMPT: usize = 4000;

/// Distinct file paths kept per project.
const MAX_FILES: usize = 400;

impl History {
    pub fn load(path: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let text = serde_json::to_string(self)?;
        std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }
}

fn find_at(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    (from..=hay.len() - needle.len()).find(|&i| &hay[i..i + needle.len()] == needle)
}

/// The value of a `"key":"…"` field, as bytes, without parsing the line.
///
/// The same trick [`crate::couse`] uses and for the same reason: a full
/// `serde_json` parse of 735k lines across three gigabytes costs minutes, and
/// almost every line is answered by a substring probe. Lines that carry prompt
/// text are parsed properly — they are 2% of the total.
fn field<'a>(line: &'a [u8], key: &str) -> Option<&'a [u8]> {
    let needle = format!("\"{key}\":\"");
    let start = find_at(line, needle.as_bytes(), 0)? + needle.len();
    let end = find_at(line, b"\"", start)?;
    Some(&line[start..end])
}

fn text_field(line: &[u8], key: &str) -> Option<String> {
    field(line, key).and_then(|v| std::str::from_utf8(v).ok().map(str::to_string))
}

/// Every `"file_path":"…"` on the line — a line can carry several tool calls.
fn file_paths(line: &[u8]) -> Vec<String> {
    let needle = b"\"file_path\":\"";
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(start) = find_at(line, needle, at) {
        let from = start + needle.len();
        let Some(end) = find_at(line, b"\"", from) else {
            break;
        };
        if let Ok(path) = std::str::from_utf8(&line[from..end]) {
            out.push(path.to_string());
        }
        at = end;
    }
    out
}

/// The project a working directory belongs to: the first path element under
/// `code_root`. `None` for anywhere else, which is honest — a session sitting
/// in the home directory was not working on a project.
///
/// `code_root` is a parameter rather than a constant because this repo is
/// PUBLIC: baking one machine's home directory into the source publishes it,
/// and makes the function untestable without writing that path into a test too.
pub fn project_of(cwd: &str, code_root: &str) -> Option<String> {
    let root = code_root.trim_end_matches('/');
    let rest = cwd.strip_prefix(root)?.strip_prefix('/')?;
    let name = rest.split('/').next()?;
    (!name.is_empty()).then(|| name.to_string())
}

/// Everything that arrives as a "user" message without a person having typed it.
///
/// Measured, not guessed: on the live corpus `<task-notification>` alone
/// accounted for 2,934 turns and 1.67 MB — HALF the indexed text — because a
/// background task completing wakes the session with a message shaped exactly
/// like a prompt. Leaving those in makes a search for "completed" or an agent
/// id return hundreds of turns nobody asked for, and doubles the artefact.
const MACHINE_TAGS: [&str; 7] = [
    "system-reminder",
    "task-notification",
    "local-command-stdout",
    "local-command-caveat",
    "command-name",
    "command-message",
    "command-args",
];

/// Openings that mark a whole message as machinery rather than speech.
///
/// A compaction summary arrives as a user message and is the single worst thing
/// that can be in a search index: it restates EVERYTHING a session discussed,
/// so it matches almost any query about that session and buries the turn the
/// reader actually wanted. Measured on the live corpus — 186 of them, 1.7% of
/// prompts but **43% of all indexed text**, averaging 3,973 characters against
/// 93 for a real prompt.
const MACHINE_OPENINGS: [&str; 2] = [
    "This session is being continued from a previous conversation",
    "Caveat: The messages below were generated by the user while running local commands",
];

/// Strip the machinery around a prompt, leaving what the reader typed.
///
/// Indexing any of [`MACHINE_TAGS`] makes a search for a phrase match sessions
/// that merely had the phrase pushed into them — the same contamination that
/// makes a naive grep for a project name useless here. A turn that was ENTIRELY
/// machinery ends up with an empty prompt, which is the honest record: the
/// harness woke the session, the reader asked nothing.
pub fn clean_prompt(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(open) = rest.find('<') {
        let (before, from) = rest.split_at(open);
        out.push_str(before);
        let tag = MACHINE_TAGS
            .into_iter()
            .find(|t| from.starts_with(&format!("<{t}>")));
        match tag {
            Some(tag) => {
                let close = format!("</{tag}>");
                match from.find(&close) {
                    Some(end) => rest = &from[end + close.len()..],
                    // Unclosed: drop the remainder rather than keep a fragment
                    // of injected text that would search as if it were speech.
                    None => return out.trim().to_string(),
                }
            }
            None => {
                out.push('<');
                rest = &from[1..];
            }
        }
    }
    out.push_str(rest);
    let trimmed = out.trim();
    // Checked after stripping, because a summary is usually preceded by a
    // reminder block — testing the raw text would miss it.
    if MACHINE_OPENINGS.iter().any(|m| trimmed.starts_with(m)) {
        return String::new();
    }
    if trimmed.len() <= MAX_PROMPT {
        return trimmed.to_string();
    }
    let mut cut = MAX_PROMPT;
    while cut > 0 && !trimmed.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}…", &trimmed[..cut])
}

/// Cut a string to at most `max` bytes without splitting a character.
fn truncate_on_boundary(mut text: String, max: usize) -> String {
    if text.len() <= max {
        return text;
    }
    let mut cut = max;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    text.truncate(cut);
    text.push('…');
    text
}

/// Read `~/.claude/sessions/*.json` for the current name of each session.
pub fn session_names(dir: &Path) -> BTreeMap<String, String> {
    #[derive(Deserialize)]
    struct Row {
        #[serde(rename = "sessionId")]
        session_id: Option<String>,
        name: Option<String>,
    }
    let mut names = BTreeMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return names;
    };
    for entry in entries.flatten() {
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        if let Ok(row) = serde_json::from_str::<Row>(&text)
            && let (Some(id), Some(name)) = (row.session_id, row.name)
        {
            names.insert(id, name);
        }
    }
    names
}

/// What one turn accumulated while scanning.
#[derive(Default)]
struct Building {
    at: String,
    project: Option<String>,
    prompt: String,
    reply: String,
}

/// The name a transcript says it was given, if it says so.
///
/// The fallback for a session the live registry has forgotten — the registry is
/// keyed by process id, so it only covers sessions the machine still knows
/// about, and a finished one vanishes from it. This reminder is written once at
/// naming time, which is exactly why it is the FALLBACK and not the source: a
/// later rename leaves it stale, and the registry is current.
fn named_in_transcript(text: &[u8]) -> Option<String> {
    let needle = b"named this session \\\"";
    let start = find_at(text, needle, 0)? + needle.len();
    let end = find_at(text, b"\\\"", start)?;
    std::str::from_utf8(&text[start..end])
        .ok()
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

fn scan_session(
    path: &Path,
    id: &str,
    code_root: &str,
    out: &mut Vec<(String, Building)>,
    fallback_name: &mut Option<String>,
) -> Result<Vec<String>> {
    let text = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    *fallback_name = named_in_transcript(&text);
    let mut parent_of: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();
    let mut prompt_of: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();
    // (uuid, what it contributed), resolved to turns once the tree is known.
    let mut parts: Vec<(Vec<u8>, Building)> = Vec::new();
    let mut files: Vec<String> = Vec::new();

    for line in text.split(|&c| c == b'\n') {
        if line.is_empty() {
            continue;
        }
        let Some(uuid) = field(line, "uuid") else {
            continue;
        };
        let uuid = uuid.to_vec();
        if let Some(parent) = field(line, "parentUuid") {
            parent_of.insert(uuid.clone(), parent.to_vec());
        }
        if let Some(prompt) = field(line, "promptId") {
            prompt_of.insert(uuid.clone(), prompt.to_vec());
        }
        files.extend(file_paths(line));

        let mut building = Building {
            at: text_field(line, "timestamp").unwrap_or_default(),
            project: text_field(line, "cwd")
                .as_deref()
                .and_then(|cwd| project_of(cwd, code_root)),
            prompt: String::new(),
            reply: String::new(),
        };

        // Only a user line can carry a prompt, and a tool RESULT is also typed
        // "user" — it just has an array content rather than a string. Parsing
        // is confined to those few lines; the probe below is what keeps this
        // from being a full parse of 735k lines.
        if find_at(line, b"\"type\":\"user\"", 0).is_some()
            && find_at(line, b"\"content\":\"", 0).is_some()
            && find_at(line, b"\"isMeta\":true", 0).is_none()
        {
            #[derive(Deserialize)]
            struct Row {
                message: Option<Msg>,
            }
            #[derive(Deserialize)]
            struct Msg {
                content: Option<serde_json::Value>,
            }
            if let Ok(row) = serde_json::from_slice::<Row>(line)
                && let Some(serde_json::Value::String(raw)) = row.message.and_then(|m| m.content)
            {
                building.prompt = clean_prompt(&raw);
            }
        }

        // What Claude SAID. Prefiltered on both markers so the parse is
        // confined to assistant lines that actually carry prose — a line that
        // is purely a tool call never reaches serde.
        if find_at(line, b"\"type\":\"assistant\"", 0).is_some()
            && find_at(line, b"\"type\":\"text\"", 0).is_some()
        {
            #[derive(Deserialize)]
            struct SaidRow {
                message: Option<SaidMsg>,
            }
            #[derive(Deserialize)]
            struct SaidMsg {
                content: Option<Vec<Block>>,
            }
            #[derive(Deserialize)]
            struct Block {
                // Defaulted rather than required: one block without a `type`
                // would otherwise fail the whole line's parse and silently drop
                // every reply on it.
                #[serde(rename = "type", default)]
                kind: String,
                #[serde(default)]
                text: Option<String>,
            }
            if let Ok(row) = serde_json::from_slice::<SaidRow>(line)
                && let Some(blocks) = row.message.and_then(|m| m.content)
            {
                let mut said = String::new();
                for block in blocks {
                    // `text` only. `thinking` is reasoning the reader never saw
                    // and `tool_use` is a command; neither is something anyone
                    // remembers Claude saying.
                    if block.kind == "text"
                        && let Some(t) = block.text
                    {
                        let t = t.trim();
                        if t.is_empty() {
                            continue;
                        }
                        if !said.is_empty() {
                            said.push('\n');
                        }
                        said.push_str(t);
                    }
                }
                building.reply = said;
            }
        }
        parts.push((uuid, building));
    }

    // Nearest ancestor-or-self carrying a promptId. A transcript is a TREE —
    // rewinding a prompt starts a sibling branch — so the turn has to be found
    // up the parent chain, never by carrying the last id forward in file order,
    // which would merge branches that were never in one context.
    let mut cache: HashMap<Vec<u8>, Vec<u8>> = HashMap::new();
    let mut turn_of = |uuid: &[u8]| -> Vec<u8> {
        let mut walked: Vec<Vec<u8>> = Vec::new();
        let mut cur = uuid.to_vec();
        let answer = loop {
            if let Some(hit) = cache.get(&cur) {
                break hit.clone();
            }
            if let Some(p) = prompt_of.get(&cur) {
                break p.clone();
            }
            walked.push(cur.clone());
            match parent_of.get(&cur) {
                None => break uuid.to_vec(),
                Some(p) => cur = p.clone(),
            }
        };
        for w in walked {
            cache.insert(w, answer.clone());
        }
        answer
    };

    let mut turns: BTreeMap<Vec<u8>, Building> = BTreeMap::new();
    for (uuid, part) in parts {
        let key = turn_of(&uuid);
        let slot = turns.entry(key).or_default();
        // Earliest stamp wins: a turn is dated by when it was ASKED, not by
        // when its last tool call happened to return.
        if !part.at.is_empty() && (slot.at.is_empty() || part.at < slot.at) {
            slot.at = part.at;
        }
        if slot.project.is_none() {
            slot.project = part.project;
        }
        if slot.prompt.is_empty() {
            slot.prompt = part.prompt;
        }
        // Appended, not first-wins: a turn's reply is spread over every message
        // Claude sent in it, and keeping only the first would index the opening
        // sentence and drop the explanation that follows.
        if !part.reply.is_empty() && slot.reply.len() < MAX_REPLY {
            if !slot.reply.is_empty() {
                slot.reply.push('\n');
            }
            slot.reply.push_str(&part.reply);
        }
    }
    out.extend(turns.into_values().map(|b| (id.to_string(), b)));
    Ok(files)
}

/// Mine every transcript under `dir` (recursively, one level of project dirs).
pub fn scan(
    dir: &Path,
    sessions_dir: &Path,
    code_root: &str,
    generated: String,
) -> Result<History> {
    let names = session_names(sessions_dir);
    let mut transcript_names: BTreeMap<String, String> = BTreeMap::new();
    let mut raw: Vec<(String, Building)> = Vec::new();
    let mut project_files: BTreeMap<String, Vec<String>> = BTreeMap::new();

    let mut transcripts: Vec<std::path::PathBuf> = Vec::new();
    for entry in std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .flatten()
    {
        let path = entry.path();
        if path.is_dir() {
            transcripts.extend(
                std::fs::read_dir(&path)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|e| e == "jsonl")),
            );
        } else if path.extension().is_some_and(|e| e == "jsonl") {
            transcripts.push(path);
        }
    }
    transcripts.sort();

    for path in &transcripts {
        let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let before = raw.len();
        let mut said_name = None;
        let files = scan_session(path, id, code_root, &mut raw, &mut said_name)?;
        if let Some(name) = said_name {
            transcript_names.entry(id.to_string()).or_insert(name);
        }
        // Attribute files to whichever project this transcript's turns were in.
        // A transcript can span projects, so this over-attributes when a session
        // switched directories mid-file; stated because it is a limit.
        let mut seen: BTreeMap<String, ()> = BTreeMap::new();
        for (_, b) in &raw[before..] {
            if let Some(p) = &b.project {
                seen.insert(p.clone(), ());
            }
        }
        for project in seen.keys() {
            let bucket = project_files.entry(project.clone()).or_default();
            for f in &files {
                if f.starts_with(&format!("{}/{project}/", code_root.trim_end_matches('/'))) {
                    bucket.push(f.clone());
                }
            }
        }
    }

    // Sessions, in first-seen order of activity.
    let mut session_index: BTreeMap<String, usize> = BTreeMap::new();
    let mut sessions: Vec<Session> = Vec::new();
    for (id, b) in &raw {
        let idx = *session_index.entry(id.clone()).or_insert_with(|| {
            sessions.push(Session {
                id: id.clone(),
                // Registry first (current), then what the transcript says it
                // was called, then the bare id — which at least identifies it.
                name: names
                    .get(id)
                    .or_else(|| transcript_names.get(id))
                    .cloned()
                    .unwrap_or_else(|| id.clone()),
                first: b.at.clone(),
                last: b.at.clone(),
                turns: 0,
            });
            sessions.len() - 1
        });
        let s = &mut sessions[idx];
        s.turns += 1;
        if !b.at.is_empty() {
            if s.first.is_empty() || b.at < s.first {
                s.first = b.at.clone();
            }
            if b.at > s.last {
                s.last = b.at.clone();
            }
        }
    }

    // Projects, and who worked on them.
    let mut project_index: BTreeMap<String, usize> = BTreeMap::new();
    let mut projects: Vec<Project> = Vec::new();
    let mut hands: BTreeMap<(usize, usize), Hand> = BTreeMap::new();
    let mut days: BTreeMap<usize, BTreeMap<String, ()>> = BTreeMap::new();

    for (id, b) in &raw {
        let Some(name) = &b.project else { continue };
        let pi = *project_index.entry(name.clone()).or_insert_with(|| {
            projects.push(Project {
                name: name.clone(),
                turns: 0,
                first: b.at.clone(),
                last: b.at.clone(),
                hands: Vec::new(),
                files: Vec::new(),
                days: Vec::new(),
            });
            projects.len() - 1
        });
        let si = session_index[id];
        let p = &mut projects[pi];
        p.turns += 1;
        if !b.at.is_empty() {
            if p.first.is_empty() || b.at < p.first {
                p.first = b.at.clone();
            }
            if b.at > p.last {
                p.last = b.at.clone();
            }
            days.entry(pi)
                .or_default()
                .insert(b.at[..10.min(b.at.len())].to_string(), ());
        }
        let hand = hands.entry((pi, si)).or_insert(Hand {
            session: si,
            turns: 0,
            first: b.at.clone(),
            last: b.at.clone(),
        });
        hand.turns += 1;
        if !b.at.is_empty() {
            if hand.first.is_empty() || b.at < hand.first {
                hand.first = b.at.clone();
            }
            if b.at > hand.last {
                hand.last = b.at.clone();
            }
        }
    }

    for ((pi, _), hand) in hands {
        projects[pi].hands.push(hand);
    }
    for (pi, project) in projects.iter_mut().enumerate() {
        // Most turns first: this ordering IS the answer to "who made this".
        project.hands.sort_by_key(|h| std::cmp::Reverse(h.turns));
        project.days = days
            .remove(&pi)
            .map(|d| d.into_keys().collect())
            .unwrap_or_default();
        if let Some(files) = project_files.remove(&project.name) {
            let mut unique: Vec<String> = files
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();
            unique.truncate(MAX_FILES);
            project.files = unique;
        }
    }
    projects.sort_by_key(|p| std::cmp::Reverse(p.turns));
    // Sorting invalidated the indices the turns carry, so rebuild the map.
    let remap: BTreeMap<String, usize> = projects
        .iter()
        .enumerate()
        .map(|(i, p)| (p.name.clone(), i))
        .collect();

    let turns = raw
        .into_iter()
        .map(|(id, b)| Turn {
            session: session_index[&id],
            project: b.project.as_ref().and_then(|n| remap.get(n).copied()),
            at: b.at,
            prompt: b.prompt,
            reply: truncate_on_boundary(b.reply, MAX_REPLY),
        })
        .collect();

    Ok(History {
        generated,
        sessions,
        projects,
        turns,
    })
}
