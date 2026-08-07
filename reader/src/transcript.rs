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

// ⚠ **THE TWO CRATES DISAGREE ABOUT WHICH OF THESE WINS, and it is not yet a
// defect.** The console prefers `custom-title`, on the grounds that one is a
// decision and the other is a default. The viewer prefers `agent-name`, on the
// grounds that one is a name and the other is a caption. Both are documented
// with a confident rationale, and they are opposite.
//
// Measured on the live corpus 2026-08-06: 13 sessions carry both line types, and
// in all 13 the values are identical, because enrolment writes both. So nothing
// has diverged and no screen is wrong today.
//
// It diverges the first time a session is renamed through one mechanism only —
// and then the console and the /agents page call the same conversation different
// things, with no error anywhere. Deliberately NOT resolved by moving the
// precedence in here: which one should win is a judgement about what a name is
// for, not a fact about the file format, and picking one silently while
// refactoring is how a decision gets made by nobody.
