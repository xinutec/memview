//! What the projection must keep, and where it already reads better than the
//! grammar it is meant to replace.
//!
//! Each case here was found by `cargo run -p reader --bin projection`, which runs
//! both readers over the corpus and groups the disagreements. The counts in the
//! comments are that report's, taken over 134,622 distinct commands in the
//! since-retired `corpus/union.jsonl` — they are what makes a case worth a test
//! rather than a curiosity, and they are the thing to re-measure rather than
//! trust. Re-measure against `~/.claude/memview/bash-corpus.jsonl`; it is a
//! superset, so a count here can only be low.
//!
//! ⚠ **The disagreements are the point, so they are asserted on both sides.**
//! A test that only pinned the tree's answer would pass just as well after
//! somebody made the flat reader agree by making the tree wrong.

use reader::project::project;
use reader::shell::{self, Reached};
use reader::shell_ops::unwrap_command;
use reader::syntax;

/// The commands the tree finds, as the flat chain would see them.
fn tree(script: &str) -> Vec<reader::shell::Simple> {
    project(&syntax::parse(script).expect("the fixture must parse"))
}

fn argv(script: &str) -> Vec<Vec<String>> {
    tree(script).into_iter().map(|cmd| cmd.argv).collect()
}

/// 21,481 commands — 16.0% of the corpus, and the largest disagreement there is.
///
/// Inside double quotes a backslash escapes only `$`, `` ` ``, `"`, `\` and a
/// newline; before anything else it is a character. Measured in
/// `reader/probes/quoting.sh`: `"\.lpass"` is seven characters and bash prints it
/// back with the backslash on.
#[test]
fn a_backslash_in_double_quotes_is_usually_a_character() {
    assert_eq!(
        argv(r#"grep -E "\.lpass" f"#),
        [["grep", "-E", r"\.lpass", "f"]]
    );
    // The escapes that really are escapes still go.
    assert_eq!(
        argv(r#"echo "a\"b" "c\\d" "e\$f""#),
        [["echo", r#"a"b"#, r"c\d", "e$f"]]
    );
    // ⚠ **The flat reader dropped every one of them until this comparison
    // pointed at it**, which turned each of 21,481 commands' regexes into a
    // different regex. Asserted on that reader too, because the fix is in
    // `shell.rs` and the tree is what caught it.
    let flat = shell::parse(r#"grep -E "\.lpass" f"#).unwrap();
    assert_eq!(flat[0].argv[2], r"\.lpass");
}

/// 1,649 commands, and a silent loss: the command still parsed, still
/// classified, and named one word less than it was given.
///
/// A descriptor has to touch its own operator — `2>&1`, never `2 >&1` — and in
/// the flat grammar the digits sat in a repetition, where pest's implicit
/// whitespace is free to get in. So `nc -w3 host 25 2>&1` read `25 2` as one
/// digit run and the port was gone, as was the `1` of every `--limit 1 2>&1`.
#[test]
fn a_number_before_a_redirection_is_still_an_argument() {
    for script in [
        "nc -w3 host 25 2>&1",
        "adb shell monkey -c LAUNCHER 1 >/dev/null",
    ] {
        let flat = shell::parse(script).unwrap();
        assert_eq!(flat[0].argv, argv(script)[0], "{script}");
        assert!(
            flat[0]
                .argv
                .last()
                .is_some_and(|word| word == "25" || word == "1")
        );
    }
}

/// ⚠ **The three shapes where the flat reader claimed a command certainly ran
/// and it had not**, which is the one direction of error this reader is built to
/// avoid. All three are fixed in `shell.rs` and `shell.pest`; asserted on both
/// readers, because a silent regression here over-counts rather than under.
#[test]
fn the_flat_reader_no_longer_over_claims_certainty() {
    // 621 commands: `do` and `if` land in one pipeline, and `leading_keywords`
    // stopped at the `do` — so the branch never opened and its body read as
    // certain. Both arms of an `if` cannot have run.
    let script = r#"for p in a b; do if [ -d "$p" ]; then rm -rf "$p"; fi; done"#;
    for cmds in [tree(script), shell::parse(script).unwrap()] {
        // Through `unwrap_command`, because the flat grammar leaves the `then`
        // it read as a word standing in front of the command it guards.
        let rm = cmds
            .iter()
            .find(|c| unwrap_command(&c.argv).first().is_some_and(|w| w == "rm"))
            .expect("the body is there");
        assert_eq!(rm.reached, Reached::Sometimes, "{script}");
    }

    // 86 commands: a connector at the end of a line. `a &&⏎b` is one and-or
    // list — bash's own grammar puts a newline_list after the `&&` — and reading
    // the newline as a separator put `b` back at unconditional.
    for cmds in [tree("a &&\nb"), shell::parse("a &&\nb").unwrap()] {
        assert_eq!(cmds[1].reached, Reached::OnSuccess);
    }

    // 18 commands: `&>file` writes, and the `&` read as a background separator
    // left the log file belonging to no command at all.
    for cmds in [
        tree("chrome --port=9222 &>/tmp/chrome.log"),
        shell::parse("chrome --port=9222 &>/tmp/chrome.log").unwrap(),
    ] {
        assert_eq!(cmds.len(), 1, "one command, not a background and an orphan");
        assert_eq!(cmds[0].redirects.len(), 1);
        assert_eq!(cmds[0].redirects[0].target, "/tmp/chrome.log");
        assert!(cmds[0].redirects[0].write);
    }
}

/// ⚠ **`${n}_v4` is not `$n_v4`** — they name different variables, and a value
/// built one segment at a time loses the difference because the braces are
/// needed only by what FOLLOWS. 8 commands, all of them redirection targets,
/// all of them a file nobody was recorded as writing to. The spelling lives in
/// `syntax::print::print_value` for this reason.
#[test]
fn a_parameter_keeps_the_braces_that_hold_its_name_together() {
    assert_eq!(argv("replay > /tmp/${n}_v4.svg")[0], ["replay"]);
    assert_eq!(
        tree("replay > /tmp/${n}_v4.svg")[0].redirects[0].target,
        "/tmp/${n}_v4.svg"
    );
    // And drops the ones nothing needs, which is the printer's canonical form.
    assert_eq!(argv("echo ${n} x")[0], ["echo", "$n", "x"]);
}

/// 1,048 commands, and the one class of disagreement left that the flat reader
/// cannot fix: `a && for f in x; do b; done` runs `b` only if `a` worked, and
/// the `;` before `do` resets the condition because to that grammar `do` is an
/// ordinary word. Carrying it across would mean rebuilding the loop structure
/// the tree simply has.
#[test]
fn a_loop_after_an_and_carries_the_condition_into_its_body() {
    let cmds = tree("a && for f in x y; do b; done");
    let b = cmds.iter().find(|c| c.argv == ["b"]).expect("the body ran");
    assert_eq!(b.reached, Reached::OnSuccess);
    let flat = shell::parse("a && for f in x y; do b; done").unwrap();
    let flat_b = flat
        .iter()
        .find(|c| c.argv.last().is_some_and(|w| w == "b"))
        .unwrap();
    assert_eq!(
        flat_b.reached,
        Reached::Always,
        "the flat reader stopped being wrong — retire this arm"
    );
}

/// ⚠ **The word holding a substitution has two readings, and only one of them
/// is a reading.** Both readers find the inner command and agree about its
/// words. What they disagree about — 2,271 commands, in three buckets the report
/// names `quoting`, `spacing` and `something else` — is how the substitution is
/// *spelled back* into the outer argv string: the flat reader carries the source
/// with a layer of quoting taken off, and this one reprints it from the tree, so
/// `2>/dev/null` comes back as `2> /dev/null` and `'%Lp'` loses quotes it never
/// needed.
///
/// Neither spelling is wrong, because nothing downstream reads inside an
/// expansion — a substitution's value is undetermined and is counted as such.
/// It is asserted here so that the count stays a spelling difference: the moment
/// one of these strings is re-parsed as a script, it stops being one.
#[test]
fn an_inner_command_is_read_the_same_and_spelled_differently() {
    // A quote inside a quote: single quotes hold the program, double quotes are
    // part of it, and the whole substitution stands inside double quotes again.
    let script = r#"echo "$(awk '{print $5" bytes"}' f)""#;
    let program = r#"{print $5" bytes"}"#;
    let flat = shell::parse(script).unwrap();
    assert_eq!(argv(script)[0], ["awk", program, "f"]);
    assert_eq!(
        flat[0].argv,
        ["awk", program, "f"],
        "the inner command agrees"
    );

    // The word around it does not, and this is the whole of that difference.
    assert_eq!(argv(script)[1][1], r#"$(awk '{print $5" bytes"}' f)"#);
    assert_eq!(flat[1].argv[1], r#"$(awk '{print $5 bytes}' f)"#);
}

/// ⚠ **The one thing this reader exists to get right.** Neither arm of an `if`,
/// no arm of a `case`, and no line of a function body is certain — and the
/// subject of a `case` is not an arm at all.
#[test]
fn only_what_certainly_ran_is_certain() {
    let cmds = tree("case $(readlink -f p) in a) touch x ;; *) touch y ;; esac");
    let subject = cmds.iter().find(|c| c.argv[0] == "readlink").unwrap();
    assert_eq!(subject.reached, Reached::Always);
    for arm in cmds.iter().filter(|c| c.argv[0] == "touch") {
        assert_eq!(arm.reached, Reached::Sometimes);
    }

    let cmds = tree("if grep -q x f; then a; else b; fi");
    assert_eq!(cmds[0].reached, Reached::Always); // the condition really runs
    assert_eq!(cmds[1].reached, Reached::Sometimes);
    assert_eq!(cmds[2].reached, Reached::Sometimes);

    // Defining a function runs none of it.
    let cmds = tree("f() { rm -rf tmp; }");
    assert_eq!(cmds[0].argv, ["rm", "-rf", "tmp"]);
    assert_eq!(cmds[0].reached, Reached::Sometimes);
}

/// A subshell keeps its own directory, and two sibling ones are not one scope —
/// `(cd a && x); (cd b && y)` must not resolve `y` against `a`.
#[test]
fn each_subshell_gets_its_own_scope() {
    let cmds = tree("(cd a && x); (cd b && y); z");
    let scopes: Vec<&[usize]> = cmds.iter().map(|c| c.scope.as_slice()).collect();
    assert_eq!(scopes[0], scopes[1], "one group is one scope");
    assert_eq!(scopes[2], scopes[3], "and so is the next");
    assert_ne!(scopes[0], scopes[2], "but they are not the same one");
    assert!(scopes[4].is_empty(), "and `z` is back at the top level");
    // A brace group forks no shell, so its `cd` really does move the caller.
    assert!(tree("{ cd a; }").iter().all(|c| c.scope.is_empty()));
}

/// The commands inside a word run before the word is a word, and a `$( … )`
/// inside double quotes is the same node as one outside — the asymmetry that
/// hid 8,300 commands from the flat reader.
#[test]
fn a_substitution_runs_before_the_command_holding_it() {
    assert_eq!(
        argv(r#"echo "$(git rev-parse HEAD)""#),
        [
            vec!["git".to_string(), "rev-parse".into(), "HEAD".into()],
            vec!["echo".into(), "$(git rev-parse HEAD)".into()]
        ]
    );
    // Single quotes expand nothing, so there is no command in here at all.
    assert_eq!(argv("echo '$(rm -rf /)'"), [["echo", "$(rm -rf /)"]]);
}

/// A heredoc body reaches the command that opened it, whatever else the line
/// carries — it is the only reason `python3 - <<'PY'` is readable at all.
#[test]
fn a_heredoc_body_lands_on_its_own_command() {
    let cmds = tree("cat <<'A' | wc -l\none\nA");
    assert_eq!(cmds[0].argv, ["cat"]);
    assert_eq!(cmds[0].heredocs, ["one\n"]);
    assert!(
        cmds[1].heredocs.is_empty(),
        "the body belongs to `cat`, not `wc`"
    );
}

/// `time` and `!` are grammar, not a command name — so neither reaches argv,
/// and what is left is the program that ran.
#[test]
fn a_pipeline_prefix_is_not_a_command() {
    assert_eq!(argv("time ! ls -l"), [["ls", "-l"]]);
}

/// The loops the text determines, run out into what they ran.
///
/// ⚠ **This is where the reader stops looking commands up and starts evaluating
/// them**, so each rule below is bash's and each is asserted rather than
/// described. `project` is what the script SAYS and `run_out` is what it DID;
/// the two differ only here.
#[test]
fn a_loop_the_text_determines_is_run_out() {
    let ran = |script: &str| -> Vec<Vec<String>> {
        reader::project::run_out(&syntax::parse(script).expect("the fixture must parse"))
            .commands
            .into_iter()
            .map(|cmd| cmd.argv)
            .collect()
    };

    // The body once per value, with the variable standing for it — and the head
    // kept, because it holds the list the body no longer shows.
    assert_eq!(
        ran("for f in a.log b.log; do wc -l \"$f\"; done"),
        [
            vec!["for", "f", "in", "a.log", "b.log"],
            vec!["wc", "-l", "a.log"],
            vec!["wc", "-l", "b.log"]
        ]
    );
    // ⚠ `${f}x` needs no special case: the tree holds one node for both
    // spellings, so substituting it is exact where rewriting text was not.
    assert_eq!(ran("for f in a; do echo ${f}x; done")[1], ["echo", "ax"]);
    // An operator is a transduction this reader does not perform, so the value
    // is not put in underneath one.
    assert_eq!(
        ran("for f in a.txt; do echo ${f%.txt}; done")[1],
        ["echo", "${f%.txt}"]
    );

    // `$(seq …)` is arithmetic on numbers already written down, not a question
    // for anybody — the one program this reader runs in its head.
    // Five, not four: the `seq` in the head really does run, and is emitted
    // before the loop it feeds.
    assert_eq!(
        ran("for i in $(seq 1 3); do echo $i; done"),
        [
            vec!["seq", "1", "3"],
            vec!["for", "i", "in", "$(seq 1 3)"],
            vec!["echo", "1"],
            vec!["echo", "2"],
            vec!["echo", "3"]
        ]
    );
    // And an empty range is an answer: the body ran no times.
    assert_eq!(
        ran("for i in $(seq 3 1); do echo $i; done"),
        [vec!["seq", "3", "1"], vec!["for", "i", "in", "$(seq 3 1)"]]
    );

    // A glob is answered by a filesystem that is gone, so the loop stays folded
    // — but its head keeps the pattern, which is the only place it appears.
    assert_eq!(
        ran("for f in */; do ls $f; done"),
        [vec!["for", "f", "in", "*/"], vec!["ls", "$f"]]
    );

    // Loops nest, and the inner one is walked once per outer value.
    // The inner head is emitted once per outer value, because that is when it
    // was reached.
    assert_eq!(
        ran("for a in 1 2; do for b in x; do echo $a$b; done; done"),
        [
            vec!["for", "a", "in", "1", "2"],
            vec!["for", "b", "in", "x"],
            vec!["echo", "1x"],
            vec!["for", "b", "in", "x"],
            vec!["echo", "2x"]
        ]
    );
}

/// ⚠ **A body that may have run zero times must not be recorded as certainly
/// run** — the same over-claim as both arms of an `if`, and a much larger one.
/// The rule is bash's: `while` and `until` test first, a `for` over words that
/// are all written out runs once per word, and a glob counts as written out
/// because with `nullglob` off a pattern matching nothing expands to itself.
#[test]
fn a_loop_body_is_certain_only_if_the_loop_certainly_ran_it() {
    // Through `run_out`, because "it may have run no times" is a statement
    // about running: `project` says what the text holds and leaves it alone,
    // exactly as `shell::parse` does.
    let body = |script: &str| -> Reached {
        reader::project::run_out(&syntax::parse(script).expect("the fixture must parse"))
            .commands
            .into_iter()
            .find(|cmd| {
                unwrap_command(&cmd.argv)
                    .first()
                    .is_some_and(|w| w == "touch")
            })
            .expect("the body is there")
            .reached
    };
    assert_eq!(body("for f in a b; do touch $f; done"), Reached::Always);
    assert_eq!(body("for f in *.log; do touch $f; done"), Reached::Always);
    assert_eq!(
        body("for i in $(seq 1 3); do touch $i; done"),
        Reached::Always
    );
    assert_eq!(
        body("for f in $(ls); do touch $f; done"),
        Reached::Sometimes
    );
    assert_eq!(
        body("for f in $EVERY; do touch $f; done"),
        Reached::Sometimes
    );
    assert_eq!(body("while read f; do touch $f; done"), Reached::Sometimes);
    assert_eq!(body("until done_yet; do touch x; done"), Reached::Sometimes);
    assert_eq!(
        body("select f in a b; do touch $f; done"),
        Reached::Sometimes
    );
    assert_eq!(
        body("for ((i=0;i<3;i++)); do touch $i; done"),
        Reached::Sometimes
    );
}
