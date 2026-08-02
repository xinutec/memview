//! Which files a shell command used, and which way — as a projection of what
//! the command *does*.
//!
//! [`crate::shell`] reads the syntax and [`crate::shell_ops`] reads the meaning;
//! this is the last step, and it is deliberately the dull one. Each [`Op`]
//! decides its own direction — a `Copy` reads its sources and writes its
//! destination, a `Transform` writes only when it is in place, a `Git::Stage`
//! contributes nothing because staging changes no file — so this is a `match`
//! over the operations rather than a second table to keep in step with the
//! first.
//!
//! What is left here that is not a projection is the one thing an operation
//! cannot know alone: **the working directory**, which `cd` moves and a subshell
//! restores. That needs the sequence, so it lives with the loop over it.

use std::collections::BTreeMap;

use crate::shell::Simple;
use crate::shell_ops::{GitOp, Op, basename, classify, looks_like_path, resolve, unwrap_command};

/// A file another machine's command used.
///
/// Kept entirely apart from [`FileUse`], and that separation is the point: the
/// path exists on `host`, not here, so it can be reported and counted but must
/// never reach an index of local work. Reading the script is what makes "who
/// maintains isis's nixos config" answerable at all; mixing the two would make
/// every answer about this machine wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteUse {
    pub host: String,
    pub path: String,
    pub write: bool,
}

/// A file a command used, absolute, and which way it went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileUse {
    pub path: String,
    /// `true` when the command changed the file: a `>` redirect, `sed -i`, the
    /// destination of a `cp`, an `rm`. Reads and writes stay apart for the same
    /// reason they do in the tool-call miner — consulting a file and being
    /// responsible for it are different claims.
    pub write: bool,
}

/// What one script's worth of commands used, with the misses on the record.
#[derive(Debug, Default)]
pub struct Extract {
    pub files: Vec<FileUse>,
    /// Commands whose operation is not known, by name. Not an error — the
    /// honest size of what this does not yet read.
    pub unhandled: BTreeMap<String, usize>,
    /// Commands that were classified, whether or not they named a file.
    pub handled: usize,
    /// Reads and writes by the command that produced them.
    ///
    /// Diagnostic rather than mined: it answers "what is actually doing the
    /// editing", which is the question anyone asks before trusting this. The
    /// answer was not the expected one — `sed` is 10,414 invocations and 96.5%
    /// of them are `sed -n '1,40p' file`, a pager.
    pub by_command: BTreeMap<String, (usize, usize)>,
    /// Every operation, in running order, for callers that want more than the
    /// paths — what was searched for, what was renamed, which scripts ran.
    pub ops: Vec<Op>,
    /// Files used on another machine, by host. Reported, never mined into the
    /// local index — see [`RemoteUse`].
    pub remote: Vec<RemoteUse>,
    /// Nested scripts the grammar could not read. Reported rather than dropped:
    /// a devshell wrapper whose inner shell fails to parse is a silent hole in
    /// exactly the third of the corpus that runs through one.
    pub nested_unparsed: usize,
}

impl Extract {
    /// Record which command produced a use, for the diagnostic tally.
    fn note(&mut self, command: &str, write: bool) {
        let entry = self.by_command.entry(command.to_string()).or_default();
        if write {
            entry.1 += 1;
        } else {
            entry.0 += 1;
        }
    }

    /// Record a file use against this machine, or against `host` when the
    /// command is running somewhere else.
    fn push(&mut self, host: Option<&str>, command: &str, path: String, write: bool) {
        self.note(command, write);
        match host {
            Some(host) => self.remote.push(RemoteUse {
                host: host.to_string(),
                path,
                write,
            }),
            None => self.files.push(FileUse { path, write }),
        }
    }

    /// Fold a nested script's findings into this one.
    fn absorb(&mut self, inner: Extract) {
        self.files.extend(inner.files);
        self.remote.extend(inner.remote);
        self.ops.extend(inner.ops);
        self.handled += inner.handled;
        self.nested_unparsed += inner.nested_unparsed;
        for (name, n) in inner.unhandled {
            *self.unhandled.entry(name).or_insert(0) += n;
        }
        for (name, (r, w)) in inner.by_command {
            let entry = self.by_command.entry(name).or_default();
            entry.0 += r;
            entry.1 += w;
        }
    }
}

/// How deep a `bash -c 'bash -c "…"'` chain is followed.
///
/// The corpus nests twice at most — `nix develop -c bash -c '…'` — so this is a
/// backstop against pathological input rather than a limit anything real meets.
const MAX_NESTING: usize = 4;

/// The files an operation uses, and which way each goes.
///
/// The whole projection, in one place. Every direction here is a property of the
/// operation rather than of a lookup table: a `Move` writes where it lands and
/// reads where it came from, and neither fact needs stating twice.
pub fn files_of(op: &Op) -> Vec<FileUse> {
    let read = |paths: &Vec<String>| -> Vec<FileUse> {
        paths
            .iter()
            .map(|path| FileUse {
                path: path.clone(),
                write: false,
            })
            .collect()
    };
    let write = |paths: &Vec<String>| -> Vec<FileUse> {
        paths
            .iter()
            .map(|path| FileUse {
                path: path.clone(),
                write: true,
            })
            .collect()
    };
    match op {
        Op::Read { paths } | Op::Search { paths, .. } => read(paths),
        Op::Write { paths } | Op::Remove { paths, .. } => write(paths),
        Op::Copy { from, to } | Op::Move { from, to } => {
            let mut out = read(from);
            out.push(FileUse {
                path: to.clone(),
                write: true,
            });
            out
        }
        Op::Transform {
            program_file,
            paths,
            in_place,
            ..
        } => {
            // The program is read even when the operands are rewritten.
            let mut out: Vec<FileUse> = program_file
                .iter()
                .map(|path| FileUse {
                    path: path.clone(),
                    write: false,
                })
                .collect();
            out.extend(if *in_place { write(paths) } else { read(paths) });
            out
        }
        // A script's own files are collected when it is read, not here — this
        // is one command's operands, and a script is not one.
        Op::Nested { .. } | Op::Remote { .. } => Vec::new(),
        Op::Run { script } => vec![FileUse {
            path: script.clone(),
            write: false,
        }],
        // **Staging changes no file.** The edit already happened — through
        // `Edit`, `sed` or a redirect — and was counted where it occurred.
        // Counting it again here was 37% of every shell-derived write.
        Op::Git(GitOp::Stage { .. }) => Vec::new(),
        Op::Git(GitOp::Alter { paths }) => write(paths),
        Op::Git(GitOp::Inspect { paths }) => read(paths),
        Op::Git(GitOp::Other { .. }) | Op::ChangeDir { .. } | Op::Nothing | Op::Unknown { .. } => {
            Vec::new()
        }
    }
}

/// Every file the commands of one script used, resolved against `cwd`.
///
/// `cwd` is the directory the `Bash` call ran in — the transcripts record it on
/// every line, and it is the one piece of context a relative path cannot be read
/// without. `None` where it is unknown, in which case only absolute paths
/// survive.
pub fn extract(cmds: &[Simple], cwd: Option<&str>, home: &str) -> Extract {
    extract_nested(cmds, cwd, home, None, 0)
}

/// As [`extract`], tracking how deep inside `bash -c` this script sits and which
/// machine it is running on (`None` for this one).
fn extract_nested(
    cmds: &[Simple],
    cwd: Option<&str>,
    home: &str,
    host: Option<&str>,
    depth: usize,
) -> Extract {
    let mut out = Extract::default();
    // Working directory per subshell scope. A `cd` inside `( … )` writes an
    // entry only that scope and its children can see, so the script it returns
    // to is unaffected — which is what the shell does.
    let mut dirs: BTreeMap<Vec<usize>, Option<String>> = BTreeMap::new();
    dirs.insert(Vec::new(), cwd.map(str::to_string));

    for cmd in cmds {
        let here = current(&dirs, &cmd.scope);
        // A redirect names a file whatever the command is, so it counts even for
        // one that is not understood: `./gradlew build > /tmp/out` is a write.
        let carrier = cmd
            .argv
            .first()
            .map(|head| basename(head).to_string())
            .unwrap_or_else(|| "(redirect)".to_string());
        for redirect in &cmd.redirects {
            if looks_like_path(&redirect.target)
                && let Some(path) = resolve(&redirect.target, here.as_deref(), home)
            {
                out.push(host, &carrier, path, redirect.write);
            }
        }
        if cmd.argv.is_empty() {
            continue;
        }

        let op = classify(&cmd.argv, here.as_deref(), home);
        // The name to file this under is the real command's, not the wrapper's.
        let name = unwrap_command(&cmd.argv)
            .first()
            .map(|head| basename(head).to_string())
            .unwrap_or_default();

        match &op {
            // The one operation that changes what comes next rather than
            // producing a file: the scope's own directory moves, and no
            // enclosing one does.
            Op::ChangeDir { to } => {
                dirs.insert(cmd.scope.clone(), to.clone());
                out.handled += 1;
            }
            Op::Unknown { name } => {
                // `cd "$WORKDIR"` — unresolvable, so the directory becomes
                // *unknown*. Leaving it stale would resolve every later relative
                // path somewhere the command never ran.
                if name == "cd" {
                    dirs.insert(cmd.scope.clone(), None);
                }
                *out.unhandled.entry(name.clone()).or_insert(0) += 1;
            }
            // A shell inside a shell: `bash -c '…'`, `nix-shell --run '…'`.
            // Read with the *current* directory and its own scope, so a `cd`
            // inside stays inside — which is what the inner shell does.
            Op::Nested { script } => {
                out.handled += 1;
                match crate::shell::parse(script) {
                    Ok(inner) if depth < MAX_NESTING => {
                        let found = extract_nested(&inner, here.as_deref(), home, host, depth + 1);
                        out.absorb(found);
                    }
                    Ok(_) => {}
                    Err(_) => out.nested_unparsed += 1,
                }
            }
            // Another machine's shell. Read the same way, recorded elsewhere,
            // and with **no working directory**: this one's is meaningless
            // there, so only absolute paths survive unless the script `cd`s
            // first — which many of them do.
            Op::Remote {
                host: there,
                script,
            } => {
                out.handled += 1;
                match crate::shell::parse(script) {
                    Ok(inner) if depth < MAX_NESTING => {
                        let found = extract_nested(&inner, None, home, Some(there), depth + 1);
                        out.absorb(found);
                    }
                    Ok(_) => {}
                    Err(_) => out.nested_unparsed += 1,
                }
            }
            _ => {
                out.handled += 1;
                for used in files_of(&op) {
                    out.push(host, &name, used.path, used.write);
                }
            }
        }
        out.ops.push(op);
    }
    out
}

/// The working directory in force for a scope: its own if it has moved, else
/// the nearest enclosing one that has.
fn current(dirs: &BTreeMap<Vec<usize>, Option<String>>, scope: &[usize]) -> Option<String> {
    (0..=scope.len())
        .rev()
        .find_map(|n| dirs.get(&scope[..n].to_vec()))
        .cloned()
        .flatten()
}
