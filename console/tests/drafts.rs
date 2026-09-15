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

    let Wrote::Stored(mine) = drafts.put("s1", "half a thought", None, 1000) else {
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

/// ⚠ **The write that must be refused.** Two devices editing from the same
/// revision is the whole reason a revision is carried.
#[test]
fn an_edit_from_a_stale_revision_is_refused_and_hands_back_theirs() {
    let dir = scratch("stale");
    let drafts = store(&dir);
    drafts.put("s1", "mac words", None, 1000);

    // The phone read rev 1, then the Mac wrote again.
    let Wrote::Stored(second) = drafts.put("s1", "mac words, more", Some(1), 2000) else {
        panic!("the Mac is editing from what it just wrote");
    };
    assert_eq!(second.rev, 2);

    match drafts.put("s1", "phone words", Some(1), 3000) {
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
    drafts.put("s1", "the message", None, 1000);
    // The Mac sends it: composer empties, and that is a write like any other.
    let Wrote::Stored(cleared) = drafts.put("s1", "", Some(1), 2000) else {
        panic!("clearing is an ordinary edit from the current revision");
    };
    assert_eq!(cleared.rev, 2);
    assert_eq!(cleared.text, "");

    // The phone, still at rev 1, pushes the words it is holding.
    match drafts.put("s1", "the message", Some(1), 3000) {
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
    drafts.put("s1", "mac words", None, 1000);
    match drafts.put("s1", "phone words", None, 2000) {
        Wrote::Conflict(theirs) => assert_eq!(theirs.text, "mac words"),
        Wrote::Stored(_) => panic!("a phone that had never looked erased the Mac's draft"),
    }
}

/// Across a restart, which is what a file is for — the console is upgraded
/// several times an evening.
#[test]
fn a_draft_survives_the_console_restarting() {
    let dir = scratch("restart");
    store(&dir).put("s1", "written before the upgrade", None, 1000);
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
    drafts.put("gone", "words", None, 1000);
    drafts.put("alive", "words", None, 1000);

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
    drafts.put("s1", "one", None, 1000);
    drafts.put("s2", "two", None, 1000);

    let all = drafts.pull(0);
    assert_eq!(all.documents.len(), 2, "a first pull takes everything");
    assert_eq!(all.checkpoint.rev, 1);

    // Nothing has moved since.
    assert!(drafts.pull(all.checkpoint.rev).documents.is_empty());
    assert_eq!(
        drafts.pull(all.checkpoint.rev).checkpoint.rev,
        1,
        "an empty pull holds the checkpoint where it was",
    );

    drafts.put("s1", "one, more", Some(1), 2000);
    let next = drafts.pull(1);
    assert_eq!(next.documents.len(), 1);
    assert_eq!(next.documents[0].ulid, "s1");
    assert_eq!(next.checkpoint.rev, 2);
}

/// ⚠ **An empty answer means every entry landed.** That is RxDB's contract and
/// the opposite of a status code: a push that conflicts is a SUCCESSFUL request
/// carrying the current master.
#[test]
fn a_push_answers_with_the_entries_that_lost_and_nothing_else() {
    let dir = scratch("push");
    let drafts = store(&dir);
    drafts.put("s1", "mine", None, 1000);

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
    drafts.put("taken", "already here", None, 1000);

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
