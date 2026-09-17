//! Break one thing, and require the checker to object.
//!
//! A checker that has only ever been run against healthy input has demonstrated
//! nothing, and this crate already carries the scar: a regression test in
//! `transcript.rs` passed with its fix ablated, because what it really tested
//! was whichever entry `read_dir` returned first. So every rule is exercised
//! twice — once against a transcript that satisfies it, once against the same
//! transcript with a single field broken — and the pair must disagree.
//!
//! These replace `scripts/check-transcripts.py`, which found the invariants and
//! then became a second implementation of them. Two checkers drift; the module
//! doc on `is_transcript` is about exactly that failure.

use reader::transcript::{MESSAGE_TYPES, Rule, Tail, check};

const SESSION: &str = "0a1b2c3d-4e5f-4a6b-8c9d-0e1f2a3b4c5d";
const U1: &str = "11111111-1111-4111-8111-111111111111";
const U2: &str = "22222222-2222-4222-8222-222222222222";
const U3: &str = "33333333-3333-4333-8333-333333333333";
const ABSENT: &str = "99999999-9999-4999-8999-999999999999";

/// One conversation line carrying everything the corpus says such a line always
/// carries, so a test that breaks one field is breaking exactly one thing.
fn line(kind: &str, uuid: &str, parent: Option<&str>) -> String {
    let parent = match parent {
        Some(parent) => format!("\"{parent}\""),
        None => "null".to_string(),
    };
    // Asking the module which types carry a message, rather than restating it:
    // a fixture that keeps its own copy of a rule can agree with itself while
    // both drift away from the corpus.
    let message = if MESSAGE_TYPES.contains(&kind) {
        format!(r#","message":{{"role":"{kind}","content":"hello"}}"#)
    } else {
        String::new()
    };
    format!(
        r#"{{"type":"{kind}","uuid":"{uuid}","parentUuid":{parent},"sessionId":"{SESSION}","timestamp":"2026-08-10T12:00:00.000Z","cwd":"/home/example","version":"2.1.0","isSidechain":false,"userType":"external","gitBranch":"main"{message}}}"#
    )
}

/// A minimal but structurally complete conversation.
///
/// Deliberately contains a metadata line and a re-emitted uuid, so the happy
/// path covers the two shapes most easily mistaken for damage.
fn healthy() -> Vec<String> {
    vec![
        line("user", U1, None),
        line("assistant", U2, Some(U1)),
        r#"{"type":"mode","mode":"default"}"#.to_string(),
        line("user", U3, Some(U2)),
        // Lawful re-emission: the same node, later moved onto a different
        // parent. 432 lines in the live corpus do this.
        line("user", U3, Some(U1)),
    ]
}

fn joined(lines: &[String]) -> Vec<u8> {
    let mut text = lines.join("\n");
    text.push('\n');
    text.into_bytes()
}

fn rules(lines: &[String], tail: Tail) -> Vec<Rule> {
    check(&joined(lines), tail, Some(SESSION))
        .into_iter()
        .map(|violation| violation.rule)
        .collect()
}

#[test]
fn a_sound_transcript_has_nothing_wrong_with_it() {
    assert!(rules(&healthy(), Tail::MustBeComplete).is_empty());
}

#[test]
fn a_parent_that_names_nothing_is_damage() {
    // The failure the whole module exists for: the message it pointed at is
    // gone, and every reader in this workspace rendered it as if it were not.
    let mut lines = healthy();
    lines[1] = line("assistant", U2, Some(ABSENT));
    assert!(rules(&lines, Tail::MustBeComplete).contains(&Rule::DanglingParent));
}

#[test]
fn a_line_that_is_not_json_is_damage() {
    let mut lines = healthy();
    lines[1] = "{not json".to_string();
    assert!(rules(&lines, Tail::MustBeComplete).contains(&Rule::Unparseable));
}

#[test]
fn a_line_that_is_not_an_object_is_damage() {
    let mut lines = healthy();
    lines[1] = "[1, 2, 3]".to_string();
    assert!(rules(&lines, Tail::MustBeComplete).contains(&Rule::NotAnObject));
}

#[test]
fn a_type_outside_the_vocabulary_is_damage() {
    let mut lines = healthy();
    lines[1] = line("invented", U2, Some(U1));
    assert!(rules(&lines, Tail::MustBeComplete).contains(&Rule::UnknownType));
}

#[test]
fn a_conversation_line_without_identity_is_damage() {
    let mut lines = healthy();
    lines[1] = line("assistant", U2, Some(U1)).replace(&format!(r#""uuid":"{U2}","#), "");
    assert!(rules(&lines, Tail::MustBeComplete).contains(&Rule::MissingUuid));
}

#[test]
fn a_uuid_that_is_not_a_uuid_is_damage() {
    let mut lines = healthy();
    lines[1] = line("assistant", "nope", Some(U1));
    assert!(rules(&lines, Tail::MustBeComplete).contains(&Rule::MalformedUuid));
}

#[test]
fn an_absent_parent_field_is_not_the_same_as_a_null_one() {
    // ⚠ Conflating these two is not hypothetical. Doing it once reported 81,062
    // roots where there are 3,260, and 349,636 broken links where there were
    // three -- the whole first survey of the corpus was wrong from this alone.
    let mut lines = healthy();
    lines[1] = line("assistant", U2, Some(U1)).replace(r#""parentUuid":"#, r#""noParent":"#);
    assert!(rules(&lines, Tail::MustBeComplete).contains(&Rule::MissingParentField));

    // ... while an explicit null on a type that may root is entirely sound.
    let rooted = vec![line("user", U1, None), line("user", U3, None)];
    assert!(rules(&rooted, Tail::MustBeComplete).is_empty());
}

#[test]
fn an_assistant_at_the_root_is_damage() {
    // Not one of 536,780 assistant lines in the corpus begins a chain, so one
    // that does means something was severed above it.
    let mut lines = healthy();
    lines[1] = line("assistant", U2, None);
    assert!(rules(&lines, Tail::MustBeComplete).contains(&Rule::UnrootableTypeAtRoot));
}

#[test]
fn metadata_carrying_identity_is_damage() {
    let mut lines = healthy();
    lines[2] = format!(r#"{{"type":"mode","uuid":"{U1}"}}"#);
    assert!(rules(&lines, Tail::MustBeComplete).contains(&Rule::MetadataWithUuid));

    let mut parented = healthy();
    parented[2] = format!(r#"{{"type":"mode","parentUuid":"{U1}"}}"#);
    assert!(rules(&parented, Tail::MustBeComplete).contains(&Rule::MetadataWithParent));
}

#[test]
fn a_parent_that_is_neither_a_string_nor_null_is_damage() {
    let mut lines = healthy();
    lines[1] = line("assistant", U2, None).replace(r#""parentUuid":null"#, r#""parentUuid":17"#);
    assert!(rules(&lines, Tail::MustBeComplete).contains(&Rule::NonStringParent));
}

#[test]
fn one_uuid_may_move_but_may_not_change_what_it_is() {
    // Moving is lawful and the healthy fixture already does it. Changing kind is
    // not: 0 of 309,290 repeat events in the corpus.
    let mut lines = healthy();
    lines.push(line("assistant", U3, Some(U2)));
    assert!(rules(&lines, Tail::MustBeComplete).contains(&Rule::UuidTypeChange));
}

#[test]
fn a_chain_that_returns_to_itself_is_damage() {
    // Never observed, and checked anyway: it had simply never been measured, and
    // an unmeasured invariant is an assumption wearing a rule's clothes.
    let lines = vec![
        line("user", U1, Some(U3)),
        line("assistant", U2, Some(U1)),
        line("user", U3, Some(U2)),
    ];
    assert!(rules(&lines, Tail::MustBeComplete).contains(&Rule::Cycle));
}

#[test]
fn a_line_missing_what_every_line_carries_is_damage() {
    // 942,556 of 942,556 conversation lines have all seven.
    for field in ["sessionId", "timestamp", "cwd", "version", "gitBranch"] {
        let mut lines = healthy();
        lines[1] = line("assistant", U2, Some(U1)).replace(&format!(r#""{field}":"#), r#""x":"#);
        assert!(
            rules(&lines, Tail::MustBeComplete).contains(&Rule::MissingField),
            "a line without {field} should be rejected"
        );
    }
}

#[test]
fn a_line_belonging_to_another_conversation_is_damage() {
    // The check a REWRITTEN transcript has to pass: a copy taken under a new
    // session id, with its lines left alone, says in every one of them where it
    // really came from.
    let mut lines = healthy();
    lines[1] = line("assistant", U2, Some(U1)).replace(SESSION, ABSENT);
    assert!(rules(&lines, Tail::MustBeComplete).contains(&Rule::SessionMismatch));
}

#[test]
fn the_role_stated_twice_must_agree_with_itself() {
    let mut lines = healthy();
    lines[1] = line("assistant", U2, Some(U1)).replace(r#""role":"assistant""#, r#""role":"user""#);
    assert!(rules(&lines, Tail::MustBeComplete).contains(&Rule::RoleMismatch));
}

#[test]
fn a_message_belongs_to_exactly_the_types_that_have_one() {
    let mut missing = healthy();
    missing[1] = line("assistant", U2, Some(U1))
        .replace(r#","message":{"role":"assistant","content":"hello"}"#, "");
    assert!(rules(&missing, Tail::MustBeComplete).contains(&Rule::MissingField));

    let mut surprising = healthy();
    surprising[1] = line("system", U2, Some(U1)).replace(
        r#""gitBranch":"main""#,
        r#""gitBranch":"main","message":{}"#,
    );
    assert!(rules(&surprising, Tail::MustBeComplete).contains(&Rule::UnexpectedField));
}

#[test]
fn a_prompt_id_outside_a_user_line_is_damage() {
    let mut lines = healthy();
    lines[1] = line("assistant", U2, Some(U1)).replace(
        r#""gitBranch":"main""#,
        r#""gitBranch":"main","promptId":"p1""#,
    );
    assert!(rules(&lines, Tail::MustBeComplete).contains(&Rule::UnexpectedField));
}

#[test]
fn a_half_written_last_line_is_tolerated_only_where_it_is_explicable() {
    // The append race, and the only concession to leniency in the module.
    let mut text = healthy().join("\n");
    text.push('\n');
    text.push_str(r#"{"type":"user","uuid":"#); // no closing brace, no newline
    let bytes = text.into_bytes();

    let live: Vec<Rule> = check(&bytes, Tail::MayBeIncomplete, Some(SESSION))
        .into_iter()
        .map(|v| v.rule)
        .collect();
    assert_eq!(live, vec![Rule::IncompleteTail]);
    assert!(!Rule::IncompleteTail.is_damage());

    // The same bytes from a file nothing is writing to are simply broken.
    let settled: Vec<Rule> = check(&bytes, Tail::MustBeComplete, Some(SESSION))
        .into_iter()
        .map(|v| v.rule)
        .collect();
    assert_eq!(settled, vec![Rule::Unparseable]);
}

#[test]
fn the_allowance_is_for_the_last_line_and_no_other() {
    // A truncated record in the MIDDLE cannot be explained by an append in
    // progress, so liveness must not excuse it.
    let mut lines = healthy();
    lines[1] = r#"{"type":"user","uuid":"#.to_string();
    assert!(rules(&lines, Tail::MayBeIncomplete).contains(&Rule::Unparseable));
}

#[test]
fn a_complete_last_line_is_never_excused() {
    // If the file ends with a newline the final record is whole by construction,
    // so a parse failure there is damage even on a live file.
    let mut lines = healthy();
    lines.push(r#"{"type":"user","broken"#.to_string());
    assert!(rules(&lines, Tail::MayBeIncomplete).contains(&Rule::Unparseable));
}
