//! What a program in a carried language did with files.
//!
//! One set of types for every such reader — [`crate::python`] and
//! [`crate::javascript`] — because there is nothing about a `Use`, a `Tally` or
//! a `Refused` that is about Python or about JavaScript. This repository's own
//! argument against the alternative is `gate.dhall`'s header: a thing written in
//! two places drifts in one of them, and these two would have drifted the first
//! time a counter was added to one report and not the other.
//!
//! The examples in the doc comments below stay Python, since that is the reader
//! they were measured on.

use std::collections::BTreeMap;

/// A file a carried program used, as the program named it.
///
/// The path is **unresolved on purpose**: what it is relative to is the
/// directory the shell was in, which is one layer up in
/// [`crate::shell_files`] — and that is also where the rule about which words
/// may become paths at all already lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Use {
    pub path: String,
    /// `true` when the program changed the file: an `open` in a writing mode, a
    /// `write_text`, an `os.remove`.
    pub write: bool,
}

/// A command a carried program handed to the system.
///
/// ⚠ **The same distinction `Op::RemoteRun` draws, and it is the same mistake
/// on the other side of it**: `subprocess.run(["ffmpeg", "-i", f])` and
/// `child_process.spawnSync(p, args)` reach `exec()` with no shell, so joining
/// their words and parsing the result as shell would invent quoting nobody
/// wrote. `os.system(s)` and `execSync(s)` really do go through a shell, and
/// their text really is a script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ran {
    /// Text a shell parses: `os.system("cd x && ls")`, `execSync(cmd)`.
    Script(String),
    /// An argv handed straight to `exec()`.
    Argv(Vec<String>),
}

/// What one carried program did with files, and what it did that this could
/// not read.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Program {
    pub uses: Vec<Use>,
    /// Commands the program ran. Followed by [`crate::shell_files`], which is
    /// where a shell's working directory lives — so `python3 -c 'os.system("cd
    /// x && cat y")'` reads to the end, and so does the Python inside THAT.
    ///
    /// **`subprocess.run` is the largest single thing either reader could not
    /// read** — 443 calls, top of the Python worklist on 2026-08-22, ahead of
    /// the next entry by a factor of two.
    pub ran: Vec<Ran>,
    /// File operations recognised, by name — `open`, `write_text`, `os.remove`.
    pub calls: BTreeMap<String, usize>,
    /// Those among them whose path was not knowable: an f-string, a loop
    /// variable, a computed join. Counted rather than guessed at, because the
    /// size of what is being dropped is the only honest way to read the rest.
    pub unresolved: BTreeMap<String, usize>,
    /// Operations whose path is one of a **known finite set** — a name the
    /// program bound to several literals — by the set, written `{a,b}`.
    ///
    /// ⚠ **One of them ran, not all of them.** Recording a use per candidate
    /// would claim a file was changed that never was, which is the one thing
    /// this reader promises never to do. Measured 2026-08-24, this is the
    /// commonest unnamed shape in the corpus at 37.9% of unresolved file
    /// operations, so the wrong version would have been wrong thousands of
    /// times.
    ///
    /// The same object as [`crate::shell_files::Extract::bounded`]: a language
    /// without a choice. `⟦p⟧ = some element of {out/a.txt, out/b.txt}`.
    ///
    /// ⚠ Still **not named**, and `subjects_not_named` counts these.
    pub bounded: BTreeMap<String, usize>,
    /// Those among [`Program::bounded`] whose candidates share a directory, by
    /// that directory — the locus is certain even though the leaf is not.
    ///
    /// ⚠ **An annotation, not a second account — the opposite of the shell.**
    /// `shell_files` puts a word in `bounded` OR `located`, so summing both is
    /// right there. Here every entry is ALSO in [`Program::bounded`], because a
    /// finite set of literals is a language and the shared directory is a fact
    /// about that same language rather than a weaker answer to it. So
    /// `subjects_not_named` counts `bounded` alone; adding this would count one
    /// operation twice.
    pub located: BTreeMap<String, usize>,
    /// Every other call, by name. **The worklist**: what tops this is what the
    /// reader should learn next, exactly as the grammar was grown.
    pub unknown: BTreeMap<String, usize>,
    /// Whether the program moved its own working directory.
    ///
    /// It cannot be followed — `os.chdir(sys.argv[1])` has no value here — so
    /// the honest response is to stop trusting relative paths out of that
    /// program. The caller enforces it; this only reports it.
    pub chdir: bool,
    /// Set when the text is not Python any interpreter would accept, so the
    /// program **raised a `SyntaxError` and ran none of itself**.
    ///
    /// ⚠ **`uses` is empty whenever this is set, and that is the point.** A
    /// permissive grammar reads a broken program as happily as a working one
    /// and hands back the paths it mentions — which are then recorded as work
    /// that happened. Soundness here is the same property the shell oracle
    /// asserts: never claim an operation the machine did not perform.
    pub did_not_run: Option<&'static str>,
}

/// What the Python across many programs did — the report's view, and the
/// worklist for growing this reader.
#[derive(Debug, Default)]
pub struct Tally {
    pub programs: usize,
    /// File uses found, before the caller's rules about what may be a path.
    pub uses: usize,
    /// Those that survived those rules and were attributed to somebody. Filled
    /// in by the caller, because the rule is the caller's.
    pub kept: usize,
    /// Those the caller's rules then turned away, by which rule. Filled in by
    /// the caller for the same reason `kept` is.
    ///
    /// `uses == kept + refused.total()` — every use is one or the other, and
    /// there is a test that says so.
    pub refused: Refused,
    pub calls: BTreeMap<String, usize>,
    pub unresolved: BTreeMap<String, usize>,
    /// See [`Program::bounded`]. Counted by `subjects_not_named`.
    pub bounded: BTreeMap<String, usize>,
    /// See [`Program::located`] — an annotation on `bounded`, NOT a second
    /// account, so nothing sums it.
    pub located: BTreeMap<String, usize>,
    pub unknown: BTreeMap<String, usize>,
    /// Programs that moved their own working directory, whose relative paths
    /// are therefore not trusted.
    pub chdir: usize,
    /// Commands these programs ran and this followed — see [`Program::ran`].
    pub ran: usize,
}

/// Why a use this reader *did* resolve still did not become a path.
///
/// ⚠ **Three different facts, and only the first is an unknown of the kind
/// [`Program::unresolved`] holds.** The program named a file plainly; what
/// stopped it is a rule of the layer above — which directory to read it
/// against, or whether a word may be a path at all. Kept apart because a rule
/// that turns away thousands is worth revisiting and an unknowable value is not.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Refused {
    /// The program called `os.chdir`, so its relative paths name no file this
    /// reader can find. The argument is usually computed, so the move cannot be
    /// followed and the paths cannot be trusted.
    pub moved: usize,
    /// Relative, with no directory to resolve it against — the shell's own `cd`
    /// went somewhere this reader could not follow either.
    pub no_directory: usize,
    /// Not shaped like a path, by the same rule a shell operand goes through.
    pub not_a_path: usize,
}

impl Refused {
    pub fn total(&self) -> usize {
        self.moved + self.no_directory + self.not_a_path
    }

    pub fn merge(&mut self, other: &Refused) {
        self.moved += other.moved;
        self.no_directory += other.no_directory;
        self.not_a_path += other.not_a_path;
    }
}

impl Tally {
    /// Fold one program's findings in.
    pub fn absorb(&mut self, program: Program) {
        self.programs += 1;
        self.uses += program.uses.len();
        self.chdir += usize::from(program.chdir);
        self.ran += program.ran.len();
        merge(&mut self.calls, program.calls);
        merge(&mut self.unresolved, program.unresolved);
        merge(&mut self.bounded, program.bounded);
        merge(&mut self.located, program.located);
        merge(&mut self.unknown, program.unknown);
    }

    /// Fold another tally in — a nested shell's Python is this script's Python.
    pub fn merge(&mut self, other: Tally) {
        self.programs += other.programs;
        self.uses += other.uses;
        self.kept += other.kept;
        self.refused.merge(&other.refused);
        self.chdir += other.chdir;
        self.ran += other.ran;
        merge(&mut self.calls, other.calls);
        merge(&mut self.unresolved, other.unresolved);
        merge(&mut self.bounded, other.bounded);
        merge(&mut self.located, other.located);
        merge(&mut self.unknown, other.unknown);
    }
}

fn merge(into: &mut BTreeMap<String, usize>, from: BTreeMap<String, usize>) {
    for (name, n) in from {
        *into.entry(name).or_insert(0) += n;
    }
}
