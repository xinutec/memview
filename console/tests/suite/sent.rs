//! A message sent under the client's id: delivered once however often it is
//! retried, and taken back only while the session has not read it.

use console::sent::{Fate, Refusal, Send, Sent, Where, Why, Withdraw, sending, withdrawing};

const READ: Where = Where::Delivered { at: 1_000 };

#[test]
fn a_new_message_is_written() {
    assert_eq!(sending(None, Where::Nowhere), Send::Write);
}

#[test]
fn a_retry_after_a_lost_receipt_is_answered_without_a_second_message() {
    // Sent, and the answer lost on the way back: the phone retries with the same
    // id, and the message is already held, queued, or read.
    for there in [Where::Held, Where::Unread, READ] {
        assert_eq!(sending(Some(Fate::Written), there), Send::AlreadyHave);
    }
}

#[test]
fn a_message_in_the_transcript_but_not_the_record_is_still_not_sent_twice() {
    // The record lost or expired, the transcript still has it under its id.
    assert_eq!(sending(None, READ), Send::AlreadyHave);
}

#[test]
fn a_message_written_to_a_session_that_died_unread_is_written_again() {
    // The CLI ignores an id it already has, so writing again cannot double it.
    assert_eq!(sending(Some(Fate::Written), Where::Nowhere), Send::Write);
}

#[test]
fn a_late_retry_of_a_message_taken_back_is_refused() {
    // The person has its words back, and may be rewriting them.
    assert_eq!(
        sending(Some(Fate::Withdrawn), Where::Nowhere),
        Send::Refuse(Refusal {
            why: Why::Withdrawn,
            at: None
        })
    );
}

#[test]
fn a_message_the_session_read_cannot_be_taken_back_and_says_when() {
    assert_eq!(
        withdrawing(Some(Fate::Written), READ),
        Withdraw::Refuse(Refusal {
            why: Why::Read,
            at: Some(1_000)
        })
    );
}

#[test]
fn a_queued_message_is_asked_of_the_cli() {
    // Only the CLI knows whether a turn has taken it in.
    assert_eq!(
        withdrawing(Some(Fate::Written), Where::Unread),
        Withdraw::AskCli
    );
}

#[test]
fn a_held_command_is_taken_back_here() {
    assert_eq!(
        withdrawing(Some(Fate::Written), Where::Held),
        Withdraw::TakeBack
    );
}

#[test]
fn a_message_that_never_arrived_is_forgotten_and_a_second_withdrawal_is_done() {
    assert_eq!(withdrawing(None, Where::Nowhere), Withdraw::Forget);
    assert_eq!(
        withdrawing(Some(Fate::Withdrawn), Where::Nowhere),
        Withdraw::Done
    );
}

#[test]
fn the_record_outlives_the_console() {
    // A message taken back must still be refused after a restart.
    let dir = std::env::temp_dir().join(format!("sent-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("dir");
    let file = dir.join("sent.json");
    Sent::load(file.clone()).record("m-42", "s", Fate::Withdrawn, 1_000);
    assert_eq!(Sent::load(file).fate("m-42"), Some(Fate::Withdrawn));
}

#[test]
fn a_record_written_before_withdrawals_still_reads() {
    // The first build called a taken-back id cancelled.
    let dir = std::env::temp_dir().join(format!("sent-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("dir");
    let file = dir.join("sent.json");
    std::fs::write(
        &file,
        r#"{"m-42":{"session":"s","at":1000,"fate":"cancelled"}}"#,
    )
    .expect("file");
    assert_eq!(Sent::load(file).fate("m-42"), Some(Fate::Withdrawn));
}

#[test]
fn ids_older_than_a_week_are_forgotten() {
    let dir = std::env::temp_dir().join(format!("sent-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("dir");
    let sent = Sent::load(dir.join("sent.json"));
    sent.record("old", "s", Fate::Withdrawn, 0);
    sent.record("new", "s", Fate::Written, console::sent::KEEP_MS);
    assert_eq!(sent.fate("old"), None);
    assert_eq!(sent.fate("new"), Some(Fate::Written));
}

#[test]
fn the_transcript_answers_for_a_message_an_earlier_process_wrote() {
    // The CLI records a client's message under its id. A line mentioning the id
    // as another line's parent is not that message.
    let dir = std::env::temp_dir().join(format!("sent-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("dir");
    let transcript = dir.join("t.jsonl");
    let id = "113278ac-cc76-4e4e-ad0b-17d85afa59ca";
    std::fs::write(
        &transcript,
        format!(
            "{{\"parentUuid\":null,\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"hi\"}},\"uuid\":\"{id}\",\"timestamp\":\"2026-10-27T15:00:00.000Z\"}}\n\
             {{\"parentUuid\":\"{id}\",\"type\":\"assistant\",\"uuid\":\"f755ed6d-912f-4f4f-99aa-6cd37466b2f0\",\"timestamp\":\"2026-10-27T15:00:02.000Z\"}}\n"
        ),
    )
    .expect("transcript");
    assert_eq!(
        console::sent::recorded_at(&transcript, id),
        Some(1_793_113_200_000)
    );
    assert_eq!(
        console::sent::recorded_at(&transcript, "f755ed6d-912f-4f4f-99aa-6cd37466b2f0"),
        None,
        "an assistant line is not a message the client sent"
    );
    assert_eq!(
        console::sent::recorded_at(&transcript, "00000000-0000-0000-0000-000000000000"),
        None
    );
}
