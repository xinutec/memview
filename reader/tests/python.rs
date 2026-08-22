//! What the Python reader may and may not conclude.
//!
//! Every test here is a shape the corpus actually writes, and the refusals
//! matter more than the successes: a missed write is an undercount, and an
//! invented path is a claim that somebody changed a file they never opened.

use reader::python::{Ran, Use, read};

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
fn open_reads_by_default_and_writes_when_the_mode_says_so() {
    assert_eq!(uses("open('src/x.ts').read()"), [used("src/x.ts", false)]);
    assert_eq!(uses("open('src/x.ts', 'w')"), [used("src/x.ts", true)]);
    assert_eq!(uses("open('src/x.ts', 'a')"), [used("src/x.ts", true)]);
    assert_eq!(uses("open('src/x.ts', 'rb')"), [used("src/x.ts", false)]);
    // The mode by keyword is the same call: `encoding=` sits where it would be.
    assert_eq!(
        uses("open('a.json', encoding='utf-8', mode='w')"),
        [used("a.json", true)]
    );
}

#[test]
fn pathlib_reads_and_writes() {
    assert_eq!(
        uses("Path('src/x.ts').write_text(body)"),
        [used("src/x.ts", true)]
    );
    assert_eq!(
        uses("pathlib.Path('src/x.ts').read_text()"),
        [used("src/x.ts", false)]
    );
    assert_eq!(
        uses("Path('/tmp/out.bin').unlink()"),
        [used("/tmp/out.bin", true)]
    );
}

/// The corpus's commonest shape by a distance, and the reason this tracks
/// constants at all.
#[test]
fn a_name_bound_once_to_a_literal_is_the_file_it_names() {
    let source = "\
p = Path('src/geo/velocity.ts')
s = p.read_text()
p.write_text(s.replace('a', 'b'))
";
    assert_eq!(
        uses(source),
        [
            used("src/geo/velocity.ts", false),
            used("src/geo/velocity.ts", true)
        ]
    );
}

#[test]
fn a_name_bound_twice_is_not_a_constant() {
    let source = "\
p = 'src/a.ts'
p = 'src/b.ts'
Path(p).write_text(x)
";
    assert!(uses(source).is_empty());
    assert_eq!(read(source).unresolved["write_text"], 1);
}

/// `for p in files:` names every file and therefore none.
#[test]
fn a_loop_variable_is_not_a_constant_even_when_it_was_one() {
    let source = "\
p = 'src/a.ts'
for p in files:
    Path(p).write_text(x)
";
    assert!(uses(source).is_empty());
}

#[test]
fn a_computed_argument_names_nothing() {
    // The danger this guards: recording `/x.ts` as the file, when the file is
    // whatever `root` was.
    assert!(uses("open(root + '/x.ts', 'w')").is_empty());
    assert!(uses("open(f'{root}/x.ts', 'w')").is_empty());
    assert!(uses("open('src/%s.ts' % name)").is_empty());
    assert_eq!(read("open(f'{root}/x.ts', 'w')").unresolved["open"], 1);
}

/// An f-string with no placeholder is an ordinary string.
#[test]
fn a_plain_f_string_still_names_its_file() {
    assert_eq!(uses("open(f'src/x.ts', 'w')"), [used("src/x.ts", true)]);
}

#[test]
fn moving_and_copying_read_one_end_and_write_the_other() {
    assert_eq!(
        uses("shutil.copy('a/in.png', 'b/out.png')"),
        [used("a/in.png", false), used("b/out.png", true)]
    );
    assert_eq!(
        uses("os.rename('old.rs', 'new.rs')"),
        [used("old.rs", false), used("new.rs", true)]
    );
    assert_eq!(
        uses("os.remove('/tmp/x.json')"),
        [used("/tmp/x.json", true)]
    );
}

/// A question about a file is not a use of it: `if not p.exists()` is the shape
/// of code that decides whether to touch a file at all.
#[test]
fn predicates_are_not_uses() {
    assert!(uses("Path('src/x.ts').exists()").is_empty());
    assert!(uses("os.path.exists('src/x.ts')").is_empty());
    assert!(uses("Path('src/x.ts').is_file()").is_empty());
}

#[test]
fn a_path_can_be_built_from_literals() {
    assert_eq!(
        uses("open(os.path.join('src', 'geo', 'x.ts'))"),
        [used("src/geo/x.ts", false)]
    );
    assert_eq!(
        uses("(Path('src') / 'geo' / 'x.ts').write_text(s)"),
        [used("src/geo/x.ts", true)]
    );
    assert_eq!(
        uses("Path('src').joinpath('x.ts').read_text()"),
        [used("src/x.ts", false)]
    );
    // …but only from literals.
    assert!(uses("open(os.path.join(root, 'x.ts'))").is_empty());
}

#[test]
fn a_call_inside_an_argument_is_counted_where_it_stands() {
    assert_eq!(
        uses("json.dump(data, open('state.json', 'w'), indent=2)"),
        [used("state.json", true)]
    );
    assert_eq!(
        uses("data = json.load(open('state.json'))"),
        [used("state.json", false)]
    );
}

/// A program that moves its own directory is reported, because every relative
/// path in it becomes a guess — the caller is the one that acts on it.
#[test]
fn changing_directory_is_reported() {
    let program = read("os.chdir('/tmp')\nopen('x.ts', 'w')");
    assert!(program.chdir);
    assert_eq!(
        program.uses,
        [Use {
            path: "x.ts".into(),
            write: true
        }]
    );
    assert!(!read("open('x.ts', 'w')").chdir);
}

#[test]
fn a_command_the_program_ran_is_kept_for_the_shell_reader() {
    // ⚠ **`subprocess.run` was the top of this reader's worklist** — 443 calls,
    // twice the next entry — and every one of them was a whole command whose
    // files nothing could see. The list form is what the corpus writes, and it
    // reaches `exec()` with no shell, so it stays an argv.
    assert_eq!(
        read("subprocess.run(['ffmpeg', '-i', 'a b.wav', 'out.wav'])").ran,
        [Ran::Argv(vec![
            "ffmpeg".to_string(),
            "-i".to_string(),
            "a b.wav".to_string(),
            "out.wav".to_string(),
        ])]
    );
    // `os.system` always has a shell, so its argument really is a script.
    assert_eq!(
        read("os.system('cd app && cat x.ts')").ran,
        [Ran::Script("cd app && cat x.ts".to_string())]
    );
    // ⚠ **A string without `shell=True` is NOT a script.** Python looks for a
    // program of that whole name and fails; reading it as shell would credit
    // the program with work it did not do.
    assert_eq!(
        read("subprocess.run('ls -la')").ran,
        [Ran::Argv(vec!["ls -la".to_string()])]
    );
    assert_eq!(
        read("subprocess.run('cd app && ls', shell=True)").ran,
        [Ran::Script("cd app && ls".to_string())]
    );
    // One unknown word makes the whole argv unusable: a hole in the middle
    // turns the next flag into a filename.
    let computed = read("subprocess.run(['ffmpeg', '-i', f])");
    assert!(computed.ran.is_empty());
    assert_eq!(computed.unresolved.get("subprocess.run"), Some(&1));
}

#[test]
fn what_it_cannot_read_is_counted_by_name() {
    let program = read("os.sync()\nET.parse('doc.xml')\nd.frobnicate()");
    assert_eq!(program.unknown["os.sync"], 1);
    // A module nobody named is indistinguishable from a variable, so its calls
    // land on the worklist as methods. Still the name to teach next.
    assert_eq!(program.unknown[".parse"], 1);
    assert_eq!(program.unknown[".frobnicate"], 1);
    // Understood and harmless is not the same as unread.
    assert!(read("print('hello')\nx = len(y)").unknown.is_empty());
    // Neither is a keyword before a bracket, which is not a call at all.
    assert!(read("x = t not in ('a.md', 'b.md')").unknown.is_empty());
    // …nor a function the program defined two lines up, however it defined it.
    let own = read("def ri():\n    return 1\nri()\nrf = lambda: 2\nrf()");
    assert!(own.unknown.is_empty(), "{:?}", own.unknown);
}

#[test]
fn a_database_and_a_picture_are_files_too() {
    // Opening a database uses that file. Read rather than written: whether a
    // statement later changed it is in the SQL, which this does not read.
    assert_eq!(
        uses("con = sqlite3.connect('/Volumes/Backup/recall/recall.sqlite')"),
        [used("/Volumes/Backup/recall/recall.sqlite", false)]
    );
    // Here the receiver is the picture and the *argument* is the file.
    assert_eq!(
        uses("img.save('out/frame.png')"),
        [used("out/frame.png", true)]
    );
}

/// `str.replace` is nine calls in ten and `Path.replace` is the tenth, and they
/// mean opposite things — so neither is read.
#[test]
fn replace_is_read_as_neither_of_its_two_meanings() {
    assert!(uses("Path('a.ts').read_text().replace('x', 'y')").len() == 1);
    assert!(read("s.replace('a', 'b')").unresolved.is_empty());
}

/// A heredoc's body arrives whole, quotes and all, and the awkward parts of
/// Python must not derail the reading of what is around them.
#[test]
fn the_shapes_around_a_use_do_not_break_it() {
    let source = "\
#!/usr/bin/env python3
\"\"\"A docstring with 'quotes' and a # hash in it.\"\"\"
import json, re
from pathlib import Path

def patch(target: str, old: str, new: str) -> None:
    body = Path(target).read_text()
    Path(target).write_text(body.replace(old, new))

rows = [r for r in data if r['kind'] == 'x']
cfg = {'a': 1, 'b': [2, 3]}
Path('android/app/src/main/AndroidManifest.xml').write_text(cfg)
with open('/tmp/report.json', 'w') as f:
    json.dump(rows, f)
";
    assert_eq!(
        uses(source),
        [
            used("android/app/src/main/AndroidManifest.xml", true),
            used("/tmp/report.json", true)
        ]
    );
}

/// A literal left unterminated — a heredoc cut short by a lost delimiter — must
/// not swallow the program that follows it.
#[test]
fn an_unterminated_string_ends_at_its_line() {
    // ⚠ **This assertion was REVERSED 2026-08-18, and deliberately.** It used to
    // expect `src/x.ts` back: the grammar stops a one-line literal at its line's
    // end, so reading continues and the write is found. Recovery is still right
    // for READING — but CPython refuses this source outright, so the write never
    // happened and counting it invented work (memview #1033).
    //
    // The grammar is unchanged; what is new is that `did_not_run` vetoes the
    // program first. Measured over all 12,240 distinct programs, the rule that
    // does this flags 11 and CPython refuses every one.
    let source = "\
x = 'oops
Path('src/x.ts').write_text(y)
";
    assert!(reader::python::did_not_run(source).is_some());
    assert!(uses(source).is_empty());
}

#[test]
fn a_directory_is_not_a_file() {
    assert!(uses("os.makedirs('build/out')").is_empty());
    assert!(uses("Path('build/out').mkdir(parents=True)").is_empty());
    // A walk over one is a read, and a glob is recorded as written.
    assert_eq!(
        uses("glob.glob('src/**/*.ts')"),
        [used("src/**/*.ts", false)]
    );
}

#[test]
fn a_program_that_could_not_have_run_names_no_files() {
    // ⚠ **Soundness, not coverage.** `f"{d[\"k\"]}"` is a `SyntaxError` on every
    // interpreter this corpus ran — 3.9.6 and 3.12.14 both, PEP 701
    // notwithstanding — so the program raised before its first statement. A
    // permissive grammar reads it happily and hands back the paths it mentions,
    // which are then recorded as work that happened.
    let program =
        read("import json\nd = json.load(open(\"/tmp/in.json\"))\nprint(f\"{d[\\\"k\\\"]}\")\n");
    assert_eq!(
        program.did_not_run,
        Some("an escaped quote in an f-string replacement field")
    );
    assert!(
        program.uses.is_empty(),
        "a program that raised used no file"
    );
}

#[test]
fn the_shapes_that_do_run_are_left_alone() {
    // The ablation. PEP 701's nested quote is valid on 3.12, a backslash in the
    // *literal* half of an f-string has always been valid, and a backslash in an
    // ordinary string has nothing to do with any of it.
    for source in [
        "d = json.load(open(\"/tmp/in.json\"))\nprint(f\"a\\\"b\")\n",
        "p = open(\"/tmp/in.json\")\nprint(f\"{d['k']}\")\n",
        "p = open(\"/tmp/in.json\")\nprint(\"a\\\"b\")\n",
        "p = open(\"/tmp/in.json\")\nprint(f\"{{literal}}\")\n",
    ] {
        let program = read(source);
        assert_eq!(program.did_not_run, None, "{source:?}");
        assert_eq!(program.uses.len(), 1, "{source:?}");
    }
}

#[test]
fn an_unclosed_literal_is_a_program_that_never_ran() {
    // Closure, not syntax: a quote that never finds its partner cannot be a
    // program under any reading, so saying so stays inside `python.pest`'s rule
    // that no program is rejected whole. The corpus's shape is a heredoc body
    // cut short (memview #1033).
    assert!(reader::python::did_not_run("x = '''unterminated\nmore text\n").is_some());
    assert!(reader::python::did_not_run("open('a.txt").is_some());
    assert!(reader::python::did_not_run("print(open('a.txt').read()").is_some());
}

#[test]
fn a_closed_literal_is_left_alone() {
    // ⚠ The direction that costs: flagging a program DISCARDS every file it
    // named, so a false positive destroys knowledge. Measured over all 12,240
    // distinct programs, this rule over-claims none.
    assert_eq!(
        reader::python::did_not_run("x = '''fine'''\nopen('a.txt').read()\n"),
        None
    );
    assert_eq!(reader::python::did_not_run("s = \"a ' b\"\n"), None);
    assert_eq!(
        reader::python::did_not_run("# a ' in a comment\nopen('a').read()\n"),
        None
    );
    assert_eq!(reader::python::did_not_run("s = 'it\\'s escaped'\n"), None);
    // A surplus CLOSE says nothing — the shell balanced the fragment around it.
    assert_eq!(reader::python::did_not_run("print('x'))\n"), None);
}
