//! A tool call's arguments, read once into what a client draws.

use console::call::{Call, Question};
use serde_json::json;

/// The shape a real `AskUserQuestion` arrives in, captured off the wire.
fn asked() -> serde_json::Value {
    json!({
        "questions": [{
            "question": "how far should the question UI go?",
            "header": "Scope",
            "multiSelect": false,
            "options": [
                { "label": "options only", "description": "render each option as a button" },
                { "label": "full parity", "description": "free text and notes as well" }
            ]
        }]
    })
}

fn questions(input: &serde_json::Value) -> Option<Vec<Question>> {
    match Call::read("AskUserQuestion", input) {
        Call::Question { questions } => Some(questions),
        _ => None,
    }
}

#[test]
fn a_question_reads_the_way_it_was_asked() {
    let read = questions(&asked()).expect("readable");
    let question = &read[0];
    assert_eq!(question.question, "how far should the question UI go?");
    assert_eq!(question.header, "Scope");
    assert!(!question.multi_select);
    let labels: Vec<&str> = question.options.iter().map(|o| o.label.as_str()).collect();
    assert_eq!(labels, ["options only", "full parity"]);
    assert_eq!(
        question.options[0].description,
        "render each option as a button"
    );
}

#[test]
fn a_question_that_cannot_be_read_whole_is_not_a_question() {
    // A person choosing from a list cannot tell that an option is missing, so a
    // half-read question falls to the ordinary allow/refuse row instead.
    let damaged = json!({ "questions": [{
        "question": "which way?",
        "options": [{ "label": "left" }, { "description": "no label at all" }]
    }]});
    assert_eq!(questions(&damaged), None);
    let half =
        json!({ "questions": [asked()["questions"][0], { "question": "and?", "options": [] }] });
    assert_eq!(questions(&half), None);
    assert_eq!(questions(&json!({ "questions": [] })), None);
    assert_eq!(questions(&json!({ "command": "rm -rf /tmp/x" })), None);
}

#[test]
fn only_an_explicit_true_is_several_choices_and_a_missing_header_is_empty() {
    let mut input = asked();
    input["questions"][0]["multiSelect"] = json!("yes");
    input["questions"][0]
        .as_object_mut()
        .expect("object")
        .remove("header");
    let question = &questions(&input).expect("readable")[0];
    assert!(!question.multi_select);
    assert_eq!(question.header, "");
}

#[test]
fn an_edit_is_the_text_it_replaces_and_what_replaces_it() {
    let read = Call::read(
        "Edit",
        &json!({ "file_path": "/a.rs", "old_string": "one", "new_string": "", "replace_all": true }),
    );
    assert_eq!(
        read,
        Call::Edit {
            path: "/a.rs".to_string(),
            before: "one".to_string(),
            after: String::new(),
            everywhere: true,
        }
    );
}

#[test]
fn a_command_is_a_shell_call_and_one_without_is_not() {
    assert!(matches!(
        Call::read("Bash", &json!({ "command": "ls", "description": "list" })),
        Call::Bash { command, description: Some(said) } if command == "ls" && said == "list"
    ));
    assert!(matches!(
        Call::read("Bash", &json!({ "description": "no command" })),
        Call::Other { .. }
    ));
}

#[test]
fn any_other_call_shows_the_argument_it_works_on_else_its_argument_names() {
    let shown = |tool, input| match Call::read(tool, &input) {
        Call::Other { shown, .. } => shown,
        other => panic!("{other:?}"),
    };
    assert_eq!(
        shown("Read", json!({ "file_path": "/a.rs", "limit": 5 })),
        "/a.rs"
    );
    assert_eq!(
        shown("Grep", json!({ "path": "src", "pattern": "fn" })),
        "src"
    );
    assert_eq!(shown("TaskStop", json!({ "task_id": "b1" })), "task_id");
    assert_eq!(shown("TaskList", json!({})), "TaskList");
}

#[test]
fn a_workflow_is_named_by_its_script() {
    let named = |input| match Call::read("Workflow", &input) {
        Call::Workflow { name } => name,
        other => panic!("{other:?}"),
    };
    assert_eq!(
        named(json!({ "script": "export const meta = {\n  name: 'comment-pass-1721',\n}\nagent('x', { name: 'no' })" })).as_deref(),
        Some("comment-pass-1721")
    );
    assert_eq!(
        named(json!({ "name": "deep-research" })).as_deref(),
        Some("deep-research")
    );
    assert_eq!(
        named(json!({ "scriptPath": "/p/scripts/comment-pass-1721-wf_0480c7f6-60f.js" }))
            .as_deref(),
        Some("comment-pass-1721")
    );
}
