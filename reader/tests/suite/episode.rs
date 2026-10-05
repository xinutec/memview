//! The episode key: acts in sequence with their subjects abstracted to
//! identity, so that recurrence across days and files is one shape.
//!
//! Each property here is one the census rests on, and the instrument cannot
//! check for itself: a key that kept a path would rank nothing twice and read
//! as "no recurrence", which is the same answer a true absence gives.

use reader::episode::{Call, Token, grams, key, tokens};
use reader::project::read as parse;
use reader::shell_files::trace;

const HOME: &str = "/home/example";
const CWD: &str = "/home/example/Code/health";

/// One call's tokens, as the census would build them.
fn call(script: &str, said: Option<&str>) -> Call {
    let cmds = parse(script).unwrap_or_else(|at| panic!("failed to parse, stopped at {at:?}"));
    Call {
        tokens: tokens(&trace(&cmds, Some(CWD), HOME).steps).tokens,
        said: said.map(str::to_string),
    }
}

/// The one key an episode of these calls has at length `n`.
fn only(calls: &[Call], n: usize) -> String {
    let all = grams(calls, n);
    assert_eq!(all.len(), 1, "expected one {n}-gram, got {all:?}");
    all.into_iter().next().expect("one").0
}

#[test]
fn the_same_sequence_over_different_files_is_one_key() {
    let f = [call("sed -i 's/a/b/' f.ts && cat f.ts", None)];
    let g = [call("sed -i 's/a/b/' g.ts && cat g.ts", None)];
    assert_eq!(only(&f, 2), only(&g, 2));
    assert_eq!(only(&f, 2), "Rewrite(A) ; Page(A)");
}

#[test]
fn a_sequence_over_two_files_is_a_different_key() {
    let same = [call("sed -i 's/a/b/' f.ts && cat f.ts", None)];
    let other = [call("sed -i 's/a/b/' f.ts && cat g.ts", None)];
    assert_ne!(only(&same, 2), only(&other, 2));
    assert_eq!(only(&other, 2), "Rewrite(A) ; Page(B)");
}

#[test]
fn a_gram_says_where_the_next_call_begins() {
    let one = [call("git log --oneline -3 && git status", None)];
    let two = [call("git log --oneline -3", None), call("git status", None)];
    assert_eq!(only(&one, 2), "History ; Status");
    assert_eq!(only(&two, 2), "History | Status");
}

#[test]
fn context_is_not_an_act() {
    let calls = [call("cd src && echo start && cat f.ts && sleep 1", None)];
    assert_eq!(calls[0].tokens.len(), 1, "{:?}", calls[0].tokens);
    assert_eq!(only(&calls, 1), "Page(A)");
}

#[test]
fn a_carrier_contributes_its_childrens_acts_once() {
    let calls = [call("bash -c 'cat f.ts'", None)];
    assert_eq!(only(&calls, 1), "Page(A)");
}

#[test]
fn an_unlifted_act_is_in_the_sequence_by_its_shape() {
    let calls = [call("cat f.ts && cargo test && git commit -m done", None)];
    let keys: Vec<String> = grams(&calls, 3).into_iter().map(|(k, _)| k).collect();
    assert_eq!(keys.len(), 1);
    let k = &keys[0];
    assert!(k.starts_with("Page(A) ; "), "{k}");
    assert!(k.ends_with(" ; Commit"), "{k}");
    assert!(
        k.contains(" · cargo test"),
        "the queued act keeps its census shape: {k}"
    );
}

#[test]
fn a_gram_carries_the_description_of_the_call_its_last_act_is_in() {
    let calls = [
        call("git commit -m done", Some("Commit the fix")),
        call("git log -1", Some("Check the commit landed")),
    ];
    let bigrams = grams(&calls, 2);
    assert_eq!(bigrams.len(), 1);
    assert_eq!(bigrams[0].0, "Commit | History");
    assert_eq!(bigrams[0].1, Some("Check the commit landed"));
}

#[test]
fn every_window_is_a_gram_and_a_short_episode_has_none() {
    let calls = [call("cat a.ts && cat b.ts && cat c.ts", None)];
    assert_eq!(grams(&calls, 1).len(), 3);
    assert_eq!(grams(&calls, 2).len(), 2);
    assert_eq!(grams(&calls, 3).len(), 1);
    assert_eq!(grams(&calls, 4).len(), 0);
    assert_eq!(grams(&calls, 0).len(), 0);
}

#[test]
fn letters_name_subjects_in_order_of_first_appearance_across_the_gram() {
    let a = Token {
        label: "X".to_string(),
        subjects: vec!["/p".to_string(), "/q".to_string()],
    };
    let b = Token {
        label: "Y".to_string(),
        subjects: vec!["/q".to_string(), "/r".to_string(), "/p".to_string()],
    };
    assert_eq!(key(&[(&a, false), (&b, true)]), "X(A,B) | Y(B,C,A)");
}

#[test]
fn a_pager_on_a_pipe_is_how_the_act_before_it_is_shown() {
    let calls = [call("grep -n x f.ts | head -5", None)];
    assert_eq!(only(&calls, 1), "Search(A)");
    let counted = tokens(
        &trace(
            &parse("grep -n x f.ts | head -5").expect("parses"),
            Some(CWD),
            HOME,
        )
        .steps,
    );
    assert_eq!((counted.tokens.len(), counted.folded), (1, 1));
}

#[test]
fn a_stream_act_that_changes_the_product_is_kept_as_a_suffix() {
    let count = [call("cat f.ts | wc -l", None)];
    assert_eq!(only(&count, 1), "Page+Measure(A)");
    let filter = [call("grep x f.ts | grep -v y", None)];
    assert!(
        only(&filter, 1).starts_with("Search+search · grep -v"),
        "{}",
        only(&filter, 1)
    );
}

#[test]
fn a_stream_act_with_nothing_before_it_stands_alone() {
    let calls = [call("echo x | head -1", None)];
    assert_eq!(only(&calls, 1), "Page");
}

#[test]
fn a_program_that_touches_no_file_is_still_an_act_and_its_pager_folds() {
    let calls = [call("kubectl get pods -A 2>&1 | head -5", None)];
    let k = only(&calls, 1);
    assert!(k.starts_with("nothing with files · kubectl get"), "{k}");
    let calls = [call(
        "sleep 5; gh run list --limit 2 2>&1 | head -20; git rev-parse HEAD",
        None,
    )];
    let keys: Vec<String> = grams(&calls, 1).into_iter().map(|(k, _)| k).collect();
    assert_eq!(keys.len(), 2, "{keys:?}");
    assert!(
        keys[0].starts_with("nothing with files · gh run"),
        "{keys:?}"
    );
}

#[test]
fn a_loops_head_is_furniture_and_its_body_the_acts() {
    let calls = [call("for i in 1 2; do cat f.ts; done", None)];
    assert_eq!(only(&calls, 2), "Page(A) ; Page(A)");
    let calls = [call("for i in $(seq 1 3); do sleep 1; done", None)];
    assert!(calls[0].tokens.is_empty(), "{:?}", calls[0].tokens);
}

#[test]
fn a_filter_the_table_calls_nothing_is_a_stream_act() {
    let calls = [call("cat f.ts | tr -d '\\n'", None)];
    let k = only(&calls, 1);
    assert!(k.starts_with("Page+nothing with files · tr"), "{k}");
}
