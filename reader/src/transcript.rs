//! Where a conversation is on disk, and what its lines are called.
//!
//! Deliberately small, and deliberately not a reader. Both crates walk
//! `~/.claude/projects`, and both must, because they want different things from
//! it: the console finds one conversation by id and reads the last few kilobytes
//! of it, while the viewer walks every transcript in the tree including the
//! nested ones a session dispatched. Neither walk is the other with a flag, so
//! what is shared here is the *knowledge* they both need and not a function that
//! would have to serve both badly.

use std::collections::HashMap;
use std::path::Path;

/// Whether this path is a conversation, as opposed to the directory beside it.
///
/// ⚠ **The extension is the whole rule, and leaving it out has already cost a
/// session.** Claude Code files a transcript as `<id>.jsonl` and may put a
/// DIRECTORY named `<id>` right beside it, holding `subagents/` and
/// `tool-results/`. A directory's file stem is its whole name, so anything
/// matching on the stem finds the directory first whenever `read_dir` happens to
/// return it first — which is a coin toss, and made a first regression test pass
/// under ablation.
///
/// Everything downstream then reads a directory as a conversation and gets
/// nothing, **and nothing anywhere reports an error**, because "no events" is a
/// legitimate answer for a session that has just started. Seen live: a resumed
/// 119 MB conversation opened with no history, no name and 0 exchanges, while
/// its transcript sat in the same directory.
///
/// The viewer had required the extension for months; the console had not, and
/// shipped the bug with the knowledge one module away. That is the argument for
/// this crate in one function.
pub fn is_transcript(path: &Path) -> bool {
    path.extension().and_then(|ext| ext.to_str()) == Some("jsonl")
}

/// A line type that carries what a conversation calls itself.
pub struct NameLine {
    /// The `type` a transcript line declares.
    pub line_type: &'static str,
    /// The field on that line holding the name.
    pub field: &'static str,
}

/// What a session was called by whoever started it — enrolment, or
/// `--remote-control <name>`.
pub const AGENT_NAME: NameLine = NameLine {
    line_type: "agent-name",
    field: "agentName",
};

/// What the conversation was renamed to by hand, after the fact.
pub const CUSTOM_TITLE: NameLine = NameLine {
    line_type: "custom-title",
    field: "customTitle",
};

/// The opening of the JSON object for a name line, for a byte scan.
///
/// Built rather than written out, so the two crates cannot drift on the spelling
/// of a field — which is the failure this module exists to prevent, one level
/// down from [`is_transcript`].
pub fn name_needle(line: &NameLine) -> Vec<u8> {
    format!(r#"{{"type":"{}","{}":""#, line.line_type, line.field).into_bytes()
}

/// The order to read a name in when the question is **which conversation is
/// this**, for a list somebody picks from.
///
/// The title first, because that is what a person last chose to call it.
pub const AS_CONVERSATION: [&NameLine; 2] = [&CUSTOM_TITLE, &AGENT_NAME];

/// The order to read a name in when the question is **who did this work**.
///
/// The agent name first, because that is the identity the work was done under;
/// a title is what one view calls it.
pub const AS_ACTOR: [&NameLine; 2] = [&AGENT_NAME, &CUSTOM_TITLE];

// ⚠ **THE TWO CRATES DISAGREED, AND THE ANSWER IS THAT BOTH WERE RIGHT.** The
// console preferred `custom-title`, the viewer `agent-name`, each with a
// confident rationale, and the rationales were opposite. Resolved 2026-08-07 by
// reading the CLI rather than by choosing: **it carries both orders, split by
// what the name is for.** From the 2.1.221 binary —
//
//     the session labeller: agentName || customTitle || aiTitle || summary
//                           || firstPrompt || … || sessionId.slice(0, 8)
//     the resume picker   : customTitle || aiTitle || lastPrompt || summaryHint
//                           || firstPrompt          (agentName is never consulted)
//
// So the disagreement was this distinction, discovered twice and named nowhere.
// The console lists conversations to pick between, which is the picker's
// question; `/agents` says who works where, which is the labeller's. Each keeps
// the behaviour it already had, and the order is now a stated decision instead
// of two independent guesses that happened to agree.
//
// ⚠ **`ai-title` is deliberately in neither.** It is the CLI's own description of
// a conversation — "Review DICOM scan documentation" — written once near the head
// of the file and never changed. Acceptable as a caption; wrong as a name on a
// page about who did the work. The actor chain falls through to the session id
// instead.
//
// Measured on the live corpus while deciding: 13 of 13 conversations carry both
// line types and **none disagree at the end**, because the CLI writes both on
// adjacent lines. But 6 of the 13 have been renamed at least once — one four
// times, one five — so the agreement is the CLI's doing rather than luck, and the
// precedence still has to be right for the day a single mechanism writes one of
// them. In one file `agent-name` had taken a value `custom-title` never did: that
// file's `ai-title`.

// ---------------------------------------------------------------------------
// Structure: whether a transcript is intact, as opposed to merely readable.
// ---------------------------------------------------------------------------

/// Whether this path is a *conversation*, and not merely something `.jsonl`.
///
/// ⚠ **Stricter than [`is_transcript`], and both are correct.** That one tests
/// the extension alone, which is what a viewer wants: it walks the whole tree,
/// takes what it understands and shrugs at the rest. A checker cannot shrug. A
/// session's sidecar directory holds `subagents/workflows/wf_*/journal.jsonl`,
/// a different format with its own `started` / `result` line types and no uuid
/// anywhere, and feeding those to the rules below reported 1,052 violations
/// that were not defects — a different file being read as the wrong thing.
///
/// A transcript is named for the session it records, so the name is the test.
pub fn is_conversation(path: &Path) -> bool {
    is_transcript(path)
        && path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .is_some_and(is_uuid)
}

/// The canonical 8-4-4-4-12 form, lowercase.
///
/// Hand-written rather than a regex so this crate stays a leaf; see the crate
/// doc for why its dependency list is a decision and not a convenience.
pub fn is_uuid(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 36 {
        return false;
    }
    bytes.iter().enumerate().all(|(i, &b)| {
        if matches!(i, 8 | 13 | 18 | 23) {
            b == b'-'
        } else {
            b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
        }
    })
}

/// A line that belongs to the conversation and therefore carries identity.
///
/// Measured over 1.29M lines: every one of these ALWAYS has a `uuid` and
/// ALWAYS has a `parentUuid` field, with no exception in either direction.
pub const CONVERSATION_TYPES: [&str; 4] = ["assistant", "user", "attachment", "system"];

/// A line that describes the conversation from outside it, and never carries
/// identity — no `uuid`, no `parentUuid`, in 1.29M lines.
///
/// ⚠ Sixteen types exist, not fifteen. A survey that found fifteen missed
/// `pr-link` entirely, and an unknown type is indistinguishable from a corrupt
/// one, so the omission would have been reported as damage.
pub const METADATA_TYPES: [&str; 12] = [
    "last-prompt",
    "permission-mode",
    "bridge-session",
    "mode",
    "queue-operation",
    "ai-title",
    "agent-name",
    "custom-title",
    "file-history-snapshot",
    "file-history-delta",
    "pr-link",
    "frame-link",
];

/// Present on EVERY conversation line, whatever its type — 942,556 of 942,556.
///
/// A line missing one of these is not merely sparse: the writer that produced it
/// was not the writer that produced the rest of the corpus.
pub const REQUIRED_ON_CONVERSATION: [&str; 7] = [
    "sessionId",
    "timestamp",
    "cwd",
    "version",
    "isSidechain",
    "userType",
    "gitBranch",
];

/// The types that carry a `message`, exactly.
///
/// `user` and `assistant` always have one; `system` and `attachment` never do.
/// Both halves are absolute, so either a missing message or a surprising one is
/// a fault.
pub const MESSAGE_TYPES: [&str; 2] = ["user", "assistant"];

/// The only type that ever carries a `promptId`.
///
/// ⚠ It is NOT required even there — a handful of `user` lines lack it, so only
/// the converse is a rule. `couse` inherits this field down the parent chain
/// precisely because it is sparse, which is what makes link integrity load
/// bearing for a published number rather than merely tidy.
pub const PROMPT_ID_TYPE: &str = "user";

/// The only two types ever observed starting a chain.
///
/// An `assistant` with no parent does not occur once in 536,429 assistant
/// lines, so one appearing means something severed the chain above it rather
/// than that a conversation began there.
pub const ROOTABLE_TYPES: [&str; 2] = ["user", "system"];

/// What was wrong with a line, or with the file as a whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    /// Not valid JSON. See [`Tail`] for the one case where this is tolerated.
    Unparseable,
    /// Valid JSON, but not an object.
    NotAnObject,
    /// A `type` this vocabulary does not contain.
    UnknownType,
    /// A conversation line with no `uuid`, which cannot be placed in the tree.
    MissingUuid,
    /// A `uuid` that is not a uuid.
    MalformedUuid,
    /// A conversation line with no `parentUuid` key at all.
    ///
    /// ⚠ Distinct from a `parentUuid` of `null`, and conflating the two is not
    /// hypothetical: doing so once reported 81,062 roots where there are 3,260,
    /// and 349,636 broken links where there were three.
    MissingParentField,
    /// A type that is never a root, appearing as one.
    UnrootableTypeAtRoot,
    /// A metadata line carrying identity it should not have.
    MetadataWithUuid,
    /// A metadata line carrying a parent it should not have.
    MetadataWithParent,
    /// A `parentUuid` that is neither a string nor `null`.
    NonStringParent,
    /// One uuid used for two different kinds of thing.
    ///
    /// A uuid is re-emitted constantly and may move to a new parent (432 do,
    /// which is how an edited turn is recorded), but never changes its `type`:
    /// 0 of 309,290 repeat events.
    UuidTypeChange,
    /// A `parentUuid` naming a `uuid` that is not in the file. **The message it
    /// pointed at is gone.**
    DanglingParent,
    /// A parent chain that returns to itself.
    Cycle,
    /// A conversation line missing a field every one of them carries.
    MissingField,
    /// A line carrying a field its type never carries.
    UnexpectedField,
    /// A line claiming to belong to a different conversation than the file does.
    ///
    /// 0 of 942,556 lines disagree with their filename, which makes this the
    /// check that a *rewritten* transcript has to pass: a copy taken under a new
    /// session id and not restamped says, in every line, where it came from.
    SessionMismatch,
    /// `message.role` disagreeing with the line's own `type`.
    RoleMismatch,
    /// The final line was incomplete, on a file that is still being written.
    ///
    /// Reported rather than swallowed — see [`Tail::MayBeIncomplete`].
    IncompleteTail,
}

impl Rule {
    /// The stable name, for output a person or a script reads.
    pub fn name(self) -> &'static str {
        match self {
            Rule::Unparseable => "unparseable",
            Rule::NotAnObject => "not-an-object",
            Rule::UnknownType => "unknown-type",
            Rule::MissingUuid => "missing-uuid",
            Rule::MalformedUuid => "malformed-uuid",
            Rule::MissingParentField => "missing-parent-field",
            Rule::UnrootableTypeAtRoot => "unrootable-type-at-root",
            Rule::MetadataWithUuid => "metadata-with-uuid",
            Rule::MetadataWithParent => "metadata-with-parent",
            Rule::NonStringParent => "non-string-parent",
            Rule::UuidTypeChange => "uuid-type-change",
            Rule::DanglingParent => "dangling-parent",
            Rule::Cycle => "cycle",
            Rule::MissingField => "missing-field",
            Rule::UnexpectedField => "unexpected-field",
            Rule::SessionMismatch => "session-mismatch",
            Rule::RoleMismatch => "role-mismatch",
            Rule::IncompleteTail => "incomplete-tail",
        }
    }

    /// Whether this means the file is damaged.
    ///
    /// Only [`Rule::IncompleteTail`] is not: it means the file is *alive*, and
    /// the same bytes read a moment later will be whole.
    pub fn is_damage(self) -> bool {
        self != Rule::IncompleteTail
    }
}

/// One thing wrong, located.
#[derive(Debug, Clone)]
pub struct Violation {
    /// 1-indexed, counting every newline-terminated record including blanks.
    pub line: usize,
    pub rule: Rule,
    pub detail: String,
}

/// Whether the last line may be half-written.
///
/// ⚠ **This is the ONLY concession to leniency, and it exists because of a
/// race, not because a viewer should be forgiving.** Claude Code appends to a
/// transcript while we read it — open, append, close, per line, holding no
/// descriptor between — so a read can catch a record with its newline not yet
/// written. Being strict about that would fail a file that is perfectly well
/// formed a millisecond later.
///
/// It is deliberately narrow. It applies to the FINAL line only, only when the
/// file does not end in a newline, and it still produces a
/// [`Rule::IncompleteTail`] so nothing is silently dropped. A bad line anywhere
/// else is damage no matter how live the file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tail {
    /// Nothing is writing to this file. Every line must be whole.
    MustBeComplete,
    /// A session may be appending right now.
    MayBeIncomplete,
}

/// Read a whole transcript and report everything wrong with it.
///
/// Takes bytes rather than a path: this crate touches no filesystem, and the
/// two callers read at such different scales that neither should inherit the
/// other's idea of how much to load.
///
/// Resolution is checked only once the whole file has been read, because a
/// parent may be written after its child and file order is not something to
/// assume.
/// The conversation a file claims to be, when the caller knows it.
///
/// Passed in rather than read, because this crate touches no filesystem — the
/// binary that opened the file is the one that knows what it is called. `None`
/// skips [`Rule::SessionMismatch`] and checks everything else.
pub type Session<'a> = Option<&'a str>;

pub fn check(bytes: &[u8], tail: Tail, session: Session<'_>) -> Vec<Violation> {
    let mut found = Vec::new();
    let mut uuids: HashMap<String, &'static str> = HashMap::new();
    let mut parent_of: HashMap<String, String> = HashMap::new();
    let mut edges: Vec<(usize, String, String)> = Vec::new();

    let ends_clean = bytes.last() == Some(&b'\n');
    let records: Vec<&[u8]> = bytes.split(|&c| c == b'\n').collect();
    // A trailing newline yields one empty final element that is not a record.
    let count = if ends_clean && !records.is_empty() {
        records.len() - 1
    } else {
        records.len()
    };

    for (index, raw) in records.iter().take(count).enumerate() {
        let line = index + 1;
        if raw.is_empty() {
            continue;
        }
        let last = index + 1 == count;

        let value: serde_json::Value = match serde_json::from_slice(raw) {
            Ok(value) => value,
            Err(err) => {
                let torn = last && !ends_clean && tail == Tail::MayBeIncomplete;
                found.push(Violation {
                    line,
                    rule: if torn {
                        Rule::IncompleteTail
                    } else {
                        Rule::Unparseable
                    },
                    detail: err.to_string(),
                });
                continue;
            }
        };
        let Some(object) = value.as_object() else {
            found.push(Violation {
                line,
                rule: Rule::NotAnObject,
                detail: String::new(),
            });
            continue;
        };

        let kind = object.get("type").and_then(|t| t.as_str()).unwrap_or("");
        let conversation = CONVERSATION_TYPES.iter().find(|t| **t == kind);
        let metadata = METADATA_TYPES.contains(&kind);
        if conversation.is_none() && !metadata {
            found.push(Violation {
                line,
                rule: Rule::UnknownType,
                detail: format!("{kind:?}"),
            });
            continue;
        }

        let uuid = object.get("uuid").and_then(|u| u.as_str());
        let parent_field = object.get("parentUuid");

        if let Some(kind) = conversation {
            for field in REQUIRED_ON_CONVERSATION {
                if !object.contains_key(field) {
                    found.push(Violation {
                        line,
                        rule: Rule::MissingField,
                        detail: format!("{kind} has no {field}"),
                    });
                }
            }
            if let (Some(expected), Some(claimed)) =
                (session, object.get("sessionId").and_then(|s| s.as_str()))
                && claimed != expected
            {
                found.push(Violation {
                    line,
                    rule: Rule::SessionMismatch,
                    detail: format!("line says {claimed}, file is {expected}"),
                });
            }

            let carries_message = MESSAGE_TYPES.contains(kind);
            match (carries_message, object.get("message")) {
                (true, None) => found.push(Violation {
                    line,
                    rule: Rule::MissingField,
                    detail: format!("{kind} has no message"),
                }),
                (false, Some(_)) => found.push(Violation {
                    line,
                    rule: Rule::UnexpectedField,
                    detail: format!("{kind} carries a message"),
                }),
                _ => {}
            }
            // The role is stated twice, in the line's `type` and inside its
            // message, and the two have never disagreed. Two spellings of one
            // fact are worth checking against each other precisely because
            // nothing forces them to agree.
            if let Some(role) = object
                .get("message")
                .and_then(|m| m.get("role"))
                .and_then(|r| r.as_str())
                && role != *kind
            {
                found.push(Violation {
                    line,
                    rule: Rule::RoleMismatch,
                    detail: format!("type={kind}, role={role}"),
                });
            }
            if *kind != PROMPT_ID_TYPE && object.contains_key("promptId") {
                found.push(Violation {
                    line,
                    rule: Rule::UnexpectedField,
                    detail: format!("{kind} carries a promptId"),
                });
            }

            match uuid {
                None => found.push(Violation {
                    line,
                    rule: Rule::MissingUuid,
                    detail: format!("type={kind}"),
                }),
                Some(uuid) if !is_uuid(uuid) => found.push(Violation {
                    line,
                    rule: Rule::MalformedUuid,
                    detail: format!("{uuid:?}"),
                }),
                Some(_) => {}
            }
            match parent_field {
                None => found.push(Violation {
                    line,
                    rule: Rule::MissingParentField,
                    detail: format!("type={kind}"),
                }),
                Some(serde_json::Value::Null) if !ROOTABLE_TYPES.contains(kind) => {
                    found.push(Violation {
                        line,
                        rule: Rule::UnrootableTypeAtRoot,
                        detail: format!("{kind} has no parent"),
                    });
                }
                Some(serde_json::Value::Null) => {}
                Some(serde_json::Value::String(_)) => {}
                Some(other) => found.push(Violation {
                    line,
                    rule: Rule::NonStringParent,
                    detail: other.to_string(),
                }),
            }

            if let Some(uuid) = uuid {
                match uuids.get(uuid) {
                    Some(seen) if seen != kind => found.push(Violation {
                        line,
                        rule: Rule::UuidTypeChange,
                        detail: format!("{uuid} was {seen:?}, now {kind:?}"),
                    }),
                    Some(_) => {}
                    None => {
                        uuids.insert(uuid.to_string(), kind);
                    }
                }
                if let Some(parent) = parent_field.and_then(|p| p.as_str()) {
                    edges.push((line, uuid.to_string(), parent.to_string()));
                    parent_of
                        .entry(uuid.to_string())
                        .or_insert_with(|| parent.to_string());
                }
            }
        } else {
            if uuid.is_some() {
                found.push(Violation {
                    line,
                    rule: Rule::MetadataWithUuid,
                    detail: format!("type={kind}"),
                });
            }
            if parent_field.is_some() {
                found.push(Violation {
                    line,
                    rule: Rule::MetadataWithParent,
                    detail: format!("type={kind}"),
                });
            }
        }
    }

    for (line, uuid, parent) in &edges {
        if !uuids.contains_key(parent) {
            found.push(Violation {
                line: *line,
                rule: Rule::DanglingParent,
                detail: format!("{uuid} -> {parent} (no such uuid)"),
            });
        }
    }

    found.extend(cycles(&parent_of));
    found.sort_by_key(|violation| violation.line);
    found
}

/// Every parent chain that returns to itself.
///
/// Iterative rather than recursive: these chains run to hundreds of thousands
/// of nodes and recursion would exhaust the stack long before it found
/// anything. Measured on the whole corpus, there are none — which was worth
/// establishing rather than assuming, since it had never been checked.
fn cycles(parent_of: &HashMap<String, String>) -> Vec<Violation> {
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        OnThisWalk,
        Settled,
    }

    let mut marks: HashMap<&str, Mark> = HashMap::new();
    let mut found = Vec::new();

    for start in parent_of.keys() {
        if marks.contains_key(start.as_str()) {
            continue;
        }
        let mut walked: Vec<&str> = Vec::new();
        let mut at = Some(start.as_str());
        while let Some(node) = at {
            match marks.get(node) {
                Some(Mark::OnThisWalk) => {
                    let loop_len = walked.iter().rev().take_while(|n| **n != node).count() + 1;
                    found.push(Violation {
                        line: 0,
                        rule: Rule::Cycle,
                        detail: format!("{loop_len} nodes, through {node}"),
                    });
                    break;
                }
                Some(Mark::Settled) => break,
                None => {
                    marks.insert(node, Mark::OnThisWalk);
                    walked.push(node);
                    at = parent_of.get(node).map(String::as_str);
                }
            }
        }
        for node in walked {
            marks.insert(node, Mark::Settled);
        }
    }
    found
}

/// How much damage should fail this run.
///
/// ⚠ **A damaged transcript can never be repaired.** A rewrite drops a message
/// and it is gone, so a run that fails on any damage anywhere fails **forever**,
/// for every session — which is what happened: one session's transcript lost a
/// message and memview's gate became unpassable for everybody (#1062). A check
/// that cannot go green is a broken instrument, not a signal.
///
/// So inside a session only that session's OWN transcript fails it, which is the
/// one file its author could still have done something about. Outside a session
/// — `None`, the nightly — the count is reported in full; the nightly does not
/// gate on it either, it counts it into fleetwatch so the TREND is visible.
/// Same routing as `lint::passed_for_session` for the corpus, and for the same
/// reason: a shared substrate must not fail whoever happens to commit next.
pub fn fatal_damage(damaged: usize, mine: usize, session: Option<&str>) -> usize {
    match session {
        None => damaged,
        Some(_) => mine,
    }
}

/// One thing a person typed into a conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    /// The row's `uuid`, which is what deduplicates a rewritten stretch.
    pub uuid: String,
    /// When the CLI recorded it, ISO-8601.
    ///
    /// ⚠ **For a queued turn this is when it was ENQUEUED, not delivered.** The
    /// attachment repeats the enqueue's stamp inside itself while the row's own
    /// stamp is when the running turn consumed it — and the gap between them is
    /// real: 2026-08-27 saw five minutes. Taking the row's stamp would date a
    /// message to when the model got round to it.
    pub at: String,
    /// What the person typed, with the wrappers the CLI adds taken off.
    pub text: String,
    /// Whether it was typed while the session was working, and so arrived as a
    /// `queued_command` rather than a turn of its own.
    pub queued: bool,
}

/// Every human turn in a conversation, in order.
///
/// ⚠ **Five facts, each of which has cost somebody an afternoon.** They are
/// listed here because the crate owns them and callers kept re-deriving them
/// (memview#1215):
///
/// 1. **Dedupe by `uuid`, keep the FIRST.** The CLI rewrites earlier stretches
///    back into the same file, so a linear read returns the conversation twice
///    and the later copy is the degraded one.
/// 2. **A `tool_result` row carries `role: user`** and is not a human turn.
/// 3. **`isMeta` rows** are not human turns.
/// 4. **`<command-name>` wrappers and `<system-reminder>` blocks** are injected
///    into user messages and are not what the person typed.
/// 5. **A `queued_command` attachment IS a human turn.** A message typed while
///    the session is working is queued and handed to the running turn; the text
///    lives in an `attachment` row, never in a `user` one. 21,349 of them in the
///    corpus on 2026-08-27 — and reading only `user` rows that day produced a
///    confident report that three of Pippijn's messages had been LOST when they
///    had been delivered normally.
pub fn human_turns(bytes: &[u8]) -> Vec<Turn> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out = Vec::new();
    for line in bytes.split(|b| *b == b'\n') {
        let Ok(row) = serde_json::from_slice::<serde_json::Value>(line) else {
            continue;
        };
        let uuid = row["uuid"].as_str().unwrap_or_default().to_string();
        // Fact 1: the first copy wins.
        if !uuid.is_empty() && !seen.insert(uuid.clone()) {
            continue;
        }
        let turn = match row["type"].as_str() {
            Some("user") => typed_turn(&row, uuid),
            Some("attachment") => queued_turn(&row, uuid),
            _ => None,
        };
        if let Some(turn) = turn.filter(|turn| !turn.text.is_empty()) {
            out.push(turn);
        }
    }
    out
}

/// A turn the person typed when the session was idle.
fn typed_turn(row: &serde_json::Value, uuid: String) -> Option<Turn> {
    // Fact 3.
    if row["isMeta"].as_bool().unwrap_or(false) {
        return None;
    }
    let content = &row["message"]["content"];
    let text = match content {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Array(parts) => {
            // Fact 2: a result of a tool call wears the user's role.
            if parts.iter().any(|part| part["type"] == "tool_result") {
                return None;
            }
            parts
                .iter()
                .filter_map(|part| part["text"].as_str())
                .collect::<Vec<_>>()
                .join("")
        }
        _ => return None,
    };
    Some(Turn {
        uuid,
        at: row["timestamp"].as_str().unwrap_or_default().to_string(),
        text: spoken(&text),
        queued: false,
    })
}

/// A turn the person typed while the session was working.
fn queued_turn(row: &serde_json::Value, uuid: String) -> Option<Turn> {
    let attachment = &row["attachment"];
    if attachment["type"] != "queued_command" {
        return None;
    }
    let text: String = attachment["prompt"]
        .as_array()?
        .iter()
        .filter_map(|part| part["text"].as_str())
        .collect::<Vec<_>>()
        .join("");
    Some(Turn {
        uuid,
        // The enqueue's own stamp — see `Turn::at`.
        at: attachment["timestamp"]
            .as_str()
            .or_else(|| row["timestamp"].as_str())
            .unwrap_or_default()
            .to_string(),
        text: spoken(&text),
        queued: true,
    })
}

/// Fact 4: strip what the CLI wrapped round what the person said.
///
/// ⚠ **A `<command-name>` block means the person typed a SLASH COMMAND**, so the
/// turn is not dropped — the command is what they said. Only the machinery
/// around it goes.
fn spoken(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("<system-reminder>") {
        out.push_str(&rest[..at]);
        match rest[at..].find("</system-reminder>") {
            Some(end) => rest = &rest[at + end + "</system-reminder>".len()..],
            // Unclosed: everything after it is the reminder's, not the person's.
            None => return out.trim().to_string(),
        }
    }
    out.push_str(rest);
    for tag in [
        "command-name",
        "command-message",
        "command-args",
        "local-command-stdout",
        "local-command-caveat",
    ] {
        out = unwrap_tag(&out, tag);
    }
    out.trim().to_string()
}

/// Replace `<tag>x</tag>` with `x`, keeping the text a slash command carries.
fn unwrap_tag(text: &str, tag: &str) -> String {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    text.replace(&open, "").replace(&close, "\n")
}
