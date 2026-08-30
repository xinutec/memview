//! The resume state a mine starts from, exercised through the public API.
//!
//! ⚠ **The two `load` cases are the point.** An absent file and a damaged one
//! must not give the same answer: a damaged one read as "nothing to resume" would
//! resume from empty folds while the watermarks said the corpus had been read,
//! and the artefact would lose everything the carried state held — silently.

use memview::agents::FirstSeen;
use memview::mine::Carried;

fn stamped(sha: &str, when: &str) -> FirstSeen {
    let mut m = FirstSeen::new();
    m.insert(sha.to_string(), (when.to_string(), "builder".to_string()));
    m
}

#[test]
fn a_missing_file_is_a_first_run() {
    let dir = tempfile::tempdir().expect("tempdir");
    let got = Carried::load(&dir.path().join("absent.json")).expect("not an error");
    assert_eq!(got, None);
}

/// ⚠ Separated from the case above deliberately — see the module note.
#[test]
fn a_file_that_will_not_parse_is_fatal_rather_than_empty() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("mine-resume.json");
    std::fs::write(&path, "{ this is not json").expect("write");
    assert!(Carried::load(&path).is_err());
}

#[test]
fn what_is_saved_is_what_is_loaded() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("mine-resume.json");

    let mut carried = Carried {
        generated: "2026-08-30T09:00:00Z".to_string(),
        first_seen: stamped("abc1234", "2026-08-01T00:00:00Z"),
        ..Carried::default()
    };
    carried
        .resolved
        .insert("s-1".to_string(), "builder".to_string());
    carried.days.entry("builder".to_string()).or_default();

    carried.save(&path).expect("saves");
    let back = Carried::load(&path).expect("loads").expect("present");
    assert_eq!(back, carried);
}

/// ⚠ The reason `resolved` is carried at all. A session is named in the HEAD of
/// its transcript; a resumed run reads only the tail, so without this surviving
/// the round trip every long-lived agent comes back as a bare uuid.
#[test]
fn a_session_name_survives_the_round_trip() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("mine-resume.json");

    let mut carried = Carried::default();
    carried
        .resolved
        .insert("9f2c-uuid".to_string(), "recall".to_string());
    carried.save(&path).expect("saves");

    let back = Carried::load(&path).expect("loads").expect("present");
    assert_eq!(
        back.resolved.get("9f2c-uuid").map(String::as_str),
        Some("recall")
    );
}

// --- the property that makes a resumed mine usable at all --------------------

use memview::agents::{Resumed, scan, scan_resumed};

/// A corpus with one session, written in two stretches.
fn corpus_with(
    dir: &std::path::Path,
    project: &str,
    session: &str,
    lines: &[&str],
) -> std::path::PathBuf {
    let proj = dir.join(project);
    std::fs::create_dir_all(&proj).expect("mkdir");
    let path = proj.join(format!("{session}.jsonl"));
    std::fs::write(&path, lines.join("\n") + "\n").expect("write");
    path
}

fn append(path: &std::path::Path, lines: &[&str]) {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .expect("open");
    f.write_all((lines.join("\n") + "\n").as_bytes())
        .expect("append");
}

/// A `Read` of `path` at `stamp`, in the shape the miner parses.
///
/// ⚠ **The `id` is required and its absence is silent.** A timeline row is only
/// pushed when `call_id` finds one BEFORE the tool's name on the line; without
/// it the call still counts towards the roster's reads, so a fixture missing it
/// looks like it works and produces an empty timeline. That cost one wrong test
/// before it was noticed.
fn read_call(path: &str, stamp: &str) -> String {
    let id = stamp.replace([':', '-'], "");
    format!(
        "{{\"type\":\"assistant\",\"timestamp\":\"{stamp}\",\"message\":{{\"content\":[{{\"type\":\"tool_use\",\"id\":\"tu_{id}\",\"name\":\"Read\",\"input\":{{\"file_path\":\"{path}\"}}}}]}}}}"
    )
}

/// ⚠ **The whole point of the exercise, and the only thing that makes a resumed
/// mine safe to run nightly: reading only the tail must give the answer reading
/// everything gives.** A resume that is wrong reads no error — it mines from an
/// offset that means something else and the artefact simply becomes untrue.
///
/// ⚠ **The head and the tail each carry a REAL tool call, and that matters.** An
/// earlier version of this test used transcripts with no calls in them, so
/// `reads` was an empty map on both sides and the comparison passed by being
/// vacuous — `feedback_a_degenerate_example_cannot_show_a_convention`. With a
/// call on each side, a resume that failed to carry the roster would report one
/// read where the whole scan reports two.
#[test]
fn resuming_over_an_appended_corpus_equals_reading_it_whole() {
    let root = tempfile::tempdir().expect("tempdir");
    let sessions = tempfile::tempdir().expect("tempdir");
    let memory = tempfile::tempdir().expect("tempdir");
    let stamp = "2026-08-30T00:00:00Z";

    let path = corpus_with(
        root.path(),
        "-code",
        "s1",
        &[&read_call("/code/alpha/a.rs", "2026-08-01T10:00:00Z")],
    );

    let run = |from: Option<Resumed>| {
        scan_resumed(
            root.path(),
            sessions.path(),
            "/code",
            &memory.path().to_string_lossy(),
            "/home/example",
            stamp,
            from,
        )
        .expect("scan")
    };

    // The roster now rides inside `carried`, so the artefact itself is unused here.
    let (_first, carried) = run(None);
    assert_eq!(carried.marks.len(), 1, "the transcript should be marked");

    append(
        &path,
        &[&read_call("/code/alpha/b.rs", "2026-08-02T10:00:00Z")],
    );

    let (resumed, _) = run(Some(Resumed {
        carried,
        doing: reader::doing::Doing::default(),
        effects: reader::effects::Effects::default(),
    }));

    let whole = scan(
        root.path(),
        sessions.path(),
        "/code",
        &memory.path().to_string_lossy(),
        "/home/example",
        stamp,
    )
    .expect("whole");

    // ⚠ Guard against a VACUOUS comparison: two empty rosters are equal, and a
    // parity test that passes because neither pass saw anything proves nothing.
    assert!(!whole.agents.is_empty(), "fixture produced no agents");
    assert_eq!(
        whole.agents[0].reads.get("alpha"),
        Some(&2),
        "the whole scan should see both reads"
    );
    assert_eq!(
        resumed.agents.len(),
        whole.agents.len(),
        "resumed saw {} agents, whole saw {}",
        resumed.agents.len(),
        whole.agents.len()
    );
    for (a, b) in resumed.agents.iter().zip(whole.agents.iter()) {
        assert_eq!(a.name, b.name, "agent names diverged");
        assert_eq!(a.reads, b.reads, "{}: reads diverged", a.name);
        assert_eq!(a.writes, b.writes, "{}: writes diverged", a.name);
        // ⚠ Counted per TRANSCRIPT, not per read. Without the guard in
        // `scan_resumed`, resuming into a grown file reports one session as two
        // — the defect the full-corpus parity run found on 2026-08-30.
        assert_eq!(
            a.transcripts, b.transcripts,
            "{}: transcript count diverged",
            a.name
        );
        assert_eq!(
            a.delegated, b.delegated,
            "{}: delegated count diverged",
            a.name
        );
        assert_eq!(a.sessions, b.sessions, "{}: sessions diverged", a.name);
    }
}

/// ⚠ **A transcript nothing touched must be carried, not forgotten.** Dropping
/// its watermark would make the NEXT run believe the file was new and read it
/// whole — the saving quietly undoing itself, one file at a time, with no
/// symptom but a slow mine.
#[test]
fn an_untouched_transcript_keeps_its_watermark() {
    let root = tempfile::tempdir().expect("tempdir");
    let sessions = tempfile::tempdir().expect("tempdir");
    let code = tempfile::tempdir().expect("tempdir");
    let memory = tempfile::tempdir().expect("tempdir");

    corpus_with(
        root.path(),
        "-tmp-proj",
        "s1",
        &[
            r#"{"type":"user","sessionId":"s1","timestamp":"2026-08-01T10:00:00Z","message":{"role":"user","content":"only"}}"#,
        ],
    );
    let stamp = "2026-08-30T00:00:00Z";

    let (_, first) = scan_resumed(
        root.path(),
        sessions.path(),
        &code.path().to_string_lossy(),
        &memory.path().to_string_lossy(),
        "/home/example",
        stamp,
        None,
    )
    .expect("first");
    assert_eq!(first.marks.len(), 1);

    // Nothing changed on disk between the two runs.
    let (_, second) = scan_resumed(
        root.path(),
        sessions.path(),
        &code.path().to_string_lossy(),
        &memory.path().to_string_lossy(),
        "/home/example",
        stamp,
        Some(Resumed {
            carried: first.clone(),
            doing: reader::doing::Doing::default(),
            effects: reader::effects::Effects::default(),
        }),
    )
    .expect("second");

    assert_eq!(
        second.marks, first.marks,
        "an untouched transcript lost its watermark"
    );
}

/// A git repo under `root` with one commit, returning its short sha.
///
/// ⚠ Every inherited git variable is removed: these tests run under `cargo
/// test`, `cargo test` runs under the gate, and the gate runs from a pre-commit
/// hook that exports `GIT_DIR` and `GIT_INDEX_FILE` to everything it spawns —
/// so `git -C <tempdir> add` would write into MEMVIEW'S index.
fn repo_with_a_commit(root: &std::path::Path, name: &str) -> String {
    let repo = root.join(name);
    std::fs::create_dir_all(&repo).expect("mkdir");
    let git = |args: &[&str]| {
        let mut c = std::process::Command::new("git");
        c.arg("-C").arg(&repo).args(args);
        for var in [
            "GIT_DIR",
            "GIT_INDEX_FILE",
            "GIT_WORK_TREE",
            "GIT_OBJECT_DIRECTORY",
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "GIT_COMMON_DIR",
            "GIT_PREFIX",
            "GIT_CONFIG_PARAMETERS",
        ] {
            c.env_remove(var);
        }
        c.output().expect("git");
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "t@example.com"]);
    git(&["config", "user.name", "t"]);
    std::fs::write(repo.join("f.rs"), "x").expect("write");
    git(&["add", "f.rs"]);
    git(&["commit", "-qm", "one"]);
    let mut c = std::process::Command::new("git");
    c.arg("-C")
        .arg(&repo)
        .args(["rev-parse", "--short=8", "HEAD"]);
    for var in ["GIT_DIR", "GIT_INDEX_FILE", "GIT_WORK_TREE"] {
        c.env_remove(var);
    }
    let out = c.output().expect("git");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// ⚠ **Commit attribution is RECOMPUTED from the whole git history each run, not
/// accumulated** — so a carried roster must have its counts cleared first.
///
/// Without the reset a resumed mine reports exactly DOUBLE. Found by the first
/// full-corpus parity run, 2026-08-30, not by any fixture: the other fixtures
/// carry no git history, so there was nothing to double.
#[test]
fn a_resumed_mine_does_not_double_the_commit_counts() {
    let root = tempfile::tempdir().expect("tempdir");
    let sessions = tempfile::tempdir().expect("tempdir");
    let code = tempfile::tempdir().expect("tempdir");
    let memory = tempfile::tempdir().expect("tempdir");
    let stamp = "2026-08-30T00:00:00Z";

    let sha = repo_with_a_commit(code.path(), "alpha");
    // A turn that mentions the hash is what attributes it to this agent.
    corpus_with(
        root.path(),
        "-code",
        "s1",
        &[&format!(
            "{{\"type\":\"assistant\",\"timestamp\":\"2026-08-01T10:00:00Z\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"landed in {sha}\"}}]}}}}"
        )],
    );

    let run = |from: Option<Resumed>| {
        scan_resumed(
            root.path(),
            sessions.path(),
            &code.path().to_string_lossy(),
            &memory.path().to_string_lossy(),
            "/home/example",
            stamp,
            from,
        )
        .expect("scan")
    };

    let (first, carried) = run(None);
    let counted: usize = first.agents.iter().map(|a| a.commits).sum();
    assert_eq!(counted, 1, "the fixture must attribute exactly one commit");

    // Nothing changed on disk, so the resumed run reads no transcript at all —
    // and must still report the same attribution, not twice it.
    let (again, _) = run(Some(Resumed {
        carried,
        doing: reader::doing::Doing::default(),
        effects: reader::effects::Effects::default(),
    }));
    let recounted: usize = again.agents.iter().map(|a| a.commits).sum();
    assert_eq!(
        recounted, counted,
        "a resumed mine doubled the commit count"
    );
}

/// ⚠ **The carried episode has to survive into the log, and for a long time it
/// did not.** The driver called `Log::reopen` just before `scan_transcript`,
/// which opens the log itself on its first statement and cleared it — so the
/// carry was applied and thrown away one line later, silently.
///
/// Measured on the real corpus 2026-08-30: 78 tail rows landed in no episode
/// where a whole scan put them in the episode open at the cut, and that single
/// episode was the ONLY remaining difference between a resumed artefact and a
/// full one.
#[test]
fn a_tail_continues_the_episode_that_was_open_at_the_cut() {
    let root = tempfile::tempdir().expect("tempdir");
    let sessions = tempfile::tempdir().expect("tempdir");
    // No repositories needed: this asks about episodes, not commits.
    let _code = tempfile::tempdir().expect("tempdir");
    let memory = tempfile::tempdir().expect("tempdir");
    let stamp = "2026-08-30T00:00:00Z";

    // A user turn opens the episode; the call after it materialises the episode.
    let prompt = r#"{"type":"user","timestamp":"2026-08-01T10:00:00Z","message":{"role":"user","content":"do the thing"}}"#;
    let path = corpus_with(
        root.path(),
        "-code",
        "s1",
        &[
            prompt,
            &read_call("/code/alpha/a.rs", "2026-08-01T10:01:00Z"),
        ],
    );

    let run = |from: Option<Resumed>| {
        scan_resumed(
            root.path(),
            sessions.path(),
            "/code",
            &memory.path().to_string_lossy(),
            "/home/example",
            stamp,
            from,
        )
        .expect("scan")
    };

    let (first, carried) = run(None);
    assert_eq!(
        first.doing.episodes.len(),
        1,
        "the prompt should open one episode"
    );

    // More work under the SAME instruction, after the watermark.
    append(
        &path,
        &[&read_call("/code/alpha/b.rs", "2026-08-01T10:02:00Z")],
    );

    let (resumed, _) = run(Some(Resumed {
        carried,
        doing: first.doing,
        effects: reader::effects::Effects::default(),
    }));
    let whole = scan(
        root.path(),
        sessions.path(),
        "/code",
        &memory.path().to_string_lossy(),
        "/home/example",
        stamp,
    )
    .expect("whole");

    assert!(
        !whole.doing.rows.is_empty(),
        "fixture produced no timeline rows"
    );
    let orphaned = resumed.doing.rows.iter().filter(|r| r.e.is_none()).count();
    assert_eq!(
        orphaned,
        whole.doing.rows.iter().filter(|r| r.e.is_none()).count(),
        "a resumed tail orphaned {orphaned} row(s) the whole scan placed in an episode"
    );
    assert_eq!(
        resumed.doing.episodes.len(),
        whole.doing.episodes.len(),
        "episode count diverged"
    );
    for (a, b) in resumed
        .doing
        .episodes
        .iter()
        .zip(whole.doing.episodes.iter())
    {
        assert_eq!(a.n, b.n, "episode row count diverged: {a:?} vs {b:?}");
        assert_eq!(a.until, b.until, "episode end diverged: {a:?} vs {b:?}");
    }
}
