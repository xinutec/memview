//! **A resumed mine must give the answer a whole mine gives.** For any corpus,
//! cut at any point.
//!
//! ⚠ **This exists because six bugs of one family were found BY HAND on
//! 2026-08-30, and each needed a stronger fixture than the one before it**
//! (memview#1240). All six were the same shape — a fold written to run over
//! everything, made to run over what changed — and none was visible to the test
//! that preceded it:
//!
//!     commits doubled                     zero-change parity
//!     renames re-applied through a cycle  zero-change parity
//!     transcripts counted per READ        growth parity
//!     episode identity was a position     truncate-and-restore
//!     the carry was cleared one line on   truncate-and-restore
//!     effects sorted on a partial key     truncate-and-restore
//!
//! Doing that comparison by hand once per bug is how five of them survived to
//! be found late. Here it is the invariant, checked at every cut.
//!
//! ⚠ **The corpus is TRUNCATED and RESTORED, never synthesised in two halves.**
//! Hand-written appended lines do not exercise the pipeline: two rounds of them
//! moved `agents.json` and left `doing.json` and `effects.json` byte-identical,
//! which reads as a pass and proves nothing. Cutting real generated content on a
//! line boundary is the only form of this that works.
//!
//! ## What this actually catches, measured by ablation 2026-08-30
//!
//!     commits reset removed                 CAUGHT
//!     per-read transcript counter           CAUGHT
//!     episode renumbering removed           CAUGHT
//!     watermark episode remap removed       CAUGHT
//!     carried episode cleared               CAUGHT
//!     doing rows sorted on a partial key    NOT CAUGHT
//!     effects rows sorted on a partial key  NOT CAUGHT
//!
//! ⚠ **The two total-order sorts are NOT proven by this fixture and that is
//! stated rather than assumed.** A stable sort only differs when a tie is
//! inserted in a different order, and here the carried rows and the tail happen
//! to arrive in the same relative order the whole scan visits them in. They are
//! defensive; the renumbering is what made the real corpus agree. **Do not read
//! a green run as covering them** — a fixture that does would need a tie whose
//! two sides swap across the cut.
//!
//! ⚠ **Every assertion is guarded against being VACUOUS.** Two empty artefacts
//! compare equal. A fixture that silently produces no rows — the one that cost a
//! wrong test on 2026-08-30, because `log.push` needs a tool-use `id` nobody had
//! noticed — would otherwise pass this file completely.

use memview::agents::{Agents, Needs, Resumed, Roots, scan_resumed};
use memview::mine::Carried;

/// One transcript's worth of plausible history.
///
/// Deliberately varied across agents, projects, minutes and tool kinds: a
/// corpus where every line is alike cannot show an ordering bug, because there
/// are no ties to order wrongly.
fn transcript(session: &str, agent: &str, turns: usize, day: i64) -> String {
    let mut out = String::new();
    let mut uuid = 0u32;
    let mut next = |out: &mut String, line: String| {
        uuid += 1;
        out.push_str(&line);
        out.push('\n');
    };
    for t in 0..turns {
        let minute = day * 1440 + (t as i64 * 7 % 600);
        let stamp = format!(
            "2026-08-{:02}T{:02}:{:02}:00Z",
            1 + (day % 27),
            (minute / 60) % 24,
            minute % 60
        );
        // A user turn opens an episode; the calls under it materialise one.
        next(
            &mut out,
            format!(
                r#"{{"type":"user","sessionId":"{session}","uuid":"u{t}-{session}","timestamp":"{stamp}","cwd":"/code/{agent}","message":{{"role":"user","content":"turn {t}"}}}}"#
            ),
        );
        // ⚠ **A Bash call, because EFFECTS come only from parsed shell steps.**
        // The first version of this fixture had none and produced an empty
        // `effects.json` — which the vacuity guard caught, and which would
        // otherwise have compared empty to empty at every cut.
        next(
            &mut out,
            format!(
                r#"{{"type":"assistant","sessionId":"{session}","uuid":"sh{t}-{session}","timestamp":"{stamp}","cwd":"/code/{agent}","message":{{"content":[{{"type":"tool_use","id":"tush-{t}-{session}","name":"Bash","input":{{"command":"cat /code/{agent}/src/a{}.rs && grep -n needle /code/{agent}/src/b{}.rs"}}}}]}}}}"#,
                t % 3,
                t % 2
            ),
        );
        // ⚠ The `id` is what makes this produce a timeline row at all.
        for (k, tool, path) in [
            (0, "Read", format!("/code/{agent}/src/a{}.rs", t % 3)),
            (1, "Edit", format!("/code/{agent}/src/b{}.rs", t % 2)),
            (2, "Read", format!("/mem/reference_thing_{}.md", t % 4)),
        ] {
            next(
                &mut out,
                format!(
                    r#"{{"type":"assistant","sessionId":"{session}","uuid":"a{t}-{k}-{session}","timestamp":"{stamp}","cwd":"/code/{agent}","message":{{"content":[{{"type":"tool_use","id":"tu-{t}-{k}-{session}","name":"{tool}","input":{{"file_path":"{path}"}}}}]}}}}"#
                ),
            );
        }
    }
    out
}

/// Serialise and read back, which is what a resumed run actually receives.
fn through_wire<T: serde::Serialize + serde::de::DeserializeOwned>(v: &T) -> T {
    serde_json::from_str(&serde_json::to_string(v).expect("write")).expect("read")
}

struct Corpus {
    root: tempfile::TempDir,
    sessions: tempfile::TempDir,
    memory: tempfile::TempDir,
    code: tempfile::TempDir,
    /// The transcript the cut is taken in, and its full contents.
    cut_in: std::path::PathBuf,
    whole: String,
}

/// A repository with one commit, so commit ATTRIBUTION is exercised.
///
/// ⚠ Without this the fixture had no git history at all, `commits` was 0 on both
/// sides, and removing the reset that stops a resumed run DOUBLING them still
/// passed.
fn repo_with_a_commit(root: &std::path::Path) -> String {
    let repo = root.join("alpha");
    std::fs::create_dir_all(&repo).expect("mkdir");
    // ⚠ **Strip EVERY GIT_* variable, not a list.** Inside the gate this
    // fixture runs under memview's own pre-commit hook, which exports GIT_DIR,
    // GIT_COMMON_DIR, GIT_OBJECT_DIRECTORY and more to every child. An
    // enumerated subset missed GIT_COMMON_DIR, so `git init` here bound the new
    // repo to memview's common dir instead of making a standalone one — then
    // the miner's own `git log` (which does env-clean) found no repository and
    // returned exit 128, read as "the resumed mine lost the commits". In-gate
    // only, 2026-09-02, and only under the full parallel suite. Removing the
    // whole GIT_* set by prefix cannot drift out of date the way a list does.
    let git = |args: &[&str]| -> std::process::Output {
        let mut c = std::process::Command::new("git");
        c.arg("-C").arg(&repo).args(args);
        for (key, _) in std::env::vars() {
            if key.starts_with("GIT_") {
                c.env_remove(key);
            }
        }
        // ⚠ **Fixed dates, so the hash is DETERMINISTIC.** A commit made from
        // the wall clock gets a different sha every run, and
        // `commits::hash_candidates` deliberately refuses an all-digit short
        // hash (3.4% of the fleet's real commits) — so roughly one run in forty
        // produced a fixture that attributed nothing, tripped `refuse_vacuous`,
        // and failed CI while passing locally. A fixture whose validity is
        // decided by chance is not a fixture.
        c.env("GIT_AUTHOR_DATE", "2026-08-11T00:00:00Z");
        c.env("GIT_COMMITTER_DATE", "2026-08-11T00:00:00Z");
        let out = c.output().expect("git");
        assert!(
            out.status.success(),
            "fixture git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        out
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "t@example.com"]);
    git(&["config", "user.name", "t"]);
    std::fs::write(repo.join("src.rs"), "x").expect("write");
    git(&["add", "src.rs"]);
    git(&["commit", "-qm", "one"]);
    let sha = String::from_utf8_lossy(&git(&["rev-parse", "--short=8", "HEAD"]).stdout)
        .trim()
        .to_string();
    // ⚠ **The one property the whole fixture rests on.** An all-digit hash is
    // never attributed, on purpose, so it would make every comparison below
    // vacuous — and the failure reads as "the resumed mine lost the commits"
    // rather than "the fixture cannot express the question". With the dates
    // pinned above this is decided once, here, instead of per run.
    assert!(
        sha.chars().any(|c| c.is_ascii_alphabetic()),
        "fixture hash {sha:?} is all digits, which is never attributed — \
         change the commit content or date until it is not",
    );
    sha
}

fn corpus() -> Corpus {
    let root = tempfile::tempdir().expect("tempdir");
    let proj = root.path().join("-code");
    std::fs::create_dir_all(&proj).expect("mkdir");
    let mut cut_in = std::path::PathBuf::new();
    let mut whole = String::new();
    // ⚠ **The same day for all three, deliberately.** Rows only tie on a minute
    // when sessions OVERLAP in time, and a tie is the only thing a stable sort
    // can order by traversal. With a day each, the ablation that removes the
    // total order still passed — the fixture could not express the bug.
    for (i, (session, agent, turns, day)) in [
        ("s1", "alpha", 9, 20690),
        ("s2", "beta", 7, 20690),
        ("s3", "alpha", 11, 20690),
    ]
    .into_iter()
    .enumerate()
    {
        let text = transcript(session, agent, turns, day);
        let path = proj.join(format!("{session}.jsonl"));
        std::fs::write(&path, &text).expect("write");
        if i == 2 {
            cut_in = path;
            whole = text;
        }
    }
    let code = tempfile::tempdir().expect("tempdir");
    let sha = repo_with_a_commit(code.path());
    // A turn that mentions the hash is what attributes the commit to an agent.
    let mentions = proj.join("s4.jsonl");
    std::fs::write(
        &mentions,
        format!(
            "{{\"type\":\"assistant\",\"sessionId\":\"s4\",\"uuid\":\"m1\",\"timestamp\":\"2026-08-11T00:05:00Z\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"landed in {sha}\"}}]}}}}\n"
        ),
    )
    .expect("write");

    Corpus {
        root,
        sessions: tempfile::tempdir().expect("tempdir"),
        memory: tempfile::tempdir().expect("tempdir"),
        code,
        cut_in,
        whole,
    }
}

impl Corpus {
    fn mine(&self, from: Option<Resumed>) -> (Agents, Carried) {
        scan_resumed(
            Roots {
                projects: self.root.path(),
                sessions: self.sessions.path(),
                code_root: &self.code.path().to_string_lossy(),
                memory_root: &self.memory.path().to_string_lossy(),
                home: "/home/example",
            },
            "2026-08-30T00:00:00Z",
            from,
            Needs::EVERYTHING,
        )
        .expect("scan")
    }
}

/// A short, comparable digest of one dimension, so a failure names the dimension
/// instead of printing two 40 KB artefacts at each other.
fn digest(v: &impl serde::Serialize) -> String {
    let s = serde_json::to_string(v).expect("serialise");
    format!("{} bytes, {:016x}", s.len(), {
        // FNV-1a: a stable fingerprint, no dependency, and enough to separate
        // two artefacts that differ anywhere.
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in s.as_bytes() {
            h ^= *b as u64;
            h = h.wrapping_mul(0x100_0000_01b3);
        }
        h
    })
}

/// Every dimension, named, so the first disagreement is the report.
fn dimensions(a: &Agents) -> Vec<(&'static str, String)> {
    vec![
        ("roster", digest(&a.agents)),
        ("doing.rows", digest(&a.doing.rows)),
        ("doing.episodes", digest(&a.doing.episodes)),
        ("doing.agents", digest(&a.doing.agents)),
        ("effects.rows", digest(&a.effects.rows)),
        ("effects.paths", digest(&a.effects.paths)),
        ("memory_days", digest(&a.memory_days)),
        ("renames", digest(&a.renames)),
        ("commits", digest(&(a.commits, a.unattributed))),
    ]
}

/// Everything the artefacts say, as one comparable value.
#[allow(dead_code)]
fn shape(a: &Agents) -> String {
    serde_json::to_string(&(
        &a.agents,
        &a.doing.rows,
        &a.doing.episodes,
        &a.doing.agents,
        &a.effects.rows,
        &a.effects.paths,
        &a.memory_days,
        &a.renames,
        a.commits,
        a.unattributed,
    ))
    .expect("serialise")
}

/// ⚠ A fixture that produces nothing would pass every comparison below.
fn refuse_vacuous(a: &Agents) {
    assert!(!a.agents.is_empty(), "fixture produced no agents");
    assert!(
        !a.doing.rows.is_empty(),
        "fixture produced no timeline rows"
    );
    assert!(!a.doing.episodes.is_empty(), "fixture produced no episodes");
    assert!(!a.effects.rows.is_empty(), "fixture produced no effects");
    assert!(
        a.agents.iter().any(|x| x.transcripts > 0),
        "fixture counted no transcripts"
    );
    // ⚠ Without a commit attributed, the doubling ablation cannot be seen.
    assert!(
        a.agents.iter().any(|x| x.commits > 0),
        "fixture attributed no commits"
    );
    // ⚠ Without ties on a minute, a stable sort cannot be caught ordering by
    // traversal — the whole point of the total-order comparator.
    let mut minutes: Vec<i64> = a.effects.rows.iter().map(|r| r.t).collect();
    minutes.sort_unstable();
    let ties = minutes.windows(2).filter(|w| w[0] == w[1]).count();
    assert!(
        ties > 4,
        "fixture has only {ties} tied minutes — too few to order wrongly"
    );
}

/// The property: cut anywhere, and resuming lands where reading whole lands.
#[test]
fn a_resumed_mine_equals_a_whole_mine_at_every_cut() {
    let c = corpus();
    let (whole, _) = c.mine(None);
    refuse_vacuous(&whole);
    let expected = dimensions(&whole);

    // Line boundaries across the file, including one inside the first episode
    // and one inside the last — the orphaning case is a cut mid-instruction.
    let lines: Vec<&str> = c.whole.lines().collect();
    let cuts = [
        lines.len() / 7,
        lines.len() / 3,
        lines.len() / 2,
        (lines.len() * 4) / 5,
    ];

    for cut in cuts {
        let head = lines[..cut].join("\n") + "\n";
        std::fs::write(&c.cut_in, &head).expect("truncate");
        let (head_pass, carried) = c.mine(None);

        // Restore the withheld REAL content, byte for byte.
        std::fs::write(&c.cut_in, &c.whole).expect("restore");
        // ⚠ The timeline and the evidence are carried THROUGH THE WIRE FORM,
        // which is how a real resume receives them. Passing `default()` here
        // instead makes the resumed run start with no rows and quietly compares
        // a tail against a whole corpus — that is a broken test, not a finding.
        let (resumed, _) = c.mine(Some(Resumed {
            carried,
            doing: through_wire(&head_pass.doing),
            effects: through_wire(&head_pass.effects),
        }));

        refuse_vacuous(&resumed);
        for ((name, got), (_, want)) in dimensions(&resumed).iter().zip(expected.iter()) {
            assert_eq!(
                got,
                want,
                "cut after {cut} of {} lines: {name} disagreed with a whole mine",
                lines.len()
            );
        }
    }
}

/// ⚠ **A cut that changes nothing must also change nothing.** This is the case
/// that cannot catch a per-read counter — it reads no transcript at all — and is
/// kept precisely so the difference between the two is on the record.
#[test]
fn resuming_with_nothing_changed_is_the_same_artefact() {
    let c = corpus();
    let (whole, carried) = c.mine(None);
    refuse_vacuous(&whole);
    let expected = dimensions(&whole);

    let (again, _) = c.mine(Some(Resumed {
        carried,
        doing: through_wire(&whole.doing),
        effects: through_wire(&whole.effects),
    }));
    for ((name, got), (_, want)) in dimensions(&again).iter().zip(expected.iter()) {
        assert_eq!(got, want, "a no-op resume changed {name}");
    }
}
