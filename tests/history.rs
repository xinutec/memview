//! The history miner's pure parts, exercised through the public API.
//!
//! `clean_prompt` gets the most attention because it is the whole difference
//! between a search index and a pile of noise: a "user" message is also how the
//! harness delivers reminders, slash-command output and background-task
//! notifications, and on the live corpus those were half the indexed text.
use memview::history::{clean_prompt, project_of};

#[test]
fn a_plain_prompt_survives_untouched() {
    assert_eq!(clean_prompt("what's next?"), "what's next?");
    assert_eq!(clean_prompt("  spaced  "), "spaced");
}

#[test]
fn injected_context_is_not_speech() {
    // The reminder wraps the real message; only the message is a prompt.
    let raw = "<system-reminder>recalled memory: foo</system-reminder>proceed";
    assert_eq!(clean_prompt(raw), "proceed");
}

#[test]
fn a_background_wakeup_leaves_no_prompt_at_all() {
    // 2,934 turns on the live corpus. A task completing wakes the session with
    // a message shaped exactly like a prompt; the reader asked nothing, so the
    // honest record is an empty prompt rather than the notification's text.
    let raw = "<task-notification>\n<task-id>abc</task-id>\n<status>completed</status>\n</task-notification>";
    assert_eq!(clean_prompt(raw), "");
}

#[test]
fn slash_command_output_is_not_something_the_reader_wrote() {
    let raw = "<local-command-caveat>ignore this</local-command-caveat>\
               <command-name>/context</command-name>\
               <local-command-stdout>31% used</local-command-stdout>what's next?";
    assert_eq!(clean_prompt(raw), "what's next?");
}

#[test]
fn ordinary_angle_brackets_are_kept() {
    // Prose and code are full of them, and eating a `<` would corrupt the very
    // text the index exists to search.
    assert_eq!(
        clean_prompt("use Vec<String> when x < y"),
        "use Vec<String> when x < y"
    );
    assert_eq!(clean_prompt("a <b> tag"), "a <b> tag");
}

#[test]
fn an_unclosed_machine_tag_drops_the_rest_rather_than_leaking_it() {
    // A truncated transcript line must not leave a fragment of injected text
    // sitting in the index as if it were speech.
    let out = clean_prompt("real question<system-reminder>secret context that never closes");
    assert_eq!(out, "real question");
}

#[test]
fn a_very_long_prompt_is_truncated_on_a_character_boundary() {
    // Multibyte on the cut point: slicing mid-character would panic.
    let raw = "é".repeat(5000);
    let out = clean_prompt(&raw);
    assert!(out.len() <= 4100, "{}", out.len());
    assert!(out.ends_with('…'));
}

const ROOT: &str = "/home/example/Code";

#[test]
fn a_project_is_the_first_directory_under_the_code_root() {
    assert_eq!(
        project_of("/home/example/Code/heatcam", ROOT),
        Some("heatcam".into())
    );
    assert_eq!(
        project_of("/home/example/Code/heatcam/android/app", ROOT),
        Some("heatcam".into())
    );
    // A trailing slash on the root must not change the answer.
    assert_eq!(
        project_of("/home/example/Code/heatcam", "/home/example/Code/"),
        Some("heatcam".into())
    );
}

#[test]
fn work_outside_the_code_root_belongs_to_no_project() {
    // Honest rather than convenient: a session sitting in the home directory
    // was not working on a project, and inventing one would put turns in it.
    assert_eq!(project_of("/home/example", ROOT), None);
    assert_eq!(project_of("/tmp", ROOT), None);
    assert_eq!(project_of("/home/example/Code", ROOT), None);
    assert_eq!(project_of("/home/example/Code/", ROOT), None);
}

#[test]
fn a_sibling_directory_is_not_a_prefix_match() {
    // "/home/example/Codex/thing" starts with the root as a STRING but is not
    // under it. Requiring the separator is what keeps it out.
    assert_eq!(project_of("/home/example/Codex/thing", ROOT), None);
}

#[test]
fn a_compaction_summary_is_not_a_prompt() {
    // The worst possible thing to index: it restates everything a session
    // discussed, so one of them matches almost any query about that session.
    // 186 on the live corpus — 1.7% of prompts, 43% of all indexed text.
    let raw = "This session is being continued from a previous conversation that ran \
               out of context. Summary: we built the FLIR driver, the memory graph, …";
    assert_eq!(clean_prompt(raw), "");
}

#[test]
fn a_summary_is_caught_even_behind_a_reminder() {
    // The marker is tested AFTER stripping, because a summary is usually
    // preceded by an injected block and a raw prefix test would miss it.
    let raw = "<system-reminder>context</system-reminder>This session is being \
               continued from a previous conversation that ran out of context.";
    assert_eq!(clean_prompt(raw), "");
}

#[test]
fn a_prompt_merely_mentioning_a_summary_is_kept() {
    // Only an opening counts. Someone asking about compaction is asking a real
    // question, and dropping it would hide exactly the turn they searched for.
    let raw = "why is this session being continued from a previous conversation?";
    assert_eq!(clean_prompt(raw), raw);
}
