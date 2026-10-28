//! Where each message stands, as the CLI's own word moves it — and what a
//! transcript remembers of messages an earlier process was handed. The steps are
//! the CLI's, measured on 2.1.289; see `console/src/session/messages.rs`.

use console::protocol::Lifecycle;
use console::session::{Messages, Stands, Step};

const ID: &str = "113278ac-cc76-4e4e-ad0b-17d85afa59ca";

fn written() -> Messages {
    let mut messages = Messages::default();
    messages.add(ID, "rename the branch", Vec::new(), Step::Written, 1_000);
    messages
}

fn step(messages: &Messages) -> Step {
    messages.get(ID).expect("the message").step
}

#[test]
fn a_message_moves_through_the_queue_as_the_cli_says() {
    let mut messages = written();
    messages.stepped(ID, Lifecycle::Queued, 2_000);
    assert_eq!(step(&messages), Step::Queued);
    messages.stepped(ID, Lifecycle::Started, 3_000);
    assert_eq!(
        step(&messages),
        Step::Read {
            at: 3_000,
            echoed: false
        }
    );
}

#[test]
fn read_stays_read_when_an_aborted_turn_reports_it_cancelled() {
    // The CLI calls a message cancelled when the turn that took it in was aborted;
    // it was seen all the same.
    let mut messages = written();
    messages.stepped(ID, Lifecycle::Started, 3_000);
    messages.stepped(ID, Lifecycle::Cancelled, 4_000);
    assert!(matches!(step(&messages), Step::Read { .. }));
}

#[test]
fn a_cancel_takes_a_waiting_message_back() {
    let mut messages = written();
    messages.stepped(ID, Lifecycle::Queued, 2_000);
    messages.stepped(ID, Lifecycle::Cancelled, 3_000);
    assert_eq!(step(&messages), Step::Withdrawn);
}

#[test]
fn a_second_echo_of_a_message_is_not_shown_again() {
    // The CLI echoes a retried id it ignores; `started` comes before the first
    // echo, which must still be shown.
    let mut messages = written();
    messages.stepped(ID, Lifecycle::Started, 3_000);
    assert!(!messages.echoed(ID, 3_100), "the first echo is shown");
    assert!(messages.echoed(ID, 3_200), "the retry's echo is not");
}

#[test]
fn an_echo_without_an_id_reads_the_oldest_message_with_those_words() {
    let mut messages = Messages::default();
    messages.add("a", "yes", Vec::new(), Step::Written, 1_000);
    messages.add("b", "yes", Vec::new(), Step::Written, 1_000);
    messages.echoed_words("yes", 2_000);
    assert!(matches!(
        messages.get("a").expect("a").step,
        Step::Read { .. }
    ));
    assert_eq!(messages.get("b").expect("b").step, Step::Written);
}

#[test]
fn a_dead_process_drops_what_it_held_or_had_not_read() {
    let mut messages = written();
    messages.add("held", "/compact", Vec::new(), Step::Held, 1_000);
    messages.add(
        "read",
        "done",
        Vec::new(),
        Step::Read {
            at: 1_000,
            echoed: true,
        },
        1_000,
    );
    messages.drop_waiting();
    assert_eq!(step(&messages), Step::Dropped);
    assert_eq!(messages.get("held").expect("held").step, Step::Dropped);
    assert!(matches!(
        messages.get("read").expect("read").step,
        Step::Read { .. }
    ));
}

#[test]
fn the_phone_is_shown_what_is_not_yet_read_oldest_first() {
    let mut messages = Messages::default();
    messages.add("held", "/compact", Vec::new(), Step::Held, 1_000);
    messages.add("unread", "rename", Vec::new(), Step::Queued, 2_000);
    messages.add(
        "read",
        "done",
        Vec::new(),
        Step::Read {
            at: 3_000,
            echoed: true,
        },
        3_000,
    );
    messages.add("back", "gone", Vec::new(), Step::Withdrawn, 4_000);
    messages.add("dropped", "lost", Vec::new(), Step::Dropped, 5_000);
    let shown: Vec<_> = messages
        .waiting()
        .into_iter()
        .map(|waiting| (waiting.id, waiting.stands))
        .collect();
    assert_eq!(
        shown,
        [
            ("held".to_string(), Stands::Held),
            ("unread".to_string(), Stands::Unread),
            ("dropped".to_string(), Stands::Dropped)
        ]
    );
}

#[test]
fn a_settled_message_is_forgotten_after_a_week_and_a_waiting_one_never() {
    let week = 7 * 24 * 60 * 60 * 1000;
    let mut messages = Messages::default();
    messages.add("back", "gone", Vec::new(), Step::Withdrawn, 0);
    messages.add("queued", "still", Vec::new(), Step::Queued, 0);
    messages.add("later", "new", Vec::new(), Step::Written, week + 1);
    assert!(messages.get("back").is_none());
    assert!(messages.get("queued").is_some());
}

#[test]
fn the_record_reads_back_what_it_wrote() {
    let mut messages = written();
    messages.stepped(ID, Lifecycle::Queued, 2_000);
    let text = serde_json::to_string(&messages).expect("written");
    let read: Messages = serde_json::from_str(&text).expect("read");
    assert_eq!(read.get(ID).map(|it| it.step), Some(Step::Queued));
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

#[test]
fn the_transcript_answers_for_a_message_read_mid_turn() {
    // Delivered into a running turn, a message is recorded as a `queued_command`
    // attachment carrying the client's id as `source_uuid`.
    let dir = std::env::temp_dir().join(format!("sent-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("dir");
    let transcript = dir.join("t.jsonl");
    let id = "a9cefb2b-8fe7-4680-886a-f352ef349252";
    std::fs::write(
        &transcript,
        format!(
            "{{\"parentUuid\":\"e0aa92fb-9b56-46dc-a92f-b663107d4f0b\",\"attachment\":{{\"type\":\"queued_command\",\"prompt\":[{{\"text\":\"That's weird\",\"type\":\"text\"}}],\"source_uuid\":\"{id}\",\"commandMode\":\"prompt\"}},\"type\":\"attachment\",\"uuid\":\"394f7f32-4151-4c65-8f0f-6030a9a63d9c\",\"timestamp\":\"2026-10-26T17:07:44.995Z\"}}\n"
        ),
    )
    .expect("transcript");
    assert_eq!(
        console::sent::recorded_at(&transcript, id),
        Some(1_793_034_464_995)
    );
}
