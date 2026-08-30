//! What the effects artefact must say the same way twice.
//!
//! The mine is resumable, so the same work can be accumulated in two different
//! orders — everything read in one pass, or a carried artefact plus the tails
//! that grew. The two must finish identical, and the ONLY thing standing between
//! them is how `finish` orders what it was given.

use reader::effects::{Did, Effect, Log};
use reader::shell::Reached;

fn effect<'a>(call: &'a str, did: Did, minute: i64) -> Effect<'a> {
    Effect {
        call,
        agent: "alpha",
        minute,
        did,
        // ⚠ One path and one command deliberately: two of either would also vary
        // the DICTIONARIES, which intern in arrival order and are a separate
        // question. What is under test is row order alone.
        path: Some("memview/src/lib.rs"),
        pattern: None,
        host: None,
        command: "cat memview/src/lib.rs",
        reached: Reached::Always,
    }
}

/// ⚠ **`sort_by_key(|row| row.t)` is STABLE, so rows sharing a minute kept the
/// order they were pushed in** — which is the order transcripts happened to be
/// read, and differs between a whole scan and a resumed one.
///
/// Measured on the real corpus 2026-08-30: this artefact was the last of the
/// four still failing the parity check, at identical byte length and a different
/// hash (memview#1240).
#[test]
fn row_order_does_not_depend_on_the_order_the_effects_arrived() {
    let build = |reversed: bool| {
        let mut log = Log::default();
        let mut both = vec![
            effect("call-a", Did::Read, 500),
            effect("call-b", Did::Wrote, 500),
        ];
        if reversed {
            both.reverse();
        }
        for one in both {
            log.push(one);
        }
        log.finish("2026-08-30T00:00:00Z")
    };

    let forward = build(false);
    let backward = build(true);

    assert_eq!(forward.rows.len(), 2, "both effects should be recorded");
    assert_eq!(
        serde_json::to_string(&forward.rows).expect("rows"),
        serde_json::to_string(&backward.rows).expect("rows"),
        "row order depended on arrival order"
    );
    // The dictionaries are held constant by the fixture, so if they ever differ
    // the test is measuring the wrong thing rather than finding a bug.
    assert_eq!(
        forward.paths, backward.paths,
        "fixture varied the dictionary"
    );
    assert_eq!(
        forward.agents, backward.agents,
        "fixture varied the dictionary"
    );
}
