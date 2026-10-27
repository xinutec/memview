//! A message sent under the client's id: delivered once however often it is
//! retried, and taken back only while it can be.

use console::sent::{Cancel, Fate, Refusal, Send, Sent, Where, Why, cancelling, sending};

const DELIVERED: Where = Where::Delivered { at: 1_000 };

#[test]
fn a_new_message_is_written() {
    assert_eq!(sending(None, Where::Nowhere), Send::Write);
}

#[test]
fn a_retry_after_a_lost_receipt_is_answered_without_a_second_message() {
    // Sent, delivered, and the answer lost on the way back: the phone retries with
    // the same id, and the session already has it.
    assert_eq!(sending(Some(Fate::Written), DELIVERED), Send::AlreadyHave);
    // Or it has been handed over and not read yet, or is held for the turn's end.
    assert_eq!(
        sending(Some(Fate::Written), Where::Unread),
        Send::AlreadyHave
    );
    assert_eq!(sending(Some(Fate::Written), Where::Held), Send::AlreadyHave);
}

#[test]
fn a_message_in_the_transcript_but_not_the_record_is_still_not_sent_twice() {
    // The record lost or expired, the transcript still has it under its id.
    assert_eq!(sending(None, DELIVERED), Send::AlreadyHave);
}

#[test]
fn a_message_written_to_a_session_that_died_unread_is_written_again() {
    // The CLI ignores an id it already has, so writing again cannot double it.
    assert_eq!(sending(Some(Fate::Written), Where::Nowhere), Send::Write);
}

#[test]
fn a_late_retry_of_a_cancelled_message_is_refused() {
    // A slow retry must not land after the edit that replaced it.
    assert!(matches!(
        sending(Some(Fate::Cancelled), Where::Nowhere),
        Send::Refuse(Refusal {
            why: Why::Cancelled,
            ..
        })
    ));
}

#[test]
fn a_message_that_got_there_cannot_be_taken_back_and_says_when() {
    // Sent, delivered, receipt lost, then cancelled from the phone: refused, with
    // when it got there, so the phone does not pretend.
    assert_eq!(
        cancelling(Some(Fate::Written), DELIVERED),
        Cancel::Refuse(Refusal {
            why: Why::Delivered,
            at: Some(1_000),
            original: false,
        })
    );
}

#[test]
fn a_message_handed_to_the_session_cannot_be_taken_back() {
    assert!(matches!(
        cancelling(Some(Fate::Written), Where::Unread),
        Cancel::Refuse(Refusal {
            why: Why::InFlight,
            ..
        })
    ));
}

#[test]
fn a_held_command_can_be_taken_back() {
    assert_eq!(
        cancelling(Some(Fate::Written), Where::Held),
        Cancel::TakeBack
    );
}

#[test]
fn a_message_that_never_arrived_is_forgotten_and_a_second_cancel_is_done() {
    assert_eq!(cancelling(None, Where::Nowhere), Cancel::Forget);
    assert_eq!(
        cancelling(Some(Fate::Cancelled), Where::Nowhere),
        Cancel::Done
    );
}

#[test]
fn the_record_outlives_the_console() {
    // A cancelled id must still be refused after a restart.
    let dir = std::env::temp_dir().join(format!("sent-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("dir");
    let file = dir.join("sent.json");
    Sent::load(file.clone()).record("m-42", "s", Fate::Cancelled, 1_000);
    assert_eq!(Sent::load(file).fate("m-42"), Some(Fate::Cancelled));
}

#[test]
fn ids_older_than_a_week_are_forgotten() {
    let dir = std::env::temp_dir().join(format!("sent-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("dir");
    let sent = Sent::load(dir.join("sent.json"));
    sent.record("old", "s", Fate::Cancelled, 0);
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
