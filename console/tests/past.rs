//! Finding conversations that already happened.
//!
//! The transcripts are read rather than their filenames decoded, so these fixtures
//! are shaped like the real thing in the two ways that matter: the working
//! directory arrives a few lines in rather than on the first, and the directory a
//! transcript claims need not match the name of the folder holding it.

use std::path::Path;

use console::past::{
    conversations, interactions, named as named_of, transcript_of, words_of_claude_processes,
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
    assert_eq!(
        found[0].name.as_deref(),
        Some("music"),
        "custom-title wins: one is a decision, the other a default"
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
    std::fs::write(folder.join(format!("{id}.jsonl")), lines.join("\n")).expect("transcript");
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

    assert_eq!(interactions(&path), 3);
}

#[test]
fn the_count_starts_again_after_a_compaction() {
    // A compaction is where the session stops remembering: a count spanning one
    // would describe a conversation it cannot recall. Five exchanges, compacted
    // after the second, leaves three.
    let root = scratch("compacted");
    spoken(&root, "cut", &[1, 1, 1, 1, 1], Some(1));
    let path = transcript_of(&root, "cut").expect("transcript");

    assert_eq!(interactions(&path), 3);
}

#[test]
fn a_transcript_that_is_not_there_counts_no_exchanges() {
    // The session has just started and has written nothing yet. Zero is the
    // honest answer; the failure to answer at all is not.
    assert_eq!(interactions(Path::new("/no/such/transcript.jsonl")), 0);
}
