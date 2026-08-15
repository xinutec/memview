//! The shell grammar, against the shapes the transcripts actually contain.
//!
//! Every case here was taken from the corpus, and the ones marked as regressions
//! are constructs the grammar got wrong at some point — each cost a measurable
//! slice of the 83,799 distinct commands.

use reader::shell::{Reached, Simple, parse};

/// One command's argv, for compact assertions.
fn argv(cmd: &Simple) -> &[String] {
    &cmd.argv
}

/// The argv of every simple command, for compact assertions.
fn argvs(script: &str) -> Vec<Vec<String>> {
    parse(script)
        .unwrap_or_else(|at| panic!("failed to parse, stopped at {at:?}"))
        .into_iter()
        .map(|c: Simple| c.argv)
        .collect()
}

#[test]
fn a_pipeline_of_plain_commands() {
    assert_eq!(
        argvs("git status --short | head -3"),
        [vec!["git", "status", "--short"], vec!["head", "-3"]]
    );
}

#[test]
fn quotes_are_removed_and_adjacent_runs_are_one_word() {
    // `--flag="a b"` is a single argument. Treating the quoted run as a separate
    // word would split every flag the fleet's scripts pass this way.
    assert_eq!(
        argvs(r#"cmd --flag="a b" 'one two' "$HOME"/Code"#),
        [vec!["cmd", "--flag=a b", "one two", "$HOME/Code"]]
    );
}

#[test]
fn a_hash_inside_quotes_is_not_a_comment() {
    // REGRESSION. The quoting rules were not atomic, so pest inserted its
    // implicit comment-skipping *inside* a string: the `#` in a nix flake
    // reference or a sed expression started a comment that ate the closing
    // quote. 2,058 commands — 2.5% of the corpus — failed on this one thing.
    assert_eq!(
        argvs(r#"nix develop "/home/example/Code/lares#android" --command true"#),
        [vec![
            "nix",
            "develop",
            "/home/example/Code/lares#android",
            "--command",
            "true"
        ]]
    );
    assert_eq!(
        argvs("sed 's#a#b#g' in.py > out.py"),
        [vec!["sed", "s#a#b#g", "in.py"]]
    );
}

#[test]
fn a_comment_outside_quotes_still_ends_the_line() {
    assert_eq!(
        argvs("echo one # this is ignored\necho two"),
        [vec!["echo", "one"], vec!["echo", "two"]]
    );
}

#[test]
fn a_heredoc_body_is_not_shell() {
    // The body is data. Parsed as shell it yields commands nobody ran — here it
    // would invent `print(1)` and `import os`.
    assert_eq!(
        argvs("python3 - <<'PY'\nimport os\nprint(1)\nPY\necho after"),
        [vec!["python3", "-"], vec!["echo", "after"]]
    );
}

#[test]
fn a_heredoc_body_reaches_the_command_that_opened_it() {
    // Not shell, but not thrown away either: it is the program `python3 -` was
    // given, and 3,547 file writes live in bodies like it.
    let cmds = parse("python3 - <<'PY'\nimport os\nprint(1)\nPY\necho after").unwrap();
    assert_eq!(cmds[0].heredocs, ["import os\nprint(1)\n"]);
    assert!(cmds[1].heredocs.is_empty());
}

#[test]
fn two_heredocs_on_one_line_keep_their_own_bodies() {
    let cmds = parse("diff <(cat <<'A'\none\nA\n) <(cat <<'B'\ntwo\nB\n)").unwrap();
    let bodies: Vec<&[String]> = cmds.iter().map(|cmd| cmd.heredocs.as_slice()).collect();
    assert!(
        bodies.iter().any(|b| b == &["one\n"]) && bodies.iter().any(|b| b == &["two\n"]),
        "each body went to its own command, got {bodies:?}"
    );
}

#[test]
fn a_heredoc_nested_in_a_quoted_argument_terminates_at_its_delimiter() {
    // REGRESSION, and the corpus's commonest heredoc shape. The inner shell sees
    // a bare `PY`, but on disk that line reads `PY'` — the closing quote of the
    // outer argument sits on it. Requiring an exact match meant the terminator
    // was never found, the rest of the script was eaten as body, and the quote it
    // closed was left open.
    // The body is taken out of the line even here, inside the quoted argument,
    // because that pass works on lines and does not track quoting. It is
    // **carried in the delimiter** rather than dropped, and that is why: the
    // body belongs to the inner `python3 -`, which is not parsed until this
    // argument is re-read as a script, and a marker in the text is the only
    // thing that survives the trip.
    let cmds = parse("bash -c 'python3 - <<PY\nprint(1)\nPY'\necho done").unwrap();
    assert_eq!(argv(&cmds[1]), ["echo", "done"]);
    let inner = parse(&cmds[0].argv[2]).unwrap();
    assert_eq!(argv(&inner[0]), ["python3", "-"]);
    assert_eq!(inner[0].heredocs, ["print(1)\n"]);
}

#[test]
fn redirections_do_not_become_arguments() {
    let cmds = argvs("cargo test > /tmp/log 2>&1");
    assert_eq!(cmds, [vec!["cargo", "test"]]);
}

#[test]
fn a_backgrounded_brace_group_is_commands_not_a_word() {
    assert_eq!(
        argvs("{ echo DONE; tail -2 /tmp/x; } &"),
        [vec!["echo", "DONE"], vec!["tail", "-2", "/tmp/x"]]
    );
}

#[test]
fn brace_expansion_stays_part_of_its_word() {
    // REGRESSION. `{` opens a group and also expands a word; they are told apart
    // by whitespace, exactly as bash tells them apart. Splitting here would turn
    // one path into a word ending in `/`.
    assert_eq!(
        argvs("grep -rln x {home,thoth,life}/android"),
        [vec!["grep", "-rln", "x", "{home,thoth,life}/android"]]
    );
}

#[test]
fn command_substitution_is_parsed_as_the_commands_it_is() {
    // The inner command opens files too, so it must not be swallowed as text.
    // The inner command is emitted first — it runs first — and the outer word
    // keeps the substitution as written. Nothing is expanded: there is no value
    // to expand it to, and inventing one would be a guess about the past.
    assert_eq!(
        argvs("REV=$(git -C ~/Code/dev-lint rev-parse HEAD) nix run ."),
        [
            vec!["git", "-C", "~/Code/dev-lint", "rev-parse", "HEAD"],
            vec![
                "REV=$(git -C ~/Code/dev-lint rev-parse HEAD)",
                "nix",
                "run",
                "."
            ]
        ]
    );
}

#[test]
fn a_substitution_inside_double_quotes_is_still_a_command() {
    // ⚠ **REGRESSION, and the largest one this reader has had.** `dquoted` was
    // fully atomic, so `"$( … )"` was a single opaque token and the command
    // inside it was never walked — 8,300 distinct commands, 6.5% of the corpus,
    // 12,755 occurrences (memview#918). Worse in kind than an unparsed command:
    // that at least raises a count, while this parsed clean and recorded less
    // than happened.
    //
    // The real shape from the corpus, which is a read of a real file:
    assert_eq!(
        argvs(r#"printf "%s" "$(grep -c foo src/app/thread.scss)""#),
        [
            vec!["grep", "-c", "foo", "src/app/thread.scss"],
            vec!["printf", "%s", "$(grep -c foo src/app/thread.scss)"],
        ]
    );
    // Nesting works because the body is re-parsed, quoted or not. The outer
    // word keeps the substitution as written *minus its quotes*, which is
    // `unquote`'s long-standing reading of a word as a value.
    assert_eq!(
        argvs(r#"echo "$(dirname "$(readlink -f x)")""#),
        [
            vec!["readlink", "-f", "x"],
            vec!["dirname", "$(readlink -f x)"],
            vec!["echo", "$(dirname $(readlink -f x))"],
        ]
    );
    // Backticks expand inside double quotes too.
    assert_eq!(
        argvs(r#"echo "`git rev-parse HEAD`""#),
        [
            vec!["git", "rev-parse", "HEAD"],
            vec!["echo", "`git rev-parse HEAD`"],
        ]
    );
    // ⚠ **And it corrects WORD BOUNDARIES, which is the half that was not just
    // invisible but wrong.** Quoting restarts inside `$( )`, so the `"` around
    // `x` below does not end the outer string. The old rule scanned to the first
    // unescaped `"` and stopped there, splitting one argument into three and
    // failing outright on 17 corpus commands. The `echo` takes TWO arguments.
    assert_eq!(
        argvs(r#"echo "$(grep -c "x" f.txt)" done"#),
        [
            vec!["grep", "-c", "x", "f.txt"],
            vec!["echo", "$(grep -c x f.txt)", "done"],
        ]
    );
}

#[test]
fn single_quotes_expand_nothing_and_that_asymmetry_is_the_point() {
    // ⚠ **The one direction this fix must NOT go.** Inside single quotes a
    // substitution is six characters of text, so walking it would invent a
    // command nobody ran — the error this reader exists to avoid, and the reason
    // `squoted` stays fully atomic while `dquoted` is compound-atomic.
    assert_eq!(
        argvs("echo '$(rm -rf /tmp/x)'"),
        [vec!["echo", "$(rm -rf /tmp/x)"]]
    );
    // A `\$` does not open one either, and a bare `$` is ordinary text.
    assert_eq!(
        argvs(r#"echo "\$(not-a-command)" "$HOME/x""#),
        [vec!["echo", "$(not-a-command)", "$HOME/x"]]
    );
    // An escaped quote does not end the string — the alternative that has to be
    // tried before everything else, or `\"` closes the word early.
    assert_eq!(
        argvs(r#"echo "a \" b $(ls) c""#),
        [vec!["ls"], vec!["echo", r#"a " b $(ls) c"#]]
    );
}

#[test]
fn arithmetic_is_not_a_command_substitution() {
    // REGRESSION. `$((` matched as `$(` and stopped at the first `)`, leaving the
    // second dangling — 216 `until [ $(( $(date +%s) - t0 )) -ge 10 ]` loops.
    // Two documented limits show here, both deliberate. The arithmetic body is
    // skipped whole rather than descended into — nothing inside `$(( ))` is a
    // file, so the `$(date +%s)` within it is not reported. And `until`/`do`/
    // `done` are ordinary words, because a keyword rule would have to guess
    // whether `echo done` ends a loop. Neither names a file, so neither can put
    // a wrong path in the index.
    assert_eq!(
        argvs("until [ $(( $(date +%s) - t0 )) -ge 10 ]; do sleep 1; done"),
        [
            vec!["until", "[", "$(( $(date +%s) - t0 ))", "-ge", "10", "]"],
            vec!["do", "sleep", "1"],
            vec!["done"],
        ]
    );
}

#[test]
fn escaped_parens_belong_to_the_word() {
    // REGRESSION. `find . \( … \)` — the word ended at the backslash and the bare
    // `(` opened a group that never closed.
    //
    // ⚠ The backslash is gone from the value, because the shell removes it:
    // `printf '%s|' find . \( … \)` prints `find|.|(|…`, so `(` is the word
    // `find` was handed. argv is what the command received, not what was typed.
    assert_eq!(
        argvs(r"find . \( -name '*.kt' -o -name '*.rs' \)"),
        [vec![
            "find", ".", "(", "-name", "*.kt", "-o", "-name", "*.rs", ")"
        ]]
    );
}

#[test]
fn a_find_placeholder_is_an_argument() {
    // `\;` too: `find` is handed a bare `;`, which is why it needed escaping
    // from the shell in the first place.
    assert_eq!(
        argvs(r"find . -name '*.tmp' -exec rm {} \;"),
        [vec![
            "find", ".", "-name", "*.tmp", "-exec", "rm", "{}", ";"
        ]]
    );
}

#[test]
fn a_subshell_runs_commands() {
    assert_eq!(
        argvs("(cd android && ./gradlew build)"),
        [vec!["cd", "android"], vec!["./gradlew", "build"]]
    );
}

#[test]
fn process_substitution_is_a_command_too() {
    assert_eq!(
        argvs("diff <(ls a) <(ls b)"),
        [vec!["ls", "a"], vec!["ls", "b"], vec!["diff"]]
    );
}

#[test]
fn a_line_continuation_joins_one_command() {
    assert_eq!(
        argvs("rsync -a \\\n  --delete \\\n  src/ dst/"),
        [vec!["rsync", "-a", "--delete", "src/", "dst/"]]
    );
}

#[test]
fn an_unclosed_quote_is_an_error_not_a_silent_half_parse() {
    // The point of a restrictive grammar: what it cannot read, it says it cannot
    // read. A parser that accepts this would report a command list that quietly
    // omits whatever followed.
    assert!(parse("echo 'unterminated").is_err());
}

/// What had to hold for each command in a script to run, in order.
#[test]
fn a_backslash_outside_quotes_escapes_the_character_after_it() {
    // ⚠ **`'\''` is how a single quote gets inside a single-quoted string** —
    // close, escaped quote, reopen — and reading the `\'` as two literal
    // characters gave back a word with a backslash where the quote belongs.
    // Found by the round-trip probe (memview#833) rather than by a failing case:
    // 274 corpus calls contain a `\'`, mostly `ssh host '…'` payloads whose
    // inner script quotes something.
    assert_eq!(argvs(r"echo 'it'\''s here'"), [vec!["echo", "it's here"]]);
    // Inside single quotes nothing is special, backslash included.
    assert_eq!(argvs(r"echo 'a\b'"), [vec!["echo", r"a\b"]]);
    // A backslash escaping an operator keeps the word whole, and the operator
    // arrives as the plain character the command was handed.
    assert_eq!(argvs(r"echo a\;b"), [vec!["echo", "a;b"]]);
}

fn conditions(script: &str) -> Vec<Reached> {
    parse(script)
        .unwrap_or_else(|at| panic!("failed to parse, stopped at {at:?}"))
        .into_iter()
        .map(|c: Simple| c.reached)
        .collect()
}

#[test]
fn the_separator_says_whether_the_next_command_runs() {
    // ⚠ **The only thing in the text that says a command happened.** Without
    // this the reader credits an agent with `b` in `a && b` when `a` failed and
    // `b` never ran — 1,220 file uses in the corpus's failed calls.
    assert_eq!(conditions("a; b"), [Reached::Always, Reached::Always]);
    assert_eq!(conditions("a && b"), [Reached::Always, Reached::OnSuccess]);
    // Exit 0 on `a || b` cannot tell "a worked and b was skipped" from "a failed
    // and b worked", so `||` is not the mirror of `&&` on the status the corpus
    // is mostly made of. A non-zero exit IS the mirror and is knowable; it is
    // declined on size rather than on principle — see the case below and
    // `Reached::Sometimes`.
    assert_eq!(conditions("a || b"), [Reached::Always, Reached::Sometimes]);
    // A newline and `&` end a list exactly as `;` does.
    assert_eq!(conditions("a\nb"), [Reached::Always, Reached::Always]);
}

#[test]
fn a_chain_of_ands_is_still_one_condition() {
    // Each link needs everything before it, which is the same condition, not a
    // deeper one — otherwise a long chain would decay into "cannot tell".
    assert_eq!(
        conditions("a && b && c"),
        [Reached::Always, Reached::OnSuccess, Reached::OnSuccess]
    );
}

#[test]
fn the_knowledge_in_a_failed_or_chain_is_declined_on_size_not_on_principle() {
    // ⚠ **These three are all `Sometimes`, and one of them need not be.**
    // memview #101 asked for a fourth domain point, `OnFailure`, to recover the
    // right-hand side of a `||` when the call failed. Measured 2026-08-15 over
    // 132,554 calls: 4,945 failed at all, 390 of those contain `||`, and 113
    // file uses inside them land in this bucket — a ceiling, and 0.59% of it.
    // Too small to pay for, so the reader keeps saying "cannot tell".
    //
    // ⚠ **What the measuring found that the task had not.** The recoverable
    // knowledge is not one point in a lattice, it is one sentence: *a non-zero
    // exit proves the last `||` alternative of the final segment ran.* Both
    // shapes below fall out of it — a chain, because each link failing in turn
    // is the only way to reach the end; and the mixed form, because `a && b`
    // exiting non-zero sends control to `c` whichever way it got there. Neither
    // needs `OnFailure`, and neither needs `Reached::and` touched. `b` is the
    // part no domain point could ever confirm.
    assert_eq!(
        conditions("a || b || c"),
        [Reached::Always, Reached::Sometimes, Reached::Sometimes]
    );
    assert_eq!(
        conditions("a && b || c"),
        [Reached::Always, Reached::OnSuccess, Reached::Sometimes]
    );
    // And the scope is kept, so a rule that ever does read this can tell an
    // alternative inside a subshell from one at the top level.
    assert_eq!(
        conditions("( a || b )"),
        [Reached::Always, Reached::Sometimes]
    );
}

#[test]
fn a_status_a_semicolon_threw_away_can_never_be_confirmed() {
    // ⚠ **The call reports ONE exit status, and `;` discards the one before
    // it.** In `a && b; c` exit 0 says `c` worked and nothing whatever about
    // `a`, so `b` is unconfirmable however the call turned out — not merely
    // conditional on success. Counting it as certain is an over-claim worth
    // 15,981 file uses in the corpus's *successful* calls alone.
    assert_eq!(
        conditions("a && b; c"),
        [Reached::Always, Reached::Sometimes, Reached::Always]
    );
    // The last segment keeps its chain: this is the one the status answers.
    assert_eq!(
        conditions("a; b && c"),
        [Reached::Always, Reached::Always, Reached::OnSuccess]
    );
}

#[test]
fn at_most_one_arm_of_an_if_ran_so_neither_is_certain() {
    // ⚠ **The one place this reader could invent a file use.** Every other gap
    // in it records less than happened; recording both arms as certain records
    // MORE, and does it under the label that means "this definitely happened".
    // The condition is not a branch — `grep` really runs.
    assert_eq!(
        conditions("if grep -q foo a.txt; then cat b.txt; else cat c.txt; fi"),
        [
            Reached::Always,
            Reached::Sometimes,
            Reached::Sometimes,
            Reached::Always,
        ]
    );
    // An `elif` test is itself reached only when the one before it failed.
    assert_eq!(
        conditions("if a; then b; elif c; then d; fi"),
        [
            Reached::Always,
            Reached::Sometimes,
            Reached::Sometimes,
            Reached::Sometimes,
            Reached::Always,
        ]
    );
    // `fi` closes exactly one level, so what follows is certain again. ⚠ One
    // command can carry TWO keywords: `then if b` both stands in the outer
    // branch and opens an inner one, and reading only its first word would let
    // the inner `fi` close the outer statement — putting `d` back on the
    // certain side of a branch it is not in.
    assert_eq!(
        conditions("if a; then if b; then c; fi; fi; d"),
        [
            Reached::Always,    // if a
            Reached::Sometimes, // then if b
            Reached::Sometimes, // then c
            Reached::Sometimes, // fi — the inner statement, inside the outer arm
            Reached::Always,    // fi — closes the outer statement, not in an arm
            Reached::Always,    // d
        ]
    );
    // A substitution inside a branch did not run either — it is walked under the
    // command that holds it, which is why this lives in the walk and not in a
    // pass over the flat list afterwards.
    assert_eq!(
        conditions("if a; then cat $(ls d); fi"),
        [
            Reached::Always,
            Reached::Sometimes, // ls d
            Reached::Sometimes, // cat
            Reached::Always,
        ]
    );
}

#[test]
fn a_condition_reaches_inside_what_it_guards() {
    // A subshell that may not run holds commands that may not run, whatever
    // separates them from each other.
    assert_eq!(
        conditions("a && (b; c)"),
        [Reached::Always, Reached::OnSuccess, Reached::OnSuccess]
    );
}

#[test]
fn a_verdict_and_a_condition_together_decide_what_certainly_ran() {
    use reader::doing::Verdict;
    // The one that carries the corpus: unconditional commands survive any
    // outcome, which is why `a; b; c` keeps all three even when the call failed.
    assert!(Verdict::Failed.admits(Reached::Always));
    assert!(Verdict::Ok.admits(Reached::Always));
    // An `&&` is answered by exit 0 and by nothing else.
    assert!(Verdict::Ok.admits(Reached::OnSuccess));
    assert!(!Verdict::Failed.admits(Reached::OnSuccess));
    // A refusal is a fact about the process rather than about how it went, so
    // it overrides even an unconditional command.
    assert!(!Verdict::Rejected.admits(Reached::Always));
    // Silence is not refusal: a transcript can lack a result because it was
    // interrupted or is still running, and reading that as "nothing ran" would
    // drop every shell file use in it.
    assert!(Verdict::Unknown.admits(Reached::Always));
    assert!(!Verdict::Unknown.admits(Reached::OnSuccess));
}

#[test]
fn a_closing_keyword_is_not_a_command_and_ends_no_segment() {
    // ⚠ **`done`, `fi` and `esac` arrive here looking like unconditional
    // commands sitting after the body they close.** Treated as real ones, the
    // last of them anchors the final segment and demotes every `&&` in the whole
    // script — one `for` loop was enough to make everything before it
    // unconfirmable.
    assert_eq!(
        conditions("for f in x; do a && b; done"),
        [
            Reached::Always,
            Reached::Always,
            Reached::OnSuccess,
            Reached::Always
        ]
    );
}

/// `case … esac`, the one compound this grammar had to be taught.
///
/// Every other one — `for`, `while`, `until`, `if` — needs no rule: its keywords
/// survive as ordinary words and the commands between them parse like any
/// others. `case` cannot be waved through, because `completed*)` closes a paren
/// that was never opened, so the command dies at the FIRST arm.
///
/// ⚠ **The reason to care is not the `case`.** It is almost always the CI or
/// deploy wait — poll, match the status, break — so while the statement would
/// not parse, every `ssh`, `kubectl` and file write in the loop AROUND it was
/// invisible. 126 of 366 unreadable commands, three times the next bucket
/// (`reader/examples/unparsed-probe.rs`, 2026-08-15).
#[test]
fn a_case_arm_is_not_a_stray_paren() {
    assert_eq!(
        argvs(r#"case "$s" in completed*) echo done;; *) echo waiting;; esac"#),
        [vec!["echo", "done"], vec!["echo", "waiting"]]
    );
    // A pattern list, the optional opening paren, and an arm with no body at
    // all — `*.sample) ;;` is how the corpus skips a case it does not want.
    assert_eq!(
        argvs("case $f in (a|b|c*) touch x;; *.sample) ;; esac"),
        [vec!["touch", "x"]]
    );
    // The last arm may omit its terminator, which is what makes `esac` able to
    // be swallowed as a command by the arm's own body.
    assert_eq!(argvs("case $f in *) touch x; esac"), [vec!["touch", "x"]]);
}

#[test]
fn at_most_one_case_arm_ran_so_none_of_them_is_certain() {
    // The same reasoning as the two halves of an `if`, and the same answer: the
    // arms are alternatives, so recording one as certain claims a file use that
    // never happened.
    //
    // ⚠ **The subject is not an arm.** `case $(readlink -f x) in` really does
    // run `readlink`, whichever way the match goes, so it keeps the condition
    // standing outside the statement.
    assert_eq!(
        conditions("case $(readlink -f x) in a) touch p;; *) touch q;; esac"),
        [Reached::Always, Reached::Sometimes, Reached::Sometimes]
    );
    // `&&` inside one arm still needs what precedes it — the arm's uncertainty
    // and the separator's condition meet rather than one replacing the other.
    assert_eq!(
        conditions("case $x in a) p && q;; esac"),
        [Reached::Sometimes, Reached::Sometimes]
    );
}

#[test]
fn a_case_inside_a_loop_is_reached_through_the_do_keyword() {
    // REGRESSION, and the reason a rule that only allowed a statement at the
    // start of a command found almost nothing: `do` is an ordinary word to this
    // grammar, so `do case "$f" in …` puts the keyword and the statement in ONE
    // command. This is the corpus's commonest `case` by a wide margin.
    //
    // The point of the test is the `ssh` — the whole loop was unreadable while
    // the `case` in it was, so a remote command nobody could see ran every
    // fifteen seconds.
    assert_eq!(
        argvs(
            r#"for i in 1 2 3; do s=$(ssh host status); case "$s" in done*) break;; esac; sleep 15; done"#
        ),
        [
            vec!["for", "i", "in", "1", "2", "3"],
            vec!["ssh", "host", "status"],
            vec!["do", "s=$(ssh host status)"],
            vec!["break"],
            vec!["sleep", "15"],
            vec!["done"],
        ]
    );
}

#[test]
fn defining_a_function_runs_none_of_it() {
    // ⚠ **The other place this reader could invent a file use**, and it was
    // there from the round that added `name() { … }` — surfacing only when the
    // #901 grammar made nine more definitions parse. Binding a name writes
    // nothing, so recording the body as certain credits a write to `/tmp/o` to a
    // line that only said what `f` would do if anyone called it.
    //
    // The body is kept, not dropped: when the function IS called, its commands
    // are the only place those effects appear — the call site names no files.
    assert_eq!(
        conditions("f() { curl -s http://x > /tmp/o; }\necho hi"),
        [Reached::Sometimes, Reached::Always]
    );
    // The definition does not make what FOLLOWS it uncertain — only its own
    // body, which is the difference between this and an `if` left open.
    assert_eq!(
        argvs("f() { touch a; }\ntouch b"),
        [vec!["touch", "a"], vec!["touch", "b"]]
    );
}

#[test]
fn a_subshell_after_a_loop_keyword_is_still_a_subshell() {
    // REGRESSION, and the same one command over: `do ( … )` puts the keyword and
    // the group together, so a `group` reachable only at the start of a command
    // was unreachable in a loop. This was most of what the failure report filed
    // under "subshell / grouping", and the argument for caring is the same as
    // for `case` — `(cd $d && git commit …)` is a real write nobody could see.
    assert_eq!(
        argvs("for d in a b; do (cd $d && git commit -m x) ; done"),
        [
            vec!["for", "d", "in", "a", "b"],
            vec!["cd", "$d"],
            vec!["git", "commit", "-m", "x"],
            vec!["do"],
            vec!["done"],
        ]
    );
    // ⚠ **The subshell must keep its own directory.** That is the whole reason
    // the group is not just flattened into the enclosing command: without a
    // scope of its own, every later relative path resolves against `$d`.
    let cmds = parse("for d in a b; do (cd $d && git commit -m x) ; done").unwrap();
    let committed = cmds.iter().find(|c| c.argv[0] == "git").unwrap();
    assert!(!committed.scope.is_empty());
    let after = cmds.iter().find(|c| c.argv[0] == "done").unwrap();
    assert!(after.scope.is_empty());
}

/// ⚠ **`<<` is not an operator just because it is two characters.**
///
/// The opener scan reads bytes, so anything merely CONTAINING `<<` looked like
/// a heredoc: an arithmetic shift, a quoted string that mentions redirection, a
/// grep pattern hunting for one. The scan then ate the rest of the script
/// looking for a terminator that was never coming — silently, with no error, so
/// every command after such a line vanished from the parse and every file they
/// touched went unattributed.
///
/// The tempting fix is to ignore `<<` inside quotes, and it is wrong: the
/// corpus's commonest heredoc is `bash -c 'python3 - <<PY … PY'`, whose `<<` is
/// inside a quoted argument and is entirely real. Quoting does not separate
/// these. What separates them is whether a terminator ever arrives.
#[test]
fn a_shift_is_not_a_heredoc() {
    // The arithmetic survives as one word, because the grammar knows `$(( ))`
    // even though the opener scan does not. All the scan has to do is keep its
    // hands off it.
    assert_eq!(
        argvs("n=$((1 << 3))\necho done"),
        [vec!["n=$((1 << 3))"], vec!["echo", "done"]]
    );
}

#[test]
fn a_quoted_mention_of_redirection_is_not_a_heredoc() {
    assert_eq!(
        argvs("echo 'use << to redirect'\necho done"),
        [vec!["echo", "use << to redirect"], vec!["echo", "done"]]
    );
}

#[test]
fn a_pattern_that_looks_for_a_heredoc_does_not_open_one() {
    // The shape that started this: grepping the corpus for heredoc openers.
    assert_eq!(
        argvs("grep \"<<'EOF'\" file.txt\necho done"),
        [vec!["grep", "<<'EOF'", "file.txt"], vec!["echo", "done"]]
    );
}

#[test]
fn a_heredoc_inside_a_quoted_argument_is_still_a_heredoc() {
    // The case the naive fix would break, and the commonest shape in the corpus.
    //
    // ⚠ The body is NOT on the outer command, and asserting that it was is how
    // this test first failed against correct code. It belongs to `python3 -`,
    // which is not a command on this pass at all — it is text inside an argument,
    // re-parsed later from this very substring. The marker travels with the text
    // so that the second pass can still find it.
    let cmds = parse("bash -c 'python3 - <<PY\nimport os\nPY'\necho after").unwrap();
    assert!(cmds.iter().any(|c| c.argv == ["echo", "after"]));

    let nested = parse(&cmds[0].argv[2]).unwrap();
    assert_eq!(nested[0].argv, ["python3", "-"]);
    assert_eq!(nested[0].heredocs, ["import os\n"]);
}
