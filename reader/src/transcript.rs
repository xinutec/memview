//! Where a conversation is on disk, and what its lines are called.
//!
//! Deliberately small, and deliberately not a reader. Both crates walk
//! `~/.claude/projects`, and both must, because they want different things from
//! it: the console finds one conversation by id and reads the last few kilobytes
//! of it, while the viewer walks every transcript in the tree including the
//! nested ones a session dispatched. Neither walk is the other with a flag, so
//! what is shared here is the *knowledge* they both need and not a function that
//! would have to serve both badly.

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
