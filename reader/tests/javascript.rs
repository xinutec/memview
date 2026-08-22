//! What the JavaScript reader may and may not conclude.
//!
//! Every test here is a shape the corpus actually writes, and — as in
//! `tests/python.rs` — the refusals matter more than the successes: a missed
//! write is an undercount, and an invented path is a claim that somebody
//! changed a file they never opened.

use reader::javascript::{Ran, Use, read};

/// The uses a program made, as `(path, write)` pairs in the order they happened.
fn uses(source: &str) -> Vec<(String, bool)> {
    read(source)
        .uses
        .into_iter()
        .map(|Use { path, write }| (path, write))
        .collect()
}

fn used(path: &str, write: bool) -> (String, bool) {
    (path.to_string(), write)
}

#[test]
fn a_read_and_a_write_are_told_apart_by_the_call() {
    assert_eq!(
        uses(r#"fs.readFileSync("src/x.ts", "utf8")"#),
        [used("src/x.ts", false)]
    );
    assert_eq!(
        uses(r#"fs.writeFileSync("src/x.ts", body)"#),
        [used("src/x.ts", true)]
    );
    // Destructured out of `require("fs")`, which is how this corpus writes it.
    assert_eq!(
        uses(r#"readFileSync("src/x.ts")"#),
        [used("src/x.ts", false)]
    );
}

#[test]
fn a_name_bound_once_to_a_literal_is_the_path() {
    assert_eq!(
        uses(r#"const p = "src/x.ts"; const s = fs.readFileSync(p); fs.writeFileSync(p, s);"#),
        [used("src/x.ts", false), used("src/x.ts", true)]
    );
}

#[test]
fn a_name_bound_twice_is_not_a_constant() {
    // The whole rule: two bindings mean the value at the call site is whichever
    // ran, and this reader does not know which.
    assert!(uses(r#"let p = "a.ts"; p = "b.ts"; fs.readFileSync(p);"#).is_empty());
}

#[test]
fn a_loop_variable_is_not_a_constant_even_when_it_shadows_one() {
    // `for (const f of files)` binds inside brackets, where a first draft of the
    // grammar could not see it — and a name that is secretly rebound is the one
    // way a constant becomes a path nobody touched.
    assert!(
        uses(r#"const f = "real.ts"; for (const f of list) { fs.readFileSync(f); }"#).is_empty()
    );
}

#[test]
fn a_template_with_a_substitution_names_no_file() {
    // ⚠ Reading `${dir}/x.ts` as `/x.ts` would invent an absolute path.
    assert!(uses("fs.readFileSync(`${dir}/x.ts`)").is_empty());
    // One with nothing substituted is an ordinary string.
    assert_eq!(
        uses("fs.readFileSync(`src/x.ts`)"),
        [used("src/x.ts", false)]
    );
}

#[test]
fn a_bare_module_specifier_is_a_package_and_not_a_file() {
    // ⚠ `@angular/compiler` has a slash in it, so every "looks like a path"
    // test ever written lets it through. Node's rule is the one that decides.
    assert!(uses(r#"require("fs")"#).is_empty());
    assert!(uses(r#"require("node:fs")"#).is_empty());
    assert!(uses(r#"require("@angular/compiler")"#).is_empty());
    assert_eq!(
        uses(r#"require("./dist/db/pool.js")"#),
        [used("./dist/db/pool.js", false)]
    );
    assert_eq!(
        uses(r#"require("/app/dist/db/pool.js")"#),
        [used("/app/dist/db/pool.js", false)]
    );
}

#[test]
fn a_static_import_names_the_file_it_loaded() {
    assert_eq!(
        uses(r#"import { migrate } from "./dist/db/schema.js";"#),
        [used("./dist/db/schema.js", false)]
    );
}

#[test]
fn a_dynamic_import_is_a_call_and_keeps_what_is_chained_onto_it() {
    // The shape that showed the statement rule was stealing the call: the
    // reader took `import('x'` and left `).then(…)` to start a new statement,
    // which is how `then` and `map` got onto the worklist.
    let program = read(r#"import("./dist/config.js").then(m => console.log(m))"#);
    assert_eq!(
        program.uses.into_iter().map(|u| u.path).collect::<Vec<_>>(),
        ["./dist/config.js"]
    );
    assert!(program.unknown.is_empty(), "{:?}", program.unknown);
}

#[test]
fn a_chain_that_runs_over_a_line_is_still_one_chain() {
    // `raw.filter(…)⏎  .map(…)` is how this corpus writes it. Without the
    // newline rule the second call starts a statement of its own.
    let program = read("const gps = raw.filter(p => p.ok)\n  .map(p => p.ts);\n");
    assert!(program.unknown.is_empty(), "{:?}", program.unknown);
}

#[test]
fn a_call_whose_arguments_span_lines_is_still_a_call() {
    assert_eq!(
        uses("fs.writeFileSync(\n  \"out/x.json\",\n  JSON.stringify(data),\n)"),
        [used("out/x.json", true)]
    );
}

#[test]
fn a_regex_literal_is_not_a_division_and_does_not_eat_the_program() {
    // A `/` where an atom is expected can only open a regex — and a regex
    // holding a quote would otherwise close a string that was never open.
    let program = read(r#"const s = fs.readFileSync("a.ts", "utf8").replace(/['"]/g, "");"#);
    assert_eq!(
        program.uses.into_iter().map(|u| u.path).collect::<Vec<_>>(),
        ["a.ts"]
    );
}

#[test]
fn a_program_cut_short_ran_none_of_itself() {
    // ⚠ A heredoc that lost its terminator leaves a program no runtime accepts,
    // and reading its paths would record work that never happened.
    let program = read(r#"fs.writeFileSync("out.txt", "half"#);
    assert!(program.did_not_run.is_some());
    assert!(program.uses.is_empty());
}

#[test]
fn a_shell_command_is_kept_as_a_script_and_a_spawn_as_an_argv() {
    // ⚠ The distinction node's own manual draws: `execSync` goes through
    // `/bin/sh`, `spawnSync` does not. Joining a spawn's words into a script
    // would invent quoting nobody wrote.
    assert_eq!(
        read(r#"execSync("cd app && cat x.ts")"#).ran,
        [Ran::Script("cd app && cat x.ts".to_string())]
    );
    assert_eq!(
        read(r#"spawnSync("ffmpeg", ["-i", "a b.wav", "out.wav"])"#).ran,
        [Ran::Argv(vec![
            "ffmpeg".to_string(),
            "-i".to_string(),
            "a b.wav".to_string(),
            "out.wav".to_string(),
        ])]
    );
}

#[test]
fn one_unknown_word_makes_the_whole_argv_unusable() {
    // ⚠ A hole in the middle of an argv turns the next flag into a filename:
    // `["-i", f]` with `f` computed would read as a file called `-i`.
    let program = read(r#"spawnSync("ffmpeg", ["-i", f, "out.wav"])"#);
    assert!(program.ran.is_empty());
    assert_eq!(program.unresolved.get("spawnSync"), Some(&1));
}
