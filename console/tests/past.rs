//! Finding conversations that already happened.
//!
//! The transcripts are read rather than their filenames decoded, so these fixtures
//! are shaped like the real thing in the two ways that matter: the working
//! directory arrives a few lines in rather than on the first, and the directory a
//! transcript claims need not match the name of the folder holding it.

use std::path::Path;

use console::past::{
    Counted, conversations, counted, named as named_of, touched as touched_of, transcript_ids,
    transcript_of, words_of_claude_processes,
};

/// A wrapper shell whose *path* says claude — the shape that caused the bug.
/// Claude Code sources a snapshot under `~/.claude/` for every command it runs,
/// so every one of those command lines matches the substring "claude".
const WRAPPER: &str = "/bin/zsh -c source /home/example/.claude/shell-snapshots/snap.sh \
                       && eval 'grep -rn utterance .'";
const REAL: &str = "claude --remote-control health --resume health";

/// Write a transcript whose `cwd` line sits `depth` lines down, as the real ones
/// do — Claude Code opens with a mode line and a session id, and the working
/// directory arrives on a later `system` line.
fn transcript(dir: &Path, project: &str, id: &str, cwd: Option<&str>, depth: usize) {
    padded(dir, project, id, cwd, depth, 0);
}

/// The same, with each opening line padded to `width` bytes.
///
/// The opening lines are not uniform in the real files: a `mode` line is tens of
/// bytes and a `file-history-snapshot` is tens of kilobytes, which is the whole
/// reason the search is bounded by bytes rather than by a count of lines.
fn padded(dir: &Path, project: &str, id: &str, cwd: Option<&str>, depth: usize, width: usize) {
    let folder = dir.join(project);
    std::fs::create_dir_all(&folder).expect("project dir");
    let mut lines: Vec<String> = (0..depth)
        .map(|n| {
            format!(
                r#"{{"type":"file-history-snapshot","n":{n},"snapshot":"{}"}}"#,
                "x".repeat(width)
            )
        })
        .collect();
    if let Some(cwd) = cwd {
        lines.push(format!(r#"{{"type":"system","cwd":"{cwd}"}}"#));
    }
    lines.push(r#"{"type":"user"}"#.to_string());
    std::fs::write(folder.join(format!("{id}.jsonl")), lines.join("\n")).expect("transcript");
}

/// A transcript that names itself, the way a real one does: on repeated lines
/// near the end, because a session is renamed as its job changes.
fn named(dir: &Path, id: &str, title: Option<&str>, agent: Option<&str>) {
    let folder = dir.join("project");
    std::fs::create_dir_all(&folder).expect("project dir");
    let mut lines = vec![
        r#"{"type":"mode"}"#.to_string(),
        r#"{"type":"system","cwd":"/home/example/Code"}"#.to_string(),
    ];
    // Named early and renamed later: the later name is the one that counts.
    if let Some(agent) = agent {
        lines.push(format!(r#"{{"type":"agent-name","agentName":"{agent}"}}"#));
    }
    if let Some(title) = title {
        lines.push(format!(
            r#"{{"type":"custom-title","customTitle":"{title}"}}"#
        ));
    }
    std::fs::write(folder.join(format!("{id}.jsonl")), lines.join("\n")).expect("transcript");
}

/// A directory of this test's own.
///
/// ⚠ **`name` must be unique across the file, and nothing checks it.** Two tests
/// sharing one silently share a directory — each `remove_dir_all`s the other's
/// fixture and whichever writes last wins, so the pair passes alone and fails
/// perhaps one run in six. Cost an hour on 2026-08-07, where the reused name read
/// its neighbour's transcript and reported a conversation called `health` that
/// the test had never written.
fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("console-past-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch");
    dir
}

#[test]
fn a_transcript_is_found_by_the_directory_it_records() {
    // The folder name is deliberately not the encoded form of the cwd: the
    // encoding is undocumented, so nothing may depend on reproducing it.
    let root = scratch("records");
    transcript(
        &root,
        "some-encoded-name",
        "abc-123",
        Some("/home/example/Code/utterance"),
        3,
    );

    let found = conversations(&root);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, "abc-123");
    assert_eq!(found[0].dir, "/home/example/Code/utterance");
    assert!(found[0].bytes > 0);
}

#[test]
fn a_transcript_that_never_says_where_it_ran_is_left_out() {
    // Resuming happens in a directory. One that cannot be identified cannot be
    // resumed safely, and offering it would produce a session in the wrong place.
    let root = scratch("nowhere");
    transcript(&root, "project", "no-cwd", None, 4);

    assert!(conversations(&root).is_empty());
}

#[test]
fn the_working_directory_is_not_on_the_first_line() {
    // The trap this module exists to avoid: a reader that gives up after one line
    // finds nothing, always, and the symptom is an empty list rather than an
    // error.
    let root = scratch("depth");
    transcript(&root, "project", "deep", Some("/home/example/Code"), 8);

    assert_eq!(conversations(&root).len(), 1, "found several lines in");
}

#[test]
fn the_deepest_real_transcript_is_still_reached() {
    // Not an invented shape. Of the thirteen transcripts on this machine twelve
    // reach the working directory inside 1 KB; this one opens with twelve
    // `file-history-snapshot` lines averaging 38 KB and reaches it at 456 KB. A
    // 16-line window hid it completely, and no line count could have been chosen
    // correctly, because how many snapshots open a transcript is data.
    let root = scratch("real-depth");
    padded(
        &root,
        "project",
        "late",
        Some("/home/example/Code"),
        12,
        38_000,
    );

    assert_eq!(
        conversations(&root).len(),
        1,
        "the outlier is not reachable"
    );
}

#[test]
fn many_short_lines_are_not_what_the_bound_is_about() {
    // Two hundred lines of metadata is nothing to read when the lines are small,
    // and the old line-counting bound refused it. The cost being guarded against
    // is bytes off a file that can reach a gigabyte, not lines.
    let root = scratch("many-lines");
    transcript(&root, "project", "chatty", Some("/home/example/Code"), 200);

    assert_eq!(conversations(&root).len(), 1);
}

#[test]
fn a_transcript_that_would_cost_too_much_to_search_is_not_guessed_at() {
    // Past the budget the answer is "not found", which keeps the list honest: a
    // default directory would resume the conversation somewhere it never ran.
    let root = scratch("too-costly");
    padded(
        &root,
        "project",
        "buried",
        Some("/home/example/Code"),
        6,
        900_000,
    );

    assert!(
        conversations(&root).is_empty(),
        "not found is the honest answer; a default directory would resume somewhere wrong"
    );
}

#[test]
fn the_newest_conversation_comes_first() {
    // The list is for choosing what to pick up again, and the answer is almost
    // always the most recent one.
    let root = scratch("order");
    transcript(&root, "project", "older", Some("/home/example/Code"), 2);
    // Two writes cannot be relied on to differ in mtime, so make them.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    transcript(&root, "project", "newer", Some("/home/example/Code"), 2);

    let found = conversations(&root);
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].id, "newer");
}

#[test]
fn files_that_are_not_transcripts_are_ignored() {
    let root = scratch("junk");
    transcript(&root, "project", "real", Some("/home/example/Code"), 2);
    std::fs::write(root.join("project").join("notes.md"), "not a transcript").expect("write");
    std::fs::write(root.join("loose.jsonl"), r#"{"cwd":"/home/example/Code"}"#).expect("write");

    let found = conversations(&root);
    assert_eq!(
        found.len(),
        1,
        "only .jsonl inside a project folder: {found:?}"
    );
    assert_eq!(found[0].id, "real");
}

#[test]
fn a_conversation_that_ran_in_a_temporary_directory_is_not_listed() {
    // What the console's own smoke tests leave behind: the spawner is pointed at a
    // scratchpad, Claude Code files a transcript per working directory, and a
    // one-turn probe arrives in the list looking exactly like work to pick up.
    let root = scratch("disposable");
    transcript(&root, "project", "real", Some("/home/example/Code"), 2);
    transcript(
        &root,
        "encoded-tmp",
        "probe",
        Some("/private/tmp/claude-501/session/scratchpad-probe"),
        2,
    );

    let found = conversations(&root);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].id, "real");
}

#[test]
fn a_conversation_that_is_not_listed_still_exists() {
    // ⚠ The distinction the housekeeping turns on. `conversations` is a display
    // list and hides the probe above; `transcript_ids` is asked which
    // conversations are THERE, by whatever deletes things on the strength of the
    // answer. Confusing the two would have `images::tidy` delete the pictures of
    // a session for the crime of having run somewhere untidy.
    let root = scratch("ids");
    transcript(&root, "project", "real", Some("/home/example/Code"), 2);
    transcript(&root, "encoded-tmp", "probe", Some("/private/tmp/probe"), 2);

    assert_eq!(conversations(&root).len(), 1, "the display list hides one");
    assert_eq!(
        transcript_ids(&root),
        std::collections::BTreeSet::from(["real".to_string(), "probe".to_string()]),
        "and both of them are still on the disk"
    );
}

#[test]
fn every_spelling_of_the_temporary_directory_counts() {
    // `/tmp` is a symlink to `/private/tmp` on macOS and `$TMPDIR` is a third path
    // again, so which one a transcript records depends on how its process was
    // started. Recognising one of the three is the version that looks right.
    let root = scratch("spellings");
    for (n, cwd) in ["/tmp/probe", "/private/tmp/probe", "/var/folders/xy/probe"]
        .iter()
        .enumerate()
    {
        transcript(&root, "project", &format!("probe-{n}"), Some(cwd), 2);
    }

    assert!(conversations(&root).is_empty(), "one spelling escaped");
}

#[test]
fn a_directory_merely_beginning_like_one_is_still_listed() {
    // `/tmpfiles` is not `/tmp`, and a repository is not disposable for being
    // named unluckily. The trailing separator in the prefixes is what says so.
    let root = scratch("prefix");
    transcript(&root, "project", "kept", Some("/tmpfiles/Code/thing"), 2);

    assert_eq!(conversations(&root).len(), 1);
}

#[test]
fn a_temporary_conversation_can_still_be_found_by_id() {
    // The filter is about what competes for room on a phone screen, not about what
    // exists. Hiding a conversation from the list must not make it unreachable.
    let root = scratch("still-there");
    transcript(&root, "encoded-tmp", "probe", Some("/private/tmp/probe"), 2);

    assert!(conversations(&root).is_empty());
    assert!(
        transcript_of(&root, "probe").is_some(),
        "unlisted, not unreachable"
    );
}

#[test]
fn a_conversation_is_shown_by_the_name_it_gave_itself() {
    // A hex prefix identifies a transcript; the name identifies the work, which
    // is the thing anybody is actually choosing between.
    let root = scratch("named");
    named(&root, "with-a-name", Some("music"), Some("utterance"));

    let found = conversations(&root);
    assert_eq!(found.len(), 1);
    // ⚠ **The title wins here and the agent name wins in the viewer, and that is
    // the decision rather than an accident.** The reason used to be given as "one
    // is a decision, the other a default" — a rationale the viewer answered with
    // an equally confident opposite one. Settled 2026-08-07 by reading the CLI,
    // which carries both orders split by what the name is for: its resume picker
    // reads `customTitle` and never consults `agentName`, its session labeller
    // reads `agentName` first. This is a list of conversations to pick from, so
    // it is the picker's question. See `reader::transcript::AS_CONVERSATION`.
    assert_eq!(
        found[0].name.as_deref(),
        Some("music"),
        "a list of conversations shows what a person last renamed one to"
    );
}

#[test]
fn the_agent_name_is_used_when_nothing_was_set_by_hand() {
    let root = scratch("agent-only");
    named(&root, "auto", None, Some("health"));

    assert_eq!(conversations(&root)[0].name.as_deref(), Some("health"));
}

#[test]
fn a_conversation_that_never_took_a_name_has_none() {
    // Rather than inventing one from the id, which would read as a name and be a
    // hex string wearing a hat.
    let root = scratch("anonymous");
    named(&root, "nameless", None, None);

    assert!(conversations(&root)[0].name.is_none());
}

#[test]
fn a_later_name_replaces_an_earlier_one() {
    // Sessions get renamed as their job changes. The current name is the useful
    // one, which is why the tail is read rather than the head.
    let root = scratch("renamed");
    let folder = root.join("project");
    std::fs::create_dir_all(&folder).expect("dir");
    std::fs::write(
        folder.join("renamed.jsonl"),
        [
            r#"{"type":"system","cwd":"/home/example/Code"}"#,
            r#"{"type":"custom-title","customTitle":"first"}"#,
            r#"{"type":"custom-title","customTitle":"second"}"#,
        ]
        .join("\n"),
    )
    .expect("transcript");

    assert_eq!(conversations(&root)[0].name.as_deref(), Some("second"));
}

/// A transcript whose assistant messages carry the token counts the real ones do,
/// with a compaction filed after the `compacted_after`th of them.
///
/// `input` is deliberately tiny beside `cache_read`, as it is in life: a cached
/// prompt of half a million tokens reports two of input, so anything reading
/// `input_tokens` alone calls a full conversation empty.
fn spent(dir: &Path, id: &str, requests: &[(u64, u64, u64)], compacted_after: Option<usize>) {
    let folder = dir.join("project");
    std::fs::create_dir_all(&folder).expect("project dir");
    let mut lines = vec![r#"{"type":"system","cwd":"/home/example/Code"}"#.to_string()];
    for (nth, (input, creation, read)) in requests.iter().enumerate() {
        lines.push(format!(
            r#"{{"type":"assistant","message":{{"role":"assistant","usage":{{"input_tokens":{input},"cache_creation_input_tokens":{creation},"cache_read_input_tokens":{read},"output_tokens":9}},"content":[{{"type":"text","text":"hello"}}]}}}}"#
        ));
        if compacted_after == Some(nth) {
            lines.push(r#"{"type":"system","subtype":"compact_boundary"}"#.to_string());
        }
    }
    std::fs::write(
        folder.join(format!("{id}.jsonl")),
        // ⚠ With the trailing newline the CLI writes, because `counted` reads
        // forward a whole line at a time and stops before a partial one — a
        // fixture ending mid-line would hide its own last event from it.
        format!("{}\n", lines.join("\n")),
    )
    .expect("transcript");
}

#[test]
fn a_conversation_says_how_full_it_was_when_it_stopped() {
    // The last request's prompt, not the first and not the sum: the list is read
    // to decide what to pick up, and what matters about a conversation there is
    // how much room is left in it.
    let root = scratch("fullness");
    spent(
        &root,
        "deep",
        &[(2, 1272, 200_000), (2, 900, 546_967)],
        None,
    );

    assert_eq!(
        conversations(&root)[0].context,
        Some(2 + 900 + 546_967),
        "the newest request, with all three token kinds added together"
    );
}

#[test]
fn a_conversation_with_nothing_to_go_on_says_nothing() {
    // Rather than zero, which on screen is a claim — an empty context — where
    // this is the absence of a measurement.
    let root = scratch("no-fullness");
    named(&root, "quiet", Some("idle"), None);

    assert!(conversations(&root)[0].context.is_none());
}

#[test]
fn a_compaction_leaves_the_fullness_unknown() {
    // The measurement is not stale, it is about a conversation that no longer
    // exists: everything it counted was replaced by a summary. Half a million
    // tokens shown against a session that now holds a few is not a rounding
    // error, it is the wrong conversation's number.
    //
    // Nothing is shown rather than an estimate of what a fresh context weighs.
    // A plausible figure is indistinguishable on screen from a measured one, and
    // every other number on that card is measured — see the sibling test above
    // for the same rule where there has never been a measurement at all.
    let root = scratch("compacted-fullness");
    spent(
        &root,
        "deep",
        &[(2, 1272, 200_000), (2, 900, 546_967)],
        Some(1),
    );

    assert!(conversations(&root)[0].context.is_none());
}

#[test]
fn a_measurement_after_a_compaction_is_the_one_that_counts() {
    // The boundary clears what preceded it and nothing else. A session that has
    // spoken since knows its new size, and that is the number to show — the
    // unknown state lasts one message, not until the conversation ends.
    let root = scratch("compacted-then-spoke");
    spent(
        &root,
        "deep",
        &[(2, 900, 546_967), (2, 40, 12_000)],
        Some(0),
    );

    assert_eq!(conversations(&root)[0].context, Some(2 + 40 + 12_000));
}

#[test]
fn a_live_session_is_told_when_its_fullness_is_gone() {
    // ⚠ **The incremental read is the only way a running session finds out.**
    // The CLI writes the boundary to the file and says nothing on stdout, so a
    // console watching the stream sees a compaction happen as silence. Without
    // this the card keeps the pre-compaction figure until the session finishes
    // another turn — which, for a conversation compacted at the end of one, is
    // however long it sits idle.
    let root = scratch("compaction-reported");
    spent(&root, "deep", &[(2, 900, 546_967)], Some(0));
    let path = transcript_of(&root, "deep").expect("transcript");

    let seen = counted(&path, Counted::default());
    assert!(seen.compacted, "the boundary was in the bytes just read");
    assert!(
        seen.context.is_none(),
        "and nothing has been measured since it"
    );
}

#[test]
fn a_conversation_this_console_just_stopped_running_is_free_at_once() {
    // A transcript written seconds ago used to read as in use, which meant the
    // session a console restart had *just* killed could not be picked up for two
    // minutes — the exact moment somebody wants it back. Nothing runs `claude`
    // here but the console, and it kills what it runs on the way out, so the
    // process table is accurate immediately and freshness says nothing.
    let root = scratch("busy-fresh");
    named(&root, "fresh", Some("live"), None);

    assert!(
        !conversations(&root)[0].busy,
        "written a moment ago, but nothing is running it"
    );
}

#[test]
fn an_old_transcript_nobody_names_is_free() {
    let root = scratch("busy-old");
    named(&root, "cold", Some("dormant"), None);
    // Backdate it well past the freshness floor.
    let path = root.join("project").join("cold.jsonl");
    let long_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    filetime::set_file_mtime(&path, filetime::FileTime::from_system_time(long_ago))
        .expect("backdate");

    assert!(
        !conversations(&root)[0].busy,
        "old and unnamed by any process"
    );
}

#[test]
fn a_shell_that_merely_mentions_claude_is_not_a_session() {
    // The defect: a conversation named `utterance` was held as in use by any
    // command on this machine containing that word — `grep utterance`, `cd
    // utterance` — because every such command line also carries the path to a
    // shell snapshot under `~/.claude/`.
    assert!(words_of_claude_processes(WRAPPER).is_empty());
}

#[test]
fn a_running_session_contributes_its_arguments() {
    let words = words_of_claude_processes(&format!("{WRAPPER}\n{REAL}\n"));
    assert!(words.iter().any(|word| word == "health"), "{words:?}");
    assert!(
        !words.iter().any(|word| word == "utterance"),
        "the wrapper's words must not leak in: {words:?}"
    );
}

#[test]
fn claude_reached_by_a_full_path_still_counts() {
    let words = words_of_claude_processes("/nix/store/abc/bin/claude --resume music\n");
    assert!(words.iter().any(|word| word == "music"), "{words:?}");
}

#[test]
fn a_live_session_can_be_named_from_the_transcript_it_is_writing() {
    // The name is not on the wire: the CLI writes `customTitle`/`agentName` to
    // its transcript and announces neither on stdout, so the only way to say
    // which agent a running session is is to read the file it is filling in.
    let root = scratch("live-name");
    named(&root, "abc-123", Some("memview"), None);

    assert_eq!(named_of(&root, "abc-123").as_deref(), Some("memview"));
}

#[test]
fn a_session_with_no_transcript_has_no_name_rather_than_a_wrong_one() {
    let root = scratch("no-name");
    assert_eq!(named_of(&root, "never-ran"), None);
}

#[test]
fn a_live_session_is_dated_by_the_transcript_it_is_writing() {
    // ⚠ **Not by when the console picked it up.** `Summary::started` is when this
    // process began — carried across an upgrade, reset by a restart — and for a
    // conversation that has run all day it is out by all day: the console's own
    // session read `13h ago` on a card while its transcript was four seconds
    // old. What the list is asked is which conversation is warm, and the file
    // being appended to is the only thing that knows.
    let root = scratch("live-touched");
    named(&root, "abc-123", Some("memview"), None);

    let touched = touched_of(&root, "abc-123").expect("a transcript that exists has a date");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("after 1970")
        .as_millis() as u64;
    assert!(
        now.saturating_sub(touched) < 60_000,
        "a file written just now should be dated just now, got {touched} against {now}"
    );
}

#[test]
fn a_session_with_no_transcript_has_no_date_rather_than_the_epoch() {
    // Zero would be a date, and a client showing "56 years ago" for a session
    // that started a second ago is worse than one showing nothing.
    let root = scratch("no-touched");
    assert_eq!(touched_of(&root, "never-ran"), None);
}

/// The last conversation line of a transcript, and the moment it was written.
const SPOKE: &str = "2026-08-03T16:40:53.605Z";
const SPOKE_MS: u64 = 1_785_775_253_605;

#[test]
fn picking_a_conversation_up_is_not_something_happening_in_it() {
    // ⚠ **Measured on `scanner`, and it is why the file's own date will not do.**
    // Opened after two days, it appeared as `just now`: resuming appends `mode`,
    // `permission-mode` and `bridge-session` lines, so the file was stamped that
    // second while the last line anybody had written was two days old.
    //
    // An earlier rule compared the file's *size* against what it was at pickup,
    // reasoning that nothing is said without being appended. True, and not
    // enough — those three lines are appended and nobody said them.
    let root = scratch("picked-up");
    let folder = root.join("project");
    std::fs::create_dir_all(&folder).expect("dir");
    std::fs::write(
        folder.join("resumed.jsonl"),
        [
            r#"{"type":"system","cwd":"/home/example/Code"}"#.to_string(),
            format!(
                r#"{{"type":"user","timestamp":"{SPOKE}","message":{{"role":"user","content":[{{"type":"text","text":"carry on"}}]}}}}"#
            ),
            // Everything below here went in when the conversation was picked up.
            r#"{"type":"mode","mode":"default"}"#.to_string(),
            r#"{"type":"permission-mode","permissionMode":"default"}"#.to_string(),
            r#"{"type":"bridge-session","sessionId":"resumed"}"#.to_string(),
        ]
        .join("\n"),
    )
    .expect("transcript");

    let found = conversations(&root);
    assert_eq!(
        found[0].modified, SPOKE_MS,
        "dated by the last thing said, not by the file it was said into"
    );
    assert_eq!(
        touched_of(&root, "resumed"),
        Some(SPOKE_MS),
        "and a live session reading the same file agrees with the list"
    );
}

#[test]
fn a_transcript_with_nothing_said_in_the_tail_falls_back_to_its_file() {
    // A conversation can end on a tool result large enough to push every line
    // anybody wrote out of the tail. The file's date is then all there is, and
    // it is better than no date: it is right for every conversation that was not
    // picked up, which is most of them.
    let root = scratch("no-speech");
    named(&root, "quiet", Some("idle"), None);

    let dated = touched_of(&root, "quiet").expect("a transcript that exists has a date");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("after 1970")
        .as_millis() as u64;
    assert!(
        now.saturating_sub(dated) < 60_000,
        "written just now, so dated just now: {dated} against {now}"
    );
}

#[test]
fn a_directory_named_after_the_session_is_not_dated_either() {
    // The same trap as the transcript lookup: Claude Code puts a DIRECTORY
    // beside `<id>.jsonl`, and a directory has a modification time that a lookup
    // matching on the stem alone would happily report as the conversation's.
    let root = scratch("touched-dir");
    std::fs::create_dir_all(root.join("project").join("dir-only")).expect("sidecar dir");
    assert_eq!(touched_of(&root, "dir-only"), None);
}

/// A transcript of a conversation: prompts, the assistant messages each one took,
/// and compactions where they happened.
///
/// Shaped like the real thing in the way that matters here — an exchange is one
/// user line followed by *several* assistant and tool-result lines, so anything
/// counting lines rather than exchanges gets a different answer.
fn spoken(dir: &Path, id: &str, exchanges: &[usize], compacted_after: Option<usize>) {
    let folder = dir.join("project");
    std::fs::create_dir_all(&folder).expect("project dir");
    let mut lines = vec![r#"{"type":"system","cwd":"/home/example/Code"}"#.to_string()];
    for (nth, replies) in exchanges.iter().enumerate() {
        lines.push(format!(
            r#"{{"type":"user","message":{{"role":"user","content":[{{"type":"text","text":"ask {nth}"}}]}}}}"#
        ));
        for reply in 0..*replies {
            lines.push(format!(
                r#"{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"text","text":"answer {nth}.{reply}"}}]}}}}"#
            ));
            lines.push(format!(
                r#"{{"type":"user","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"t{nth}{reply}","content":"ok"}}]}}}}"#
            ));
        }
        if compacted_after == Some(nth) {
            lines.push(r#"{"type":"system","subtype":"compact_boundary"}"#.to_string());
        }
    }
    std::fs::write(
        folder.join(format!("{id}.jsonl")),
        // ⚠ With the trailing newline the CLI writes. A transcript is appended
        // to a line at a time, and the count reads forward from a line
        // boundary — a fixture ending mid-line would be testing a shape the
        // real files do not have.
        format!("{}\n", lines.join("\n")),
    )
    .expect("transcript");
}

#[test]
fn an_exchange_counts_once_however_many_messages_it_took() {
    // ⚠ **The unit is the exchange, not the message.** The result line's
    // `num_turns` counts the assistant messages one exchange took — measured at
    // 5 and 8 for two real ones — which answers a question nobody asked. Three
    // exchanges of 1, 4 and 2 replies are three, not seven, and not the
    // twenty-one lines they occupy.
    let root = scratch("spoken");
    spoken(&root, "counted", &[1, 4, 2], None);
    let path = transcript_of(&root, "counted").expect("transcript");

    assert_eq!(counted(&path, Counted::default()).counted.interactions, 3);
}

#[test]
fn the_count_starts_again_after_a_compaction() {
    // A compaction is where the session stops remembering: a count spanning one
    // would describe a conversation it cannot recall. Five exchanges, compacted
    // after the second, leaves three.
    let root = scratch("compacted");
    spoken(&root, "cut", &[1, 1, 1, 1, 1], Some(1));
    let path = transcript_of(&root, "cut").expect("transcript");

    assert_eq!(counted(&path, Counted::default()).counted.interactions, 3);
}

/// A conversation holding one of everything a landmark is, and a good deal of
/// what one is not.
///
/// Shaped like the real files: the bulk is assistant replies and tool results,
/// and the things worth returning to are a rounding error among them.
fn signposted(dir: &Path, id: &str) -> std::path::PathBuf {
    let folder = dir.join("project");
    std::fs::create_dir_all(&folder).expect("project dir");
    let stamp = |n: u32| format!(r#""timestamp":"2026-08-1{}T09:0{n}:00.000Z""#, n % 2 + 1);
    let said = |n: u32, text: &str| {
        format!(
            r#"{{"type":"user",{},"message":{{"role":"user","content":[{{"type":"text","text":"{text}"}}]}}}}"#,
            stamp(n)
        )
    };
    let lines = vec![
        r#"{"type":"system","cwd":"/home/example/Code"}"#.to_string(),
        said(1, "the first thing asked"),
        // Bulk, and none of it a landmark: nobody has ever wanted to return to
        // an assistant paragraph or a tool result.
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"a reply"}]}}"#.to_string(),
        r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]}}"#.to_string(),
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t2","name":"Bash","input":{}}]}}"#.to_string(),
        // A slash command, which the CLI files as a user message wrapping it.
        said(2, "<command-name>compact</command-name><command-args></command-args>"),
        r#"{"type":"system","subtype":"compact_boundary"}"#.to_string(),
        // A picture: the image block is what makes it one, and the sentence
        // beside it is where the name of the kept copy lives.
        format!(
            r#"{{"type":"user",{},"message":{{"role":"user","content":[{{"type":"image","source":{{"type":"base64"}}}},{{"type":"text","text":"look at this (the image is also at /tmp/pics/shot-9.png)"}}]}}}}"#,
            stamp(3)
        ),
        // Plumbing nobody typed, which must not read as something they said.
        said(4, "<system-reminder>do not mention this</system-reminder>"),
        said(5, "the last thing asked"),
    ];
    let path = folder.join(format!("{id}.jsonl"));
    std::fs::write(&path, format!("{}\n", lines.join("\n"))).expect("transcript");
    path
}

#[test]
fn only_the_places_worth_returning_to_are_landmarks() {
    // ⚠ **The filter is the feature.** Assistant text and tool calls are most of
    // every transcript and nobody remembers one, so a strip listing them is a
    // strip nobody can find anything in. What a person remembers is what they
    // said, what they sent, and where the conversation was cut.
    let root = scratch("signposts");
    let path = signposted(&root, "marked");

    let found = console::past::landmarks(&path);
    let kinds: Vec<_> = found.iter().map(|mark| mark.kind).collect();
    assert_eq!(
        kinds,
        vec![
            console::past::Mark::Prompt,
            console::past::Mark::Command,
            console::past::Mark::Compacted,
            console::past::Mark::Shown,
            console::past::Mark::Prompt,
            console::past::Mark::Prompt,
        ],
        "a reply, a tool call and a tool result are not places anybody goes back to"
    );
    assert_eq!(found[0].text, "the first thing asked");
    assert_eq!(found[1].text, "compact");
    assert_eq!(
        found[3].text, "shot-9.png",
        "the picture by the name kept for it"
    );
    assert_eq!(found[4].text, "look at this", "and what was said with it");
    assert_eq!(found[5].text, "the last thing asked");
    assert!(
        found
            .iter()
            .all(|mark| !mark.text.contains("system-reminder")),
        "a reminder nobody typed must not read as something they said"
    );
    assert!(
        found[0].when.is_some(),
        "a landmark without a time cannot be grouped by day"
    );
}

#[test]
fn a_landmark_lands_on_the_page_that_holds_it() {
    // ⚠ **The whole point of the cursor, and it is off by one line if it is
    // wrong.** `page` reads BACKWARDS from the offset it is given, so a cursor
    // at the start of a landmark's line returns the page that stops just before
    // it — tapping your own message and not being shown your own message. The
    // offset is therefore the end of the line, which puts the landmark last on
    // the page it comes back on.
    let root = scratch("landing");
    let path = signposted(&root, "jumped");

    for mark in console::past::landmarks(&path) {
        let page = console::past::page(&path, Some(mark.at));
        let last = page.events.last().expect("a page with something on it");
        let arrived = match (&mark.kind, &last.event) {
            (console::past::Mark::Prompt, console::protocol::Event::Prompt { text }) => {
                text == &mark.text
            }
            (console::past::Mark::Command, console::protocol::Event::Command { text }) => {
                text == &mark.text
            }
            (console::past::Mark::Shown, console::protocol::Event::Prompt { .. }) => {
                // A picture and the words sent with it are one message and two
                // events, in that order — so the words are what the page ends on.
                true
            }
            (console::past::Mark::Compacted, console::protocol::Event::Compacted) => true,
            _ => false,
        };
        assert!(
            arrived,
            "jumping to {:?} {:?} landed on {:?} instead",
            mark.kind, mark.text, last.event
        );
    }
}

/// Add `bytes` of lines that carry no events, as a long conversation's own bulk
/// does — a `file-history-snapshot` is tens of kilobytes and says nothing the
/// count is about.
fn padded_out(path: &Path, bytes: usize) {
    use std::io::Write;

    let filler = format!(
        r#"{{"type":"file-history-snapshot","snapshot":"{}"}}"#,
        "x".repeat(4000)
    );
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .expect("transcript");
    let mut written = 0;
    while written < bytes {
        writeln!(file, "{filler}").expect("filler");
        written += filler.len() + 1;
    }
}

#[test]
fn a_seed_starts_at_the_last_compaction_and_reaches_the_same_count() {
    // ⚠ **The equivalence this optimisation rests on.** Reading from zero and
    // reading from the boundary must be the same answer, because everything
    // before the boundary is thrown away by the reset when the read passes over
    // it. Two compactions, so the *last* one is the one found.
    let root = scratch("seeded");
    spoken(&root, "cut-twice", &[1, 1, 1, 1, 1], Some(1));
    let path = transcript_of(&root, "cut-twice").expect("transcript");

    let from = console::past::seed_from(&path);
    assert!(
        from > 0,
        "a transcript with a compaction in it has somewhere later to start"
    );

    let whole = counted(&path, Counted::default());
    let seeded = counted(
        &path,
        Counted {
            interactions: 0,
            through: from,
        },
    );
    assert_eq!(whole.counted.interactions, seeded.counted.interactions);
    assert_eq!(whole.counted.through, seeded.counted.through);
    assert_eq!(whole.compacted, seeded.compacted);
    assert_eq!(whole.context, seeded.context);
}

#[test]
fn a_conversation_that_never_compacted_is_seeded_from_the_start() {
    // Nothing has been forgotten, so every exchange in the file still counts and
    // there is no shortcut to take. Zero is the honest answer rather than a
    // guess at a boundary that is not there.
    let root = scratch("uncut");
    spoken(&root, "whole", &[1, 1, 1], None);
    let path = transcript_of(&root, "whole").expect("transcript");

    assert_eq!(console::past::seed_from(&path), 0);
    assert_eq!(counted(&path, Counted::default()).counted.interactions, 3);
}

#[test]
fn the_search_for_the_boundary_widens_past_its_first_window() {
    // ⚠ **The window is a guess, and this is the case where the guess is
    // wrong.** With more bulk after the compaction than the first read covers,
    // a search that gave up there would seed from zero — right, but slowly —
    // and one that read the window as if it were the whole file would drop the
    // fragment at its start and find nothing. Ten megabytes against an eight
    // megabyte first window forces exactly one widening.
    let root = scratch("widened");
    spoken(&root, "buried", &[1, 1, 1, 1], Some(0));
    let path = transcript_of(&root, "buried").expect("transcript");
    padded_out(&path, 10 * 1024 * 1024);

    let from = console::past::seed_from(&path);
    assert!(
        from > 0,
        "the boundary is in the file, just a long way back"
    );

    let whole = counted(&path, Counted::default());
    let seeded = counted(
        &path,
        Counted {
            interactions: 0,
            through: from,
        },
    );
    assert_eq!(whole.counted.interactions, 3);
    assert_eq!(seeded.counted.interactions, whole.counted.interactions);
    assert_eq!(seeded.counted.through, whole.counted.through);
}

#[test]
fn a_transcript_that_is_not_there_counts_no_exchanges() {
    // The session has just started and has written nothing yet. Zero is the
    // honest answer; the failure to answer at all is not.
    assert_eq!(
        counted(Path::new("/no/such/transcript.jsonl"), Counted::default())
            .counted
            .interactions,
        0
    );
}

#[test]
fn a_directory_named_after_the_session_is_not_its_transcript() {
    // ⚠ Claude Code puts a directory beside the transcript with exactly the same
    // name — `<id>/subagents/`, `<id>/tool-results/` — and a directory's file
    // stem is its whole name. Matching on the stem alone found the directory
    // first, and every reader downstream then reported an empty conversation:
    // no history, no name, no count, and nothing anywhere saying why. A resumed
    // 119 MB session opened blank on the phone. The extension is what tells them
    // apart.
    // The directory ALONE, so the answer cannot depend on which entry the
    // filesystem happens to hand back first — the real failure did, which is why
    // it struck one session and not the one beside it.
    let root = scratch("sidecar-only");
    std::fs::create_dir_all(root.join("project").join("orphan").join("tool-results"))
        .expect("sidecar dir");

    assert_eq!(transcript_of(&root, "orphan"), None);

    // And with both present, the file — whichever order they arrive in.
    let both = scratch("sidecar-and-file");
    spoken(&both, "twinned", &[1, 1], None);
    std::fs::create_dir_all(both.join("project").join("twinned").join("subagents"))
        .expect("sidecar dir");

    let path = transcript_of(&both, "twinned").expect("the file, not the directory");
    assert_eq!(path.extension().and_then(|e| e.to_str()), Some("jsonl"));
    assert_eq!(counted(&path, Counted::default()).counted.interactions, 2);
}

#[test]
fn only_what_arrived_since_the_last_count_is_read_again() {
    // ⚠ **The whole point, and it was measured before it was written.** This was
    // a whole-file pass at the end of every turn: 2.1 GB and 267,002 lines for
    // the largest transcript on this machine, twenty-four seconds just to read
    // the bytes, in the task that reads that session's stdout. A turn ends by
    // appending a few kilobytes, and that is all this reads.
    let root = scratch("incremental");
    spoken(&root, "growing", &[1, 1], None);
    let path = transcript_of(&root, "growing").expect("transcript");

    let first = counted(&path, Counted::default());
    assert_eq!(first.counted.interactions, 2);
    assert_eq!(
        first.counted.through,
        std::fs::metadata(&path).expect("stat").len(),
        "a file ending in a whole line is accounted for to its end"
    );

    // One more exchange, appended the way the CLI appends them.
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("append");
    use std::io::Write;
    let again = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"and again"}]}}"#;
    writeln!(file, "{again}").expect("write");
    drop(file);

    let then = counted(&path, first.counted);
    assert_eq!(
        then.counted.interactions, 3,
        "the earlier two were not counted twice"
    );
}

#[test]
fn the_tail_is_where_a_finished_background_task_is_found() {
    // ⚠ **Measured 2026-08-06, and it is the whole reason this read returns two
    // things.** A backgrounded call answers at once with a task id, so the
    // harness's notification is its only end-of-work signal — and that
    // notification is injected as a user message nobody typed, which the CLI
    // writes to the transcript and does NOT replay on stdout. So the reader of
    // the stream never sees one: a 75-second task sat on the front page for 26
    // minutes, and every close the console had ever shown turned out to come
    // from a seed re-reading the file.
    let root = scratch("finished");
    spoken(&root, "still-going", &[1], None);
    let path = transcript_of(&root, "still-going").expect("transcript");
    let before = counted(&path, Counted::default());
    assert!(before.finished.is_empty(), "nothing has finished yet");

    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("append");
    use std::io::Write;
    // ⚠ **The queue line, not the user message it later becomes.** Both are real
    // and both are here, in the order and with the gap the CLI writes them:
    // enqueued when the work ends, turned into a message only when the turn in
    // progress lets go — measured three minutes apart. Reading only the second
    // leaves the card claiming work is running for the whole of that.
    let queued = r#"{"type":"queue-operation","operation":"enqueue","content":"<task-notification>\n<task-id>b7iotoait</task-id>\n<tool-use-id>toolu_probe</tool-use-id>\n<status>completed</status>\n</task-notification>"}"#;
    writeln!(file, "{queued}").expect("write");
    drop(file);

    let after = counted(&path, before.counted);
    assert_eq!(
        after.finished,
        vec![console::protocol::Named::Call("toolu_probe".to_string())]
    );
    // And it is not an exchange: nobody said anything. The count is cumulative,
    // so the test is that it did not move.
    assert_eq!(
        after.counted.interactions, before.counted.interactions,
        "a notification is not somebody speaking"
    );
}

#[test]
fn a_monitor_that_timed_out_is_found_there_too_under_its_other_name() {
    // ⚠ **This is the path that matters for a monitor, not the live stream.**
    // The notification is written to the transcript and never put on stdout, so
    // a running session finds every ending here — including the one kind that
    // cannot name the call it came from. Verbatim from 2026-08-15, where
    // memview #925 was noticed: a monitor timed out at 14:06:32 and was still
    // drawn as running an hour later.
    let root = scratch("timed-out");
    spoken(&root, "watching", &[1], None);
    let path = transcript_of(&root, "watching").expect("transcript");
    let before = counted(&path, Counted::default());

    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("append");
    use std::io::Write;
    let queued = r#"{"type":"queue-operation","operation":"enqueue","content":"<task-notification>\n<task-id>b9drzo2f6</task-id>\n<summary>Monitor event: \"fleet bump progress, per repo\"</summary>\n<event>[Monitor timed out — re-arm if needed.]</event>\n</task-notification>"}"#;
    writeln!(file, "{queued}").expect("write");
    drop(file);

    assert_eq!(
        counted(&path, before.counted).finished,
        vec![console::protocol::Named::Task("b9drzo2f6".to_string())],
        "named by its task, which is the only name it has"
    );
}

#[test]
fn a_compaction_arriving_later_still_resets_the_count() {
    // The CLI compacts on its own and announces it nowhere on the stream — the
    // boundary is written to the file and only to the file. Counting forward
    // from an offset has to keep seeing that, or a compaction mid-session would
    // leave the number climbing past a conversation the session cannot recall.
    let root = scratch("later-compaction");
    spoken(&root, "cut-later", &[1, 1], None);
    let path = transcript_of(&root, "cut-later").expect("transcript");
    let before = counted(&path, Counted::default());
    assert_eq!(before.counted.interactions, 2);

    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("append");
    use std::io::Write;
    let boundary = r#"{"type":"system","subtype":"compact_boundary"}"#;
    let after =
        r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"after"}]}}"#;
    writeln!(file, "{boundary}").expect("write");
    writeln!(file, "{after}").expect("write");
    drop(file);

    assert_eq!(counted(&path, before.counted).counted.interactions, 1);
}

#[test]
fn a_transcript_that_shrank_is_counted_from_the_start() {
    // An offset into a file that has been replaced points at the middle of a
    // line at best. Starting again is the only answer that cannot be wrong.
    let root = scratch("shrunk");
    spoken(&root, "replaced", &[1, 1], None);
    let path = transcript_of(&root, "replaced").expect("transcript");

    let stale = Counted {
        interactions: 99,
        through: 10_000_000,
    };
    assert_eq!(counted(&path, stale).counted.interactions, 2);
}
