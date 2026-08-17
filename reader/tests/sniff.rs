//! The heredoc language sniff, against the shapes it was getting wrong.
//!
//! ⚠ **It had no tests until the third time it was wrong**, which is how a guess
//! stays wrong: the report it feeds prints a plausible number either way. Each
//! case here is a body shape taken from the corpus, with paths replaced.
use reader::sniff::looks_like;

fn guess(body: &str) -> &'static str {
    looks_like(body).0
}

#[test]
fn an_es_import_is_not_python() {
    // 504 bodies opened a line with `import ` and every one was called
    // Python. This is the commonest shape among them by a wide margin.
    assert_eq!(
        guess("import { readFileSync } from \"node:fs\";\nconst d = JSON.parse(readFileSync(f));"),
        "TypeScript or JavaScript"
    );
    assert_eq!(
        guess("import * as ts from 'typescript';\n"),
        "TypeScript or JavaScript"
    );
    assert_eq!(
        guess("import fs from \"node:fs\";\n"),
        "TypeScript or JavaScript"
    );
}

#[test]
fn a_bare_import_is_still_python() {
    // The ablation: the ES arm must not take the language it was added to
    // separate from.
    assert_eq!(guess("import sqlite3\nc = sqlite3.connect(db)\n"), "Python");
    assert_eq!(
        guess("import re, shutil, subprocess, sys\nSRC = x\n"),
        "Python"
    );
}

#[test]
fn lean_writes_def_too() {
    // The reason this matters here rather than in general: the health port
    // is Lean, so `def` is not a rare word in this corpus.
    assert_eq!(
        guess("def pi : Float := 3.141592653589793\ndef lats : List Float := []\n"),
        "Lean"
    );
    assert_eq!(guess("structure S where\n  a : Int\n  b : Int\n"), "Lean");
    assert_eq!(
        guess("@[noinline] def intLoop (n : Nat) : Int := Id.run do\n"),
        "Lean"
    );
    // ...and a Python def is not Lean.
    assert_eq!(
        guess("def test_es_log_is_transient() -> None:\n    assert True\n"),
        "Python"
    );
}

#[test]
fn a_sentence_that_says_from_is_not_python() {
    // A task body wrapped onto a line beginning "from" and was filed as a
    // program. Python's mark is the `import` half, not the `from` half.
    assert_eq!(
        guess(
            "A send path for IRC: say it through irssi.\nfrom the reader's side nothing changes.\n"
        ),
        "prose, or nothing recognised"
    );
    assert_eq!(guess("from pathlib import Path\np = Path(x)\n"), "Python");
}

#[test]
fn kotlin_is_not_stolen_by_its_second_line() {
    // `package` decided this body, but `import` was tested first and won.
    assert_eq!(
        guess("package org.lares.capture\n\nimport org.junit.Test\nimport java.io.File\n"),
        "Kotlin or Java"
    );
}

#[test]
fn swift_imports_bare_like_python_and_is_not_python() {
    assert_eq!(
        guess("import CoreAudio\nimport Foundation\n\nlet sys = AudioObjectID(0)\n"),
        "Swift"
    );
}

#[test]
fn the_marks_that_were_already_right_stay_right() {
    assert_eq!(
        guess("#!/usr/bin/env bash\nset -e\n"),
        "a script with a shebang"
    );
    assert_eq!(guess("{\"a\": 1}"), "JSON");
    assert_eq!(guess("SELECT count(*) FROM rows;"), "SQL");
    assert_eq!(guess("apiVersion: v1\nkind: Service\n"), "YAML");
    assert_eq!(guess("set -e\nfor f in *.log; do echo $f; done\n"), "shell");
}
