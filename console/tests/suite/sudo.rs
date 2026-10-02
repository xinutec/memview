//! The root password's way from the phone to `sudo`: handed to the request
//! waiting for it once, and a refusal is no password.

use console::sudo::Waiting;

#[tokio::test]
async fn a_password_reaches_the_request_waiting_for_it_once() {
    let waiting = Waiting::default();
    let answer = waiting.wait("sudo-1");
    assert!(waiting.answer("sudo-1", Some("hunter2".to_string())));
    assert_eq!(answer.await.unwrap().as_deref(), Some("hunter2"));
    // Taken: a second answer, or one for nothing, finds nobody.
    assert!(!waiting.answer("sudo-1", Some("again".to_string())));
    assert!(!waiting.answer("sudo-2", None));
}

#[tokio::test]
async fn a_refusal_is_no_password() {
    let waiting = Waiting::default();
    let answer = waiting.wait("sudo-1");
    assert!(waiting.answer("sudo-1", None));
    assert_eq!(answer.await.unwrap(), None);
}
