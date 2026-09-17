//! Unsent words, shared between the two devices that talk to this runner.
//!
//! What is covered here is the half that cannot be seen on a screen: which
//! writes are taken, which are refused, and what a refusal hands back. The
//! conflict SCREEN is a layout question and belongs to the phone-width harness;
//! whether a conflict is detected at all is decided here.

use std::collections::BTreeSet;

use console::drafts::{DraftDoc, Drafts, PushEntry, Wrote};

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

/// The ordinary case, and the one the feature exists for: written on one
/// device, read on the other.
#[test]
fn a_draft_written_on_one_device_is_read_by_the_other() {
    let dir = scratch("read-across");
    let drafts = store(&dir);
    assert_eq!(drafts.get("s1"), None, "a session nobody has typed in");

    let Wrote::Stored(mine) = drafts.apply("s1", "half a thought", None, 1000) else {
        panic!("a first write cannot conflict with nothing");
    };
    assert_eq!(mine.rev, 1);

    let theirs = drafts.get("s1").expect("the phone asks and is told");
    assert_eq!(theirs.text, "half a thought");
    assert_eq!(
        theirs.at, 1000,
        "the time is what tells the two drafts apart on the screen"
    );
}

/// ⚠ **The write that must be refused.** Two devices editing from the same text
/// is the whole reason an assumed state is carried.
#[test]
fn an_edit_from_text_somebody_has_moved_past_is_refused_and_hands_back_theirs() {
    let dir = scratch("stale");
    let drafts = store(&dir);
    drafts.apply("s1", "mac words", None, 1000);

    // The phone has read "mac words", then the Mac wrote again.
    let Wrote::Stored(second) = drafts.apply("s1", "mac words, more", Some("mac words"), 2000)
    else {
        panic!("the Mac is editing from what it just wrote");
    };
    assert_eq!(second.rev, 2);

    match drafts.apply("s1", "phone words", Some("mac words"), 3000) {
        Wrote::Conflict(theirs) => {
            assert_eq!(
                theirs.text, "mac words, more",
                "a refusal must carry THEIRS"
            );
            assert_eq!(theirs.rev, 2);
        }
        Wrote::Stored(_) => panic!("a stale edit silently won, which is the data loss"),
    }
    assert_eq!(
        drafts.get("s1").expect("still there").text,
        "mac words, more",
        "the refused text must not have landed",
    );
}

/// ⚠ **A sent message must not come back.** Sending clears the composer, which
/// arrives as an empty write; the other device still holds the words at the old
/// revision and will push them.
#[test]
fn a_cleared_draft_is_a_tombstone_so_the_other_device_cannot_resurrect_it() {
    let dir = scratch("tombstone");
    let drafts = store(&dir);
    drafts.apply("s1", "the message", None, 1000);
    // The Mac sends it: composer empties, and that is a write like any other.
    let Wrote::Stored(cleared) = drafts.apply("s1", "", Some("the message"), 2000) else {
        panic!("clearing is an ordinary edit from the text that was there");
    };
    assert_eq!(cleared.rev, 2);
    assert_eq!(cleared.text, "");

    // The phone, still assuming the words it is holding, pushes them back.
    match drafts.apply("s1", "the message", Some("the message"), 3000) {
        Wrote::Conflict(theirs) => assert_eq!(
            theirs.text, "",
            "theirs is the cleared draft, so the person is asked rather than surprised",
        ),
        Wrote::Stored(_) => panic!("a message already sent was resurrected"),
    }
}

/// A first write cannot erase one it has never seen.
#[test]
fn a_device_that_has_never_seen_the_draft_cannot_overwrite_it_by_being_first() {
    let dir = scratch("first-write");
    let drafts = store(&dir);
    drafts.apply("s1", "mac words", None, 1000);
    match drafts.apply("s1", "phone words", None, 2000) {
        Wrote::Conflict(theirs) => assert_eq!(theirs.text, "mac words"),
        Wrote::Stored(_) => panic!("a phone that had never looked erased the Mac's draft"),
    }
}

/// Across a restart, which is what a file is for — the console is upgraded
/// several times an evening.
#[test]
fn a_draft_survives_the_console_restarting() {
    let dir = scratch("restart");
    store(&dir).apply("s1", "written before the upgrade", None, 1000);
    let after = store(&dir);
    assert_eq!(
        after.get("s1").expect("read back from disk").text,
        "written before the upgrade",
    );
}

/// ⚠ **An empty sweep is not evidence that every conversation has gone.** Same
/// argument as the sentences: a sweep that found nothing must forget nothing.
#[test]
fn conversations_gone_from_disk_are_forgotten_but_an_empty_sweep_forgets_nothing() {
    let dir = scratch("forget");
    let drafts = store(&dir);
    drafts.apply("gone", "words", None, 1000);
    drafts.apply("alive", "words", None, 1000);

    drafts.forget(&BTreeSet::new());
    assert!(drafts.get("gone").is_some(), "an empty sweep swept");

    drafts.forget(&BTreeSet::from(["alive".to_string()]));
    assert!(
        drafts.get("gone").is_none(),
        "a dead conversation kept its draft"
    );
    assert!(drafts.get("alive").is_some());
}

/// A document as a client would push one.
fn doc(id: &str, text: &str, rev: u64) -> DraftDoc {
    DraftDoc {
        ulid: id.to_string(),
        text: text.to_string(),
        at: 1000,
        deleted: false,
        rev,
    }
}

/// ⚠ **The checkpoint is what makes a pull resumable**, and it must not move
/// past what was actually delivered — a client that checkpointed ahead of its
/// rows would never see the ones it skipped.
#[test]
fn a_pull_answers_only_past_the_checkpoint_and_says_how_far_it_got() {
    let dir = scratch("pull");
    let drafts = store(&dir);
    drafts.apply("s1", "one", None, 1000);
    drafts.apply("s2", "two", None, 1000);

    let all = drafts.pull(0);
    assert_eq!(all.documents.len(), 2, "a first pull takes everything");
    assert_eq!(
        all.checkpoint.rev, 2,
        "the cursor counts writes across the store, so two drafts are 1 and 2",
    );

    // Nothing has moved since.
    assert!(drafts.pull(all.checkpoint.rev).documents.is_empty());
    assert_eq!(
        drafts.pull(all.checkpoint.rev).checkpoint.rev,
        2,
        "an empty pull holds the checkpoint where it was",
    );

    drafts.apply("s1", "one, more", Some("one"), 2000);
    let next = drafts.pull(2);
    assert_eq!(next.documents.len(), 1);
    assert_eq!(next.documents[0].ulid, "s1");
    assert_eq!(next.checkpoint.rev, 3);
}

/// ⚠ **THE DEFECT THIS PINS IS TOTAL, NOT LATE: the stranded conversation never
/// syncs at all.**
///
/// The cursor is ONE number for the collection. While `rev` was minted per
/// document, a draft written in a conversation nobody had typed in yet took
/// rev 1 — and a client that had already pulled a different conversation was
/// checkpointed past it, so `rev > since` never matched and the words sat on the
/// device that wrote them for ever.
///
/// It looks random from outside, because whether it bites depends on what OTHER
/// conversations have been written in — and a stranded draft never arrives at
/// all, so there is nothing late to notice.
#[test]
fn a_draft_in_a_new_conversation_is_delivered_to_a_client_that_is_already_ahead() {
    let dir = scratch("cursor-strands");
    let drafts = store(&dir);

    // One conversation gets some use.
    drafts.apply("busy", "a", None, 1000);
    drafts.apply("busy", "ab", Some("a"), 2000);
    drafts.apply("busy", "abc", Some("ab"), 3000);
    let caught_up = drafts.pull(0).checkpoint.rev;
    assert_eq!(caught_up, 3);

    // Now somebody types in a conversation for the first time.
    drafts.apply("fresh", "the first words here", None, 4000);

    let next = drafts.pull(caught_up);
    assert_eq!(
        next.documents.len(),
        1,
        "a draft written after another conversation got ahead was never delivered",
    );
    assert_eq!(next.documents[0].ulid, "fresh");
    assert!(
        next.documents[0].rev > caught_up,
        "a new draft must sort AFTER what the client has already seen",
    );
}

/// ⚠ **An empty answer means every entry landed.** That is RxDB's contract and
/// the opposite of a status code: a push that conflicts is a SUCCESSFUL request
/// carrying the current master.
#[test]
fn a_push_answers_with_the_entries_that_lost_and_nothing_else() {
    let dir = scratch("push");
    let drafts = store(&dir);
    drafts.apply("s1", "mine", None, 1000);

    let clean = drafts.push(vec![PushEntry {
        new_document_state: doc("s2", "fresh", 0),
        assumed_master_state: None,
    }]);
    assert!(clean.is_empty(), "a fresh insert cannot conflict");

    let lost = drafts.push(vec![PushEntry {
        new_document_state: doc("s1", "theirs", 0),
        assumed_master_state: None,
    }]);
    assert_eq!(lost.len(), 1, "a first write over an existing draft loses");
    assert_eq!(lost[0].text, "mine", "the loser is handed the master");
    assert_eq!(lost[0].rev, 1);
}

/// One batch, both outcomes — which is why the status cannot carry the answer.
#[test]
fn a_batch_can_both_land_and_lose() {
    let dir = scratch("batch");
    let drafts = store(&dir);
    drafts.apply("taken", "already here", None, 1000);

    let lost = drafts.push(vec![
        PushEntry {
            new_document_state: doc("fresh", "lands", 0),
            assumed_master_state: None,
        },
        PushEntry {
            new_document_state: doc("taken", "loses", 0),
            assumed_master_state: None,
        },
    ]);
    assert_eq!(lost.len(), 1);
    assert_eq!(lost[0].ulid, "taken");
    assert_eq!(
        drafts.get("fresh").expect("the other one landed").text,
        "lands",
    );
}

/// ⚠ **One device typing must never collide with itself.**
///
/// Driven through `push`, with the entries shaped the way RxDB sends them,
/// because the defect lives in what the wire carries rather than in the rule.
/// A revision is minted here, so a client learns its own new one only on the
/// next PULL; RxDB meanwhile sets its assumed master to the document it SENT,
/// which carries the revision it was editing FROM. So `assumedMasterState.rev`
/// is one behind for the whole pull interval, and judging a push on it refuses
/// the second keystroke and calls it a conflict. Continuous typing is the
/// ordinary case, not an edge.
///
/// ⚠ Ablated to prove it can fail: with the rule put back to
/// comparing `assumed.rev` against the current revision, this test fails on the
/// second push, which is returned as a conflict against this device's own
/// previous keystroke. It is the bug the hand-rolled client hit as "a poll
/// reply older than local state read as another device writing".
#[test]
fn one_device_pushing_twice_before_it_pulls_does_not_clash_with_itself() {
    let dir = scratch("self-clash");
    let drafts = store(&dir);

    // Nothing here yet, so RxDB assumes nothing.
    assert!(
        drafts
            .push(vec![PushEntry {
                new_document_state: doc("s1", "typing", 0),
                assumed_master_state: None,
            }])
            .is_empty(),
        "a first write cannot conflict with nothing",
    );
    assert_eq!(drafts.get("s1").expect("stored").rev, 1);

    // The second keystroke, before any pull. What the client assumes is the
    // document it SENT — text "typing", and the revision it edited from, which
    // is 0 and NOT the 1 the runner minted.
    let lost = drafts.push(vec![PushEntry {
        new_document_state: doc("s1", "typing more", 0),
        assumed_master_state: Some(doc("s1", "typing", 0)),
    }]);
    assert!(
        lost.is_empty(),
        "a device editing on from its own last push is not a second writer; got {lost:?}",
    );
    assert_eq!(drafts.get("s1").expect("stored").text, "typing more");
    assert_eq!(
        drafts.get("s1").expect("stored").rev,
        2,
        "the revision still counts, for the pull cursor",
    );

    // And a third, still without pulling.
    assert!(
        drafts
            .push(vec![PushEntry {
                new_document_state: doc("s1", "typing more still", 0),
                assumed_master_state: Some(doc("s1", "typing more", 0)),
            }])
            .is_empty(),
        "three keystrokes inside one pull interval is ordinary typing",
    );
    assert_eq!(
        drafts.get("s1").expect("still there").text,
        "typing more still",
    );
}

/// The other half of the same rule: a device that HAS diverged still loses.
/// Without this, the test above passes for a rule that accepts everything.
#[test]
fn a_device_that_assumed_different_words_still_loses() {
    let dir = scratch("still-conflicts");
    let drafts = store(&dir);
    drafts.apply("s1", "what the mac wrote", None, 1000);

    // The phone assumes text that was never here — it has been away.
    let lost = drafts.push(vec![PushEntry {
        new_document_state: doc("s1", "what the phone wrote", 0),
        assumed_master_state: Some(doc("s1", "an older shared draft", 0)),
    }]);
    assert_eq!(lost.len(), 1, "a genuine divergence must still conflict");
    assert_eq!(
        lost[0].text, "what the mac wrote",
        "the loser is handed THEIRS"
    );
    assert_eq!(
        drafts.get("s1").expect("still there").text,
        "what the mac wrote",
        "the refused text must not have landed",
    );
}

/// Two devices that happened to type the same words have nothing to choose
/// between, so this is agreement and not a conflict.
#[test]
fn two_devices_that_wrote_the_same_words_agree() {
    let dir = scratch("same-words");
    let drafts = store(&dir);
    drafts.apply("s1", "on my way", None, 1000);

    let Wrote::Stored(_) = drafts.apply("s1", "on my way!", Some("on my way"), 2000) else {
        panic!("assuming exactly what is there is the landing case");
    };
}
