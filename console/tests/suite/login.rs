//! Signing in from the phone: the link `claude auth login` prints, and the code
//! handed back to it. Against a stub that speaks as the CLI does with no terminal.

use std::path::PathBuf;

use console::login::{Login, LoginCode};

fn stub() -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/stub-cli")
        .display()
        .to_string()
}

fn code(code: &str) -> LoginCode {
    LoginCode {
        code: code.to_string(),
    }
}

#[tokio::test]
async fn the_link_comes_back_and_a_good_code_signs_in() {
    let login = Login::default();
    let started = login.start(&stub()).await.expect("a link");
    assert_eq!(
        started.url,
        "https://claude.com/cai/oauth/authorize?code=true&state=stub"
    );
    // Pasted from a phone, with the newline a text field can leave behind.
    let done = login.finish(code(" good-code\n")).await.expect("an answer");
    assert!(done.ok, "{}", done.said);
    assert!(done.said.contains("Login successful"), "{}", done.said);
    assert!(
        !done.said.contains("oauth/authorize"),
        "only what followed the code"
    );
}

#[tokio::test]
async fn a_wrong_code_says_why() {
    let login = Login::default();
    login.start(&stub()).await.expect("a link");
    let done = login.finish(code("wrong")).await.expect("an answer");
    assert!(!done.ok);
    assert!(done.said.contains("invalid code"), "{}", done.said);
}

#[tokio::test]
async fn a_code_with_nothing_waiting_is_refused() {
    let login = Login::default();
    assert!(login.finish(code("good-code")).await.is_err());
    // And a finished attempt is not waiting either.
    login.start(&stub()).await.expect("a link");
    login.finish(code("good-code")).await.expect("an answer");
    assert!(login.finish(code("good-code")).await.is_err());
}

#[tokio::test]
async fn starting_again_replaces_the_attempt() {
    let login = Login::default();
    login.start(&stub()).await.expect("a link");
    login.start(&stub()).await.expect("a second link");
    let done = login.finish(code("good-code")).await.expect("an answer");
    assert!(done.ok, "{}", done.said);
}

#[tokio::test]
async fn a_binary_that_cannot_start_says_so() {
    let login = Login::default();
    let refused = login.start("/nonexistent/claude").await;
    assert!(matches!(refused, Err(why) if why.contains("could not start")));
}
