//! The lint pass that leaves the corpus and asks whether what it says is true.
//!
//! Every one of these is a shape that the fifteen intra-corpus rules pass
//! cleanly, because they ask whether the document graph is well-formed and this
//! asks whether it is accurate. The real case behind them: `lares` was retired
//! to `~/Archive/lares` and recorded in one memory while fifteen others still
//! sent a reader to `~/Code/lares`.

use memview::lint::{Severity, check_world};
use memview::store::Corpus;

/// Build a corpus of one memory whose body is `body`, plus a matching index.
fn corpus_saying(dir: &std::path::Path, body: &str) -> Corpus {
    std::fs::write(
        dir.join("MEMORY.md"),
        "# Memory index\n- [p](project_thing.md)\n",
    )
    .expect("write index");
    std::fs::write(
        dir.join("project_thing.md"),
        format!(
            "---\nname: project_thing\ndescription: d\nmetadata:\n  type: project\n---\n\n{body}\n"
        ),
    )
    .expect("write memory");
    Corpus::load(dir).expect("loads")
}

#[test]
fn a_named_repo_that_exists_is_not_a_finding() {
    let corpus_dir = tempfile::tempdir().expect("tempdir");
    let code = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(code.path().join("observe")).expect("mkdir");

    let corpus = corpus_saying(corpus_dir.path(), "Captures live at `~/Code/observe/data`.");
    assert!(check_world(&corpus, code.path()).is_empty());
}

#[test]
fn a_named_repo_that_is_gone_is_an_error() {
    let corpus_dir = tempfile::tempdir().expect("tempdir");
    let code = tempfile::tempdir().expect("tempdir");

    let corpus = corpus_saying(
        corpus_dir.path(),
        "Captures live at `~/Code/lares/captures`.",
    );
    let findings = check_world(&corpus, code.path());

    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].rule, "dead-repo-path");
    assert_eq!(findings[0].severity, Severity::Error);
    assert!(findings[0].detail.contains("lares"));
}

#[test]
fn naming_the_archive_location_records_the_retirement_and_clears_it() {
    // The escape hatch is deliberately "say where it went", not "silence this".
    let corpus_dir = tempfile::tempdir().expect("tempdir");
    let code = tempfile::tempdir().expect("tempdir");

    let corpus = corpus_saying(
        corpus_dir.path(),
        "Captures were at `~/Code/lares/captures`; the repo now lives at `~/Archive/lares`.",
    );
    assert!(check_world(&corpus, code.path()).is_empty());
}

#[test]
fn retiring_one_repo_does_not_excuse_a_stale_reference_to_another() {
    // The exemption is per repo, not per document — otherwise a single archive
    // note at the bottom of a long memory would waive every path in it.
    let corpus_dir = tempfile::tempdir().expect("tempdir");
    let code = tempfile::tempdir().expect("tempdir");

    let corpus = corpus_saying(
        corpus_dir.path(),
        "lares moved to `~/Archive/lares`. Deps still come from `~/Code/scanner`.",
    );
    let findings = check_world(&corpus, code.path());

    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(findings[0].detail.contains("scanner"), "{findings:?}");
}

#[test]
fn a_retirement_note_does_not_reach_a_live_path_further_down_the_document() {
    // The shape the per-document exemption missed, and it is the common one: a
    // long project memory opens with "retired to ~/Archive/lares" and then, forty
    // lines later, still tells a reader "captures live at ~/Code/lares/captures"
    // as a live instruction. Both are the SAME repo, so the per-repo check does
    // not separate them — the banner cleared the whole file.
    //
    // Measured on the real corpus 2026-08-12: `project_lares_recon` carried the
    // retirement banner in its first paragraph and four live paths below it, and
    // `dead-repo-path` was silent on all four.
    let corpus_dir = tempfile::tempdir().expect("tempdir");
    let code = tempfile::tempdir().expect("tempdir");

    let corpus = corpus_saying(
        corpus_dir.path(),
        "lares was retired to `~/Archive/lares`.\n\n\
         Some unrelated paragraph about the port.\n\n\
         Existing captures live at `~/Code/lares/captures` (gitignored, NOT /tmp).\n",
    );
    let findings = check_world(&corpus, code.path());

    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].rule, "dead-repo-path");
    assert!(findings[0].detail.contains("lares"), "{findings:?}");
}

#[test]
fn the_absolute_spelling_of_the_same_path_is_read_too() {
    // Both forms occur in the corpus and mean the same place; reading only the
    // tilde form would let the other rot unchecked. The absolute prefix comes
    // from the root under test, which is why no home path appears here.
    let corpus_dir = tempfile::tempdir().expect("tempdir");
    let code = tempfile::tempdir().expect("tempdir");

    let corpus = corpus_saying(
        corpus_dir.path(),
        &format!("See {}/lares/rust.", code.path().display()),
    );
    let findings = check_world(&corpus, code.path());

    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(findings[0].detail.contains("lares"), "{findings:?}");
}

#[test]
fn a_file_inside_a_live_repo_is_not_checked() {
    // Only the repo root is verified. A file inside one moves for ordinary
    // reasons, and reporting those would drown the real signal in churn.
    let corpus_dir = tempfile::tempdir().expect("tempdir");
    let code = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(code.path().join("health")).expect("mkdir");

    let corpus = corpus_saying(
        corpus_dir.path(),
        "Run `~/Code/health/scripts/a-script-that-does-not-exist.sh`.",
    );
    assert!(check_world(&corpus, code.path()).is_empty());
}

#[test]
fn an_unreadable_root_reports_rather_than_passing_silently() {
    // The failure mode this exists to prevent: a check that answers "no
    // findings" because it could not look reads exactly like a clean bill.
    let corpus_dir = tempfile::tempdir().expect("tempdir");
    let corpus = corpus_saying(corpus_dir.path(), "Captures at `~/Code/lares`.");

    let findings = check_world(&corpus, std::path::Path::new("/nonexistent/code/root"));

    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].rule, "unresolvable-code-root");
    assert_eq!(findings[0].severity, Severity::Error);
}

#[test]
fn a_url_that_merely_contains_the_root_path_is_not_a_repo_claim() {
    // The reason this pass parses markdown instead of scanning bytes. A link
    // destination is not a claim about a checkout, and `store.rs` already
    // carries the scar from a line-scanner that could not tell the difference.
    let corpus_dir = tempfile::tempdir().expect("tempdir");
    let code = tempfile::tempdir().expect("tempdir");

    let corpus = corpus_saying(
        corpus_dir.path(),
        &format!(
            "See [the plan](https://example.invalid{}/lares/plan.md).",
            code.path().display()
        ),
    );
    assert!(
        check_world(&corpus, code.path()).is_empty(),
        "a URL was read as a repo path claim"
    );
}

#[test]
fn a_command_inside_a_fenced_block_is_still_a_claim() {
    // Deliberately the opposite call from the index parser: there a link inside
    // a fence was noise, here a command in a fence is the most actionable thing
    // a memory can say.
    let corpus_dir = tempfile::tempdir().expect("tempdir");
    let code = tempfile::tempdir().expect("tempdir");

    let corpus = corpus_saying(
        corpus_dir.path(),
        "Run it:\n\n```sh\n~/Code/lares/deploy.sh\n```\n",
    );
    let findings = check_world(&corpus, code.path());

    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(findings[0].detail.contains("lares"), "{findings:?}");
}

#[test]
fn a_dead_path_named_only_in_the_description_is_caught() {
    // No markdown node covers frontmatter, so it is appended explicitly — and it
    // matters: the description is the half a reader sees first.
    let corpus_dir = tempfile::tempdir().expect("tempdir");
    let code = tempfile::tempdir().expect("tempdir");

    std::fs::write(
        corpus_dir.path().join("MEMORY.md"),
        "# Memory index\n- [p](project_thing.md)\n",
    )
    .expect("write index");
    std::fs::write(
        corpus_dir.path().join("project_thing.md"),
        "---\nname: project_thing\ndescription: \"captures at ~/Code/lares/captures\"\nmetadata:\n  type: project\n---\n\nNothing in the body.\n",
    )
    .expect("write memory");
    let corpus = Corpus::load(corpus_dir.path()).expect("loads");

    let findings = check_world(&corpus, code.path());
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert!(findings[0].detail.contains("lares"), "{findings:?}");
}

/// A repo with one commit, so a sha claim has something real to resolve against.
fn repo_with_a_commit(root: &std::path::Path, name: &str) -> String {
    let repo = root.join(name);
    std::fs::create_dir_all(&repo).expect("mkdir");
    let git = |args: &[&str]| {
        scrubbed_git(&repo).args(args).output().expect("git");
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "t@example.com"]);
    git(&["config", "user.name", "t"]);
    std::fs::write(repo.join("f"), "x").expect("write");
    git(&["add", "f"]);
    git(&["commit", "-qm", "one"]);
    let out = scrubbed_git(&repo)
        .args(["rev-parse", "--short=8", "HEAD"])
        .output()
        .expect("git");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// `git -C <dir>` with every inherited git variable removed.
///
/// ⚠ **`-C` sets the DIRECTORY and loses to `GIT_INDEX_FILE`, which wins.** These
/// tests run under `cargo test`, `cargo test` runs under the gate, and the gate
/// runs from `git commit`'s pre-commit hook — which exports `GIT_DIR` and
/// `GIT_INDEX_FILE` to everything it spawns. So `git -C <tempdir> add f` wrote
/// the entry into MEMVIEW'S index while the blob went to the tempdir's object
/// store, leaving `100644 c1b0730e… for 'f'` pointing at an object the repo does
/// not have. The commit then died on `error: Error building trees`.
///
/// It cost two failed commits and a wrong diagnosis: running the suite by hand
/// leaves the index byte-identical, because a shell has none of these variables
/// set. The bug only exists inside the hook, which is the one place it matters.
fn scrubbed_git(repo: &std::path::Path) -> std::process::Command {
    let mut c = std::process::Command::new("git");
    c.arg("-C").arg(repo);
    for var in [
        "GIT_DIR",
        "GIT_INDEX_FILE",
        "GIT_WORK_TREE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_COMMON_DIR",
        "GIT_CEILING_DIRECTORIES",
        "GIT_PREFIX",
        "GIT_CONFIG_PARAMETERS",
    ] {
        c.env_remove(var);
    }
    c
}

#[test]
fn a_commit_that_exists_is_not_a_finding() {
    let corpus_dir = tempfile::tempdir().expect("tempdir");
    let code = tempfile::tempdir().expect("tempdir");
    let sha = repo_with_a_commit(code.path(), "observe");

    let corpus = corpus_saying(corpus_dir.path(), &format!("Fixed in `{sha}`."));
    let findings: Vec<_> = check_world(&corpus, code.path())
        .into_iter()
        .filter(|f| f.rule == "unresolvable-commit")
        .collect();
    assert!(findings.is_empty(), "{findings:?}");
}

/// ⚠ The memory does NOT say which repo a sha belongs to, and its name does not
/// imply one. Resolving against the repo guessed from the name reported 65 dead
/// of 237 — and five of the first six existed in a different repo. The question
/// is asked of every repository, which is what this pins.
#[test]
fn a_commit_in_a_repo_the_memory_does_not_name_still_resolves() {
    let corpus_dir = tempfile::tempdir().expect("tempdir");
    let code = tempfile::tempdir().expect("tempdir");
    repo_with_a_commit(code.path(), "observe");
    let elsewhere = repo_with_a_commit(code.path(), "unrelated");

    let corpus = corpus_saying(corpus_dir.path(), &format!("Fixed in `{elsewhere}`."));
    let findings: Vec<_> = check_world(&corpus, code.path())
        .into_iter()
        .filter(|f| f.rule == "unresolvable-commit")
        .collect();
    assert!(findings.is_empty(), "{findings:?}");
}

#[test]
fn a_commit_no_repository_holds_is_reported() {
    let corpus_dir = tempfile::tempdir().expect("tempdir");
    let code = tempfile::tempdir().expect("tempdir");
    repo_with_a_commit(code.path(), "observe");

    let corpus = corpus_saying(corpus_dir.path(), "Fixed in `deadbee`.");
    let findings: Vec<_> = check_world(&corpus, code.path())
        .into_iter()
        .filter(|f| f.rule == "unresolvable-commit")
        .collect();
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].severity, Severity::Warning);
}

/// ⚠ A decimal number is valid hex and this corpus writes them in backticks —
/// `1048575` and `1234567` both appear. Neither is a commit.
#[test]
fn an_all_digit_token_is_not_read_as_a_commit() {
    let corpus_dir = tempfile::tempdir().expect("tempdir");
    let code = tempfile::tempdir().expect("tempdir");
    repo_with_a_commit(code.path(), "observe");

    let corpus = corpus_saying(
        corpus_dir.path(),
        "The cap is `1048575` bytes, not `1234567`.",
    );
    let findings: Vec<_> = check_world(&corpus, code.path())
        .into_iter()
        .filter(|f| f.rule == "unresolvable-commit")
        .collect();
    assert!(findings.is_empty(), "{findings:?}");
}

/// ⚠ A session-id prefix is eight hex characters and the corpus cites them in
/// backticks exactly as it cites shas.
#[test]
fn a_session_id_prefix_is_not_read_as_a_commit() {
    let corpus_dir = tempfile::tempdir().expect("tempdir");
    let code = tempfile::tempdir().expect("tempdir");
    repo_with_a_commit(code.path(), "observe");

    std::fs::write(
        corpus_dir.path().join("MEMORY.md"),
        "# Memory index\n- [p](project_thing.md)\n",
    )
    .expect("write index");
    std::fs::write(
        corpus_dir.path().join("project_thing.md"),
        "---\nname: project_thing\ndescription: d\nmetadata:\n  type: project\n  \
         originSessionId: 296dae53-3f84-4bd1-afbb-9ddcddedbdbb\n---\n\n\
         Written by `296dae53`, which is a session and not a commit.\n",
    )
    .expect("write memory");
    let corpus = Corpus::load(corpus_dir.path()).expect("loads");

    let findings: Vec<_> = check_world(&corpus, code.path())
        .into_iter()
        .filter(|f| f.rule == "unresolvable-commit")
        .collect();
    assert!(findings.is_empty(), "{findings:?}");
}

/// ⚠ The bug that cost two failed commits: `-C` sets the directory, `GIT_DIR`
/// wins.
///
/// `cargo test` runs under the gate, the gate runs from `git commit`'s
/// pre-commit hook, and that hook exports `GIT_DIR`/`GIT_INDEX_FILE` to every
/// child. Inherited, `git -C <tempdir> add f` wrote into the COMMITTING repo's
/// index while the blob went to the tempdir — `100644 c1b0730e… for 'f'`, an
/// entry pointing at an object that repo does not have, and the commit died on
/// `error: Error building trees`. Reproduced against a copy of the index, then
/// fixed.
///
/// ⚠ **Asserted on the command, not by setting the variables.** The obvious test
/// exports `GIT_DIR` and checks nothing leaks — but `std::env::set_var` is
/// process-global while cargo runs tests in parallel THREADS, so that test
/// corrupts whichever neighbour happens to shell out at the same moment. A
/// regression test for a contamination bug must not itself contaminate.
#[test]
fn the_test_helper_scrubs_every_inherited_git_variable() {
    let repo = std::path::Path::new("/tmp/does-not-need-to-exist");
    let cmd = scrubbed_git(repo);
    let removed: Vec<&str> = cmd
        .get_envs()
        .filter(|(_, v)| v.is_none())
        .filter_map(|(k, _)| k.to_str())
        .collect();
    for var in [
        "GIT_DIR",
        "GIT_INDEX_FILE",
        "GIT_WORK_TREE",
        "GIT_OBJECT_DIRECTORY",
    ] {
        assert!(removed.contains(&var), "{var} still inherited: {removed:?}");
    }
}

/// ⚠ A retired repository still holds its commits.
///
/// `dead-repo-path` accepts `~/Archive/<repo>` as the retirement record, so a
/// memory may legitimately cite a sha from a repo that has left `~/Code`.
/// Searching only the code root reported two real commits as unresolvable —
/// `lares` and `scanner-frozen` — the rule disagreeing with its neighbour about
/// where a retired repo lives.
#[test]
fn a_commit_in_the_archive_beside_the_code_root_still_resolves() {
    let corpus_dir = tempfile::tempdir().expect("tempdir");
    let home = tempfile::tempdir().expect("tempdir");
    let code = home.path().join("Code");
    std::fs::create_dir(&code).expect("mkdir");
    std::fs::create_dir(home.path().join("Archive")).expect("mkdir");

    let sha = repo_with_a_commit(&home.path().join("Archive"), "lares");
    let corpus = corpus_saying(corpus_dir.path(), &format!("Retired at `{sha}`."));

    let findings: Vec<_> = check_world(&corpus, &code)
        .into_iter()
        .filter(|f| f.rule == "unresolvable-commit")
        .collect();
    assert!(findings.is_empty(), "{findings:?}");
}

/// A root with no sibling archive finds nothing extra and does not reach outside.
#[test]
fn a_code_root_without_an_archive_beside_it_is_fine() {
    let corpus_dir = tempfile::tempdir().expect("tempdir");
    let code = tempfile::tempdir().expect("tempdir");
    repo_with_a_commit(code.path(), "observe");

    let corpus = corpus_saying(corpus_dir.path(), "Fixed in `deadbee`.");
    let findings: Vec<_> = check_world(&corpus, code.path())
        .into_iter()
        .filter(|f| f.rule == "unresolvable-commit")
        .collect();
    assert_eq!(findings.len(), 1, "{findings:?}");
}

/// ⚠ Not every repository the fleet uses lives under the code root.
///
/// `~/.config/home-manager` is one, and searching only `~/Code` reported five of
/// its commits as existing nowhere — which this rule's own text would have read
/// as "not cloned on this machine" about a repo that is right there.
#[test]
fn a_commit_in_a_config_repo_beside_the_code_root_still_resolves() {
    let corpus_dir = tempfile::tempdir().expect("tempdir");
    let home = tempfile::tempdir().expect("tempdir");
    let code = home.path().join("Code");
    std::fs::create_dir(&code).expect("mkdir");
    std::fs::create_dir(home.path().join(".config")).expect("mkdir");

    let sha = repo_with_a_commit(&home.path().join(".config"), "home-manager");
    let corpus = corpus_saying(corpus_dir.path(), &format!("Deployed by `{sha}`."));

    let findings: Vec<_> = check_world(&corpus, &code)
        .into_iter()
        .filter(|f| f.rule == "unresolvable-commit")
        .collect();
    assert!(findings.is_empty(), "{findings:?}");
}
