//! Unsent words, shared between the devices that talk to this runner.
//!
//! What is covered here is the half that cannot be seen on a screen: that two
//! devices editing at once end up with ONE text holding both edits, and that
//! nothing in the protocol can refuse a write. The composer is covered at phone
//! width, and the round trip through real browsers is
//! `frontend/projects/console-web/e2e/two-devices.spec.ts`.
//!
//! ⚠ **The old suite spent thirteen tests on which push LOSES.** There is no
//! losing side any more — see the note at the top of `console/src/drafts.rs` for
//! the measurement that ended that design.

use std::collections::BTreeSet;

use console::drafts::{Drafts, PushEntry, document_of, from_base64};

/// A scratch directory of this test's own, named for the case, as `gist.rs`
/// does it — the pid keeps two runs of the binary apart.
fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("console-drafts-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch");
    dir
}

fn store(dir: &std::path::Path) -> Drafts {
    Drafts::load(dir.join("drafts.json"))
}

/// A device that has typed `text` into a document of its own, as bytes to push.
///
/// ⚠ **Each call makes a NEW document**, which is what makes two of them
/// concurrent: neither knows anything about the other's edits, exactly as two
/// phones that have not synced do not.
fn typed(text: &str) -> Vec<u8> {
    document_of(text)
}

/// What the runner holds for `id`, as words.
fn words(drafts: &Drafts, id: &str) -> String {
    drafts.get(id).map(|d| d.text).unwrap_or_default()
}

fn base64(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// The ordinary case, and the one the feature exists for: written on one
/// device, read on the other.
#[test]
fn a_draft_written_on_one_device_is_read_by_the_other() {
    let dir = scratch("read-across");
    let drafts = store(&dir);
    assert_eq!(drafts.get("s1"), None, "a session nobody has typed in");

    let mine = drafts
        .merge("s1", &typed("half a thought"), 1000)
        .expect("a document");
    assert_eq!(mine.rev, 1);
    assert_eq!(mine.text, "half a thought");

    let theirs = drafts.get("s1").expect("the phone asks and is told");
    assert_eq!(theirs.text, "half a thought");
    assert_eq!(theirs.at, 1000);
}

/// ⚠ **The headline, and the whole reason for this design.** Two devices that
/// each wrote without seeing the other keep BOTH texts. Nobody is asked to
/// choose, and nothing is thrown away.
#[test]
fn two_devices_writing_at_once_keep_both_edits() {
    let dir = scratch("merge");
    let drafts = store(&dir);

    drafts.merge("s1", &typed("from the mac"), 1000).unwrap();
    drafts.merge("s1", &typed("from the phone"), 1001).unwrap();

    let held = words(&drafts, "s1");
    assert!(
        held.contains("from the mac"),
        "the first device's words were dropped: {held:?}"
    );
    assert!(
        held.contains("from the phone"),
        "the second device's words were dropped: {held:?}"
    );
}

/// The property the whole thing rests on: an update applied twice is an update
/// applied once. A retry after a timeout must not double the words.
#[test]
fn the_same_update_arriving_twice_changes_nothing() {
    let dir = scratch("idempotent");
    let drafts = store(&dir);
    let update = typed("said once");

    drafts.merge("s1", &update, 1000).unwrap();
    let after_one = words(&drafts, "s1");
    drafts.merge("s1", &update, 1001).unwrap();

    assert_eq!(words(&drafts, "s1"), after_one, "a retry doubled the words");
    assert_eq!(after_one, "said once");
}

/// And the other half: the order they arrive in cannot matter, or two devices on
/// a slow link would settle on different texts.
#[test]
fn the_order_updates_arrive_in_does_not_change_the_result() {
    let one = typed("alpha");
    let two = typed("beta");

    let forwards = scratch("order-forwards");
    let a = store(&forwards);
    a.merge("s1", &one, 1000).unwrap();
    a.merge("s1", &two, 1001).unwrap();

    let backwards = scratch("order-backwards");
    let b = store(&backwards);
    b.merge("s1", &two, 1000).unwrap();
    b.merge("s1", &one, 1001).unwrap();

    assert_eq!(
        words(&a, "s1"),
        words(&b, "s1"),
        "two runners given the same edits in different orders disagree"
    );
}

/// A cleared draft is an empty text, not a missing row: the entry stays so the
/// other device learns the words are gone rather than pushing them back.
#[test]
fn a_cleared_draft_stays_as_an_empty_one() {
    let dir = scratch("cleared");
    let drafts = store(&dir);
    let written = typed("about to be sent");
    drafts.merge("s1", &written, 1000).unwrap();

    // Clearing is an edit like any other: the device deletes what it wrote and
    // pushes the document that results.
    drafts.merge("s1", &emptied(&written), 1001).unwrap();

    assert_eq!(words(&drafts, "s1"), "");
    assert!(
        drafts.get("s1").is_some(),
        "the row went with the words, so the other device could push them back"
    );
}

/// Everything survives the console being restarted, because the phone may be the
/// only other copy and it may be asleep.
#[test]
fn a_draft_survives_the_console_restarting() {
    let dir = scratch("restart");
    {
        let drafts = store(&dir);
        drafts
            .merge("s1", &typed("written before the restart"), 1000)
            .unwrap();
    }
    let after = store(&dir);
    assert_eq!(words(&after, "s1"), "written before the restart");
    // And it can still be merged into, which a state that failed to decode could not.
    after.merge("s1", &typed("and after"), 1001).unwrap();
    let held = words(&after, "s1");
    assert!(held.contains("written before the restart"), "{held:?}");
    assert!(held.contains("and after"), "{held:?}");
}

#[test]
fn conversations_gone_from_disk_are_forgotten_but_an_empty_sweep_forgets_nothing() {
    let dir = scratch("forget");
    let drafts = store(&dir);
    drafts.merge("alive", &typed("keep me"), 1000).unwrap();
    drafts.merge("gone", &typed("drop me"), 1001).unwrap();

    drafts.forget(&BTreeSet::new());
    assert!(
        drafts.get("gone").is_some(),
        "an empty sweep found nothing; it is not evidence everything has gone"
    );

    drafts.forget(&BTreeSet::from(["alive".to_string()]));
    assert!(drafts.get("alive").is_some());
    assert_eq!(drafts.get("gone"), None);
}

#[test]
fn a_pull_answers_only_past_the_checkpoint_and_says_how_far_it_got() {
    let dir = scratch("pull");
    let drafts = store(&dir);
    drafts.merge("s1", &typed("first"), 1000).unwrap();
    let after_first = drafts.pull(0).checkpoint.rev;

    drafts.merge("s2", &typed("second"), 1001).unwrap();
    let page = drafts.pull(after_first);
    assert_eq!(
        page.documents.len(),
        1,
        "everything came back, not the page"
    );
    assert_eq!(page.documents[0].ulid, "s2");
    assert!(page.checkpoint.rev > after_first);

    // Caught up: nothing to send, and the cursor does not go backwards.
    let nothing = drafts.pull(page.checkpoint.rev);
    assert!(nothing.documents.is_empty());
    assert_eq!(nothing.checkpoint.rev, page.checkpoint.rev);
}

/// ⚠ **The counter is store-wide on purpose.** A per-conversation one left a
/// fresh conversation at rev 1 behind a client already at 3, so it was never
/// delivered — and looked random from outside.
#[test]
fn a_draft_in_a_new_conversation_reaches_a_client_that_is_already_ahead() {
    let dir = scratch("ahead");
    let drafts = store(&dir);
    for n in 0..3 {
        drafts
            .merge("busy", &typed(&format!("edit {n}")), 1000 + n)
            .unwrap();
    }
    let caught_up = drafts.pull(0).checkpoint.rev;

    drafts
        .merge("fresh", &typed("the first word here"), 2000)
        .unwrap();
    let page = drafts.pull(caught_up);
    assert_eq!(
        page.documents
            .iter()
            .map(|d| d.ulid.as_str())
            .collect::<Vec<_>>(),
        ["fresh"],
        "a new conversation did not reach a client already ahead of it"
    );
}

/// A push answers with the MERGED document, so the client is level without a
/// second round trip — and what it carries is what the merge produced, not what
/// was sent.
#[test]
fn a_push_answers_with_what_the_merge_produced() {
    let dir = scratch("push");
    let drafts = store(&dir);
    drafts.merge("s1", &typed("already here"), 1000).unwrap();

    let back = drafts.push(vec![PushEntry {
        ulid: "s1".to_string(),
        update: base64(&typed("and this too")),
        at: 1001,
    }]);

    assert_eq!(back.len(), 1);
    assert!(back[0].text.contains("already here"), "{:?}", back[0].text);
    assert!(back[0].text.contains("and this too"), "{:?}", back[0].text);
    // And it carries a document the sender can apply, not just the words.
    assert!(!from_base64(&back[0].update).expect("base64").is_empty());
}

/// Nonsense over the wire is dropped, not fatal: it arrives from a client that
/// may be older than this binary, and one bad push must not take the store with it.
#[test]
fn a_push_that_is_not_a_document_is_dropped_and_harms_nothing() {
    let dir = scratch("rubbish");
    let drafts = store(&dir);
    drafts.merge("s1", &typed("good words"), 1000).unwrap();

    let back = drafts.push(vec![
        PushEntry {
            ulid: "s1".to_string(),
            update: "not base64 at all!!".to_string(),
            at: 1001,
        },
        PushEntry {
            ulid: "s1".to_string(),
            update: base64(b"base64, but not a document"),
            at: 1002,
        },
    ]);

    assert!(back.is_empty(), "rubbish was answered as though it landed");
    assert_eq!(
        words(&drafts, "s1"),
        "good words",
        "a bad push damaged the draft it named"
    );
}

/// A merge that changes no character does not move the clock the list dates a
/// draft by — a device can contribute history without anybody having typed.
#[test]
fn a_merge_that_changes_nothing_leaves_the_clock_alone() {
    let dir = scratch("clock");
    let drafts = store(&dir);
    let update = typed("unchanged");
    drafts.merge("s1", &update, 1000).unwrap();
    drafts.merge("s1", &update, 9999).unwrap();
    assert_eq!(drafts.get("s1").unwrap().at, 1000);
}

/// The document that results from writing something and then deleting all of it
/// — what a device pushes when the box is emptied.
fn emptied(had: &[u8]) -> Vec<u8> {
    use yrs::updates::decoder::Decode;
    use yrs::{GetString, ReadTxn, Text, Transact};
    let doc = yrs::Doc::new();
    doc.transact_mut()
        .apply_update(yrs::Update::decode_v1(had).expect("a document"))
        .expect("apply");
    let text = doc.get_or_insert_text(console::drafts::TEXT);
    {
        let mut txn = doc.transact_mut();
        let len = u32::try_from(text.get_string(&txn).chars().count()).expect("a draft's length");
        text.remove_range(&mut txn, 0, len);
    }
    doc.transact()
        .encode_state_as_update_v1(&yrs::StateVector::default())
}

/// ⚠ **With bytes a real browser produced**, not ones this process made. Two
/// documents merging in Rust is not evidence that a document Yjs wrote merges
/// into one yrs holds — and that is the pair production actually has.
#[test]
fn an_update_from_the_browser_merges_into_one_the_runner_holds() {
    let dir = scratch("browser");
    let drafts = store(&dir);
    // Captured from the phone in `e2e/two-devices.spec.ts`: a document whose only
    // content is `from the phone`.
    const FROM_A_BROWSER: &str = "AQGb3cjABwAEAQR0ZXh0DmZyb20gdGhlIHBob25lAA==";

    drafts.merge("s1", &typed("from the mac"), 1000).unwrap();
    let update = console::drafts::from_base64(FROM_A_BROWSER).expect("base64");
    drafts.merge("s1", &update, 1001).expect("a document");

    let held = words(&drafts, "s1");
    assert!(
        held.contains("from the mac"),
        "the runner's own words went: {held:?}"
    );
    assert!(
        held.contains("from the phone"),
        "the browser's words went: {held:?}"
    );
}
