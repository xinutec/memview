//! What Claude was *doing*, one level above what it typed.
//!
//! [`crate::shell_ops`] answers "what does this command do to files", which is
//! the right question for attribution and the wrong one for understanding: it
//! reads `cargo test` and `nix build` and `kubectl rollout` as
//! [`Op::Nothing`](crate::shell_ops::Op::Nothing), because none of them names a
//! file. Two thirds of what a session spends its time on is invisible at that
//! level, and it is the two thirds a person would name first if you asked what
//! the session had been doing.
//!
//! So this is a second, coarser reading of the same commands — **not a
//! replacement**. The file dimensions stay exactly as they are; this says what
//! kind of work the command was part of. It is deliberately lossy: an
//! [`Activity`] cannot be turned back into the command it came from, and is not
//! meant to be.
//!
//! Built by the same method as everything else here: the set below is what the
//! corpus actually contains, ranked by `activity-report`, and the tail it
//! cannot name is counted rather than rounded away.

use crate::shell::Simple;
use crate::shell_ops::{GitOp, Op, basename, unwrap_command};

/// What one command was doing, in the vocabulary a person would use.
///
/// The closed set. A command that fits none of it is [`Activity::Other`],
/// carrying its name so the gap is a number rather than a shrug.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Activity {
    /// Changed a file — by any means: `Edit`, `sed -i`, a redirect, a Python
    /// heredoc, `cp` onto a destination.
    Edit,
    /// Read one: `cat`, `sed -n`, `head`, the `Read` tool.
    Inspect,
    /// Looked for something. The pattern is the point, and the reason this is
    /// not folded into `Inspect`.
    Search,
    /// Ran a test suite.
    Test,
    /// Compiled or bundled something.
    Build,
    /// Ran a linter, formatter or type checker over the tree.
    Check,
    /// Ran a script or a program of the fleet's own.
    Run,
    /// Asked git something, or told it something.
    Vcs,
    /// Put something somewhere it runs: `kubectl apply`, `nixos-rebuild`,
    /// `home-manager switch`, a sync script.
    Deploy,
    /// Looked at a machine — its logs, its services, its network.
    Observe,
    /// Installed or updated dependencies and toolchains.
    Install,
    /// Asked a database.
    Query,
    /// Moved around, listed a directory, printed something. Understood, and not
    /// work anybody would name.
    Navigate,
    /// Understood, and not work: a loop keyword, a `[` test, a `then`. The
    /// grammar leaves these as ordinary commands on purpose, and counting them
    /// as activities would drown the vocabulary in syntax.
    Nothing,
    /// Not in the table, carrying the command's name.
    Other { name: String },
}

/// Where the work happened, when it was not this machine.
///
/// A property of the activity rather than a kind of activity: deploying to isis
/// and reading isis's logs are different work on the same host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Where {
    pub host: String,
}

/// The activity a command performs, from its typed operation and its name.
///
/// Both are needed and neither is enough. The operation knows that `sed -i`
/// changes a file and `cargo test` does not; the name knows that `cargo test`
/// is a test run and `cargo build` is not.
pub fn of(op: &Op, cmd: &Simple) -> Activity {
    // A redirect changes a file whatever the command is, and it is how a third
    // of the corpus's writing is done: `echo … > f`, `cat <<EOF > f`, and the
    // 8,386 commands that are a redirect and nothing else.
    if cmd.redirects.iter().any(|r| r.write) {
        return Activity::Edit;
    }
    let argv = &cmd.argv;
    // The operation decides first where it can: a command that demonstrably
    // changed a file was editing, whatever it is called.
    match op {
        Op::Write { paths } if !paths.is_empty() => return Activity::Edit,
        // A deletion is a change whether or not the path could be resolved:
        // `rm -rf "$BUILD"` names nothing and still removed something.
        Op::Remove { .. } => return Activity::Edit,
        Op::Copy { .. } | Op::Move { .. } => return Activity::Edit,
        Op::Transform { in_place: true, .. } => return Activity::Edit,
        Op::Git(GitOp::Alter { .. }) => return Activity::Edit,
        Op::Search { .. } => return Activity::Search,
        _ => {}
    }
    let argv = unwrap_command(carried(argv));
    // Stripping the wrappers can leave nothing: `for f in *; do` is a keyword
    // whose command is on the next line, and `nix develop -c` with no argument
    // is a devshell nobody asked to run anything in. Syntax, not work.
    let Some(head) = argv.first() else {
        return Activity::Nothing;
    };
    let name = basename(head);
    let sub = argv.get(1).map(String::as_str).unwrap_or_default();
    if let Some(activity) = named(name, sub, argv) {
        return activity;
    }
    // Nothing in the name; fall back to what the operation did.
    match op {
        Op::Read { paths } if !paths.is_empty() => Activity::Inspect,
        Op::Transform { .. } => Activity::Inspect,
        Op::Run { .. } => Activity::Run,
        Op::Python { .. } | Op::Nested { .. } => Activity::Run,
        Op::Git(_) => Activity::Vcs,
        Op::ChangeDir { .. } => Activity::Navigate,
        Op::Remote { .. } => Activity::Observe,
        Op::Unknown { name } => Activity::Other { name: name.clone() },
        _ => Activity::Other {
            name: name.to_string(),
        },
    }
}

/// The command a devshell wrapper was asked to run, which is the work.
///
/// `nix develop -c cargo test` is a test run, not a build of nix. The file
/// layer already looks through the wrapper — `Verb::Carries` classifies the
/// rest of the line in place — but it leaves the outer argv behind, and asked
/// of *that*, every devshell invocation in the corpus reads as `nix develop`.
fn carried(argv: &[String]) -> &[String] {
    let name = argv.first().map(|head| basename(head)).unwrap_or_default();
    if !matches!(name, "nix" | "nix-shell" | "nix-build") {
        return argv;
    }
    match argv
        .iter()
        .position(|word| word == "-c" || word == "--command")
    {
        Some(at) if at + 1 < argv.len() => &argv[at + 1..],
        _ => argv,
    }
}

/// The activity a command's *name* implies, where its file operation cannot say.
///
/// This is where the vocabulary earns its keep: `cargo test`, `cargo build` and
/// `cargo add` are one command to the file layer — none of them names a file —
/// and three different kinds of work to anybody reading a session.
fn named(name: &str, sub: &str, argv: &[String]) -> Option<Activity> {
    // A build tool running a test task is a test run, and the task is not the
    // subcommand: `./gradlew :app:testDebugUnitTest`, `cargo test -p geo`.
    // Asked of the name alone, every gradle invocation reads as a build.
    let tests = || {
        argv.iter()
            .skip(1)
            .any(|word| word.contains("test") || word.contains("Test") || word.contains("spec"))
    };
    // Subcommand first: one program, several kinds of work.
    match (name, sub) {
        ("cargo" | "npm" | "pnpm" | "yarn" | "bun", "test") => return Some(Activity::Test),
        ("cargo", "clippy" | "fmt") => return Some(Activity::Check),
        ("cargo" | "npm" | "pnpm" | "yarn", "build") => return Some(Activity::Build),
        ("cargo" | "npm" | "pnpm" | "yarn" | "bun", "add" | "install" | "update" | "i") => {
            return Some(Activity::Install);
        }
        ("cargo" | "npm" | "pnpm", "run") => return Some(Activity::Run),
        ("nix", "build" | "develop") => return Some(Activity::Build),
        ("nix", "flake") => return Some(Activity::Install),
        ("nix", "run") => return Some(Activity::Run),
        ("git", _) => return Some(Activity::Vcs),
        ("kubectl", "apply" | "rollout" | "delete" | "create" | "patch" | "scale") => {
            return Some(Activity::Deploy);
        }
        ("docker" | "podman", "push" | "build") => return Some(Activity::Deploy),
        ("kubectl" | "docker" | "podman", _) => return Some(Activity::Observe),
        ("go", "test") => return Some(Activity::Test),
        ("go", "build") => return Some(Activity::Build),
        // Any other subcommand of a build tool is still build work: `cargo
        // check`, `cargo doc`, `npm ls`. Naming each one would be a list that
        // grows forever and says nothing new.
        ("gradlew" | "gradle", _) if tests() => return Some(Activity::Test),
        ("cargo" | "npm" | "pnpm" | "yarn" | "go" | "gradlew" | "gradle", _) => {
            return Some(Activity::Build);
        }
        ("adb", "install" | "uninstall" | "push") => return Some(Activity::Deploy),
        // The GitHub CLI is version control when it changes something and a
        // look at CI when it does not.
        ("gh", "run" | "api" | "status") => return Some(Activity::Observe),
        ("gh", _) => return Some(Activity::Vcs),
        _ => {}
    }
    Some(match name {
        "pytest" | "vitest" | "jest" | "playwright" | "ctest" | "nextest" => Activity::Test,
        "tsc" | "ng" | "cmake" | "make" | "javac" | "kotlinc" | "swift" | "rustc" | "lake"
        | "esbuild" | "vite" | "webpack" | "nix-build" => Activity::Build,
        "clippy" | "ruff" | "mypy" | "pyright" | "eslint" | "biome" | "prettier" | "ktlint"
        | "stylelint" | "shellcheck" | "black" | "isort" | "clang-format" | "ast-grep" => {
            Activity::Check
        }
        "kubectl" | "helm" | "nixos-rebuild" | "home-manager" | "flyctl" | "tofu" | "terraform"
        | "k3s" | "k3d" | "flux" | "argocd" => Activity::Deploy,
        "journalctl" | "systemctl" | "dmesg" | "ping" | "dig" | "nc" | "wg" | "lsof" | "ps"
        | "top" | "df" | "uptime" | "system_profiler" | "launchctl" | "printenv" | "env"
        | "nixos-version" | "mount" | "screen" | "curl" | "wget" | "adb" | "pgrep" | "pkill"
        | "killall" | "kill" | "netstat" | "ifconfig" | "traceroute" | "host" | "whois"
        | "iostat" | "vm_stat" | "sw_vers" | "diskutil" | "networksetup" | "ssh" | "scp"
        | "rsync" | "sftp" | "restic" | "borg" | "rclone" | "probe" => Activity::Observe,
        "pip" | "pip3" | "uv" | "poetry" | "brew" | "nix-env" | "nix-channel" | "direnv"
        | "npx" | "corepack" => Activity::Install,
        "mariadb" | "mysql" | "psql" | "sqlite3" | "redis-cli" | "mongosh" => Activity::Query,
        // File commands whose operands could not be resolved — a `$VAR`
        // destination, a glob that named nothing. The change happened anyway.
        "cp" | "mv" | "ln" | "shred" | "truncate" => Activity::Edit,
        // An interpreter or a wrapper with no better name: it ran something.
        "node" | "python" | "python3" | "bash" | "sh" | "zsh" | "deno" | "bun" | "ruby"
        | "perl" | "nix" | "nix-shell" | "ffmpeg" | "xcrun" | "osascript" | "xargs" | "tar"
        | "unzip" | "zip" | "gzip" | "openssl" | "tsx" | "resvg" | "convert" | "sips" | "qpdf"
        | "pandoc" => Activity::Run,
        "cd" | "ls" | "pwd" | "echo" | "printf" | "mkdir" | "which" | "whoami" | "date"
        | "sleep" | "tree" | "clear" | "export" | "source" | "open" | "hostname" | "uname"
        | "id" | "basename" | "dirname" | "realpath" | "readlink" | "touch" | "chmod" => {
            Activity::Navigate
        }
        // Readers, whether or not they were given a file: `… | head -3` reads a
        // stream, and it is the commonest command in the corpus after `cd`.
        "cat" | "head" | "tail" | "less" | "more" | "wc" | "sort" | "uniq" | "cut" | "tr"
        | "nl" | "column" | "sed" | "awk" | "jq" | "yq" | "diff" | "od" | "xxd" | "strings"
        | "file" | "stat" | "du" | "base64" | "rev" | "paste" | "comm" | "tee" | "bat" => {
            Activity::Inspect
        }
        "grep" | "rg" | "egrep" | "fgrep" | "ag" | "ack" | "find" | "fd" | "locate" => {
            Activity::Search
        }
        // Shell syntax the grammar leaves as ordinary words, so that `echo done`
        // cannot end a loop. None of it is work.
        "for" | "do" | "done" | "then" | "else" | "elif" | "if" | "fi" | "while" | "until"
        | "case" | "esac" | "in" | "break" | "continue" | "return" | "exit" | "seq" | "true"
        | "false" | ":" | "[" | "[[" | "test" | "eval" | "set" | "unset" | "read" | "wait"
        | "trap" | "shift" | "local" | "function" | "yes" | "sync" | "type" | "hash" | "disown"
        | "jobs" | "su" | "sudo" | "exec" | "command" | "builtin" | "alias" => Activity::Nothing,
        _ => return None,
    })
}

impl Activity {
    /// Whether this is work anybody would put on a timeline.
    ///
    /// `cd`, `ls`, `echo` and the loop keywords are two fifths of every command
    /// in the corpus and none of them is a thing a session *did*. Kept in the
    /// vocabulary — refusing to name them would leave them in the worklist
    /// forever — and left out of the record.
    pub fn is_work(&self) -> bool {
        !matches!(self, Activity::Navigate | Activity::Nothing)
    }

    /// A stable name, for tallies and for the wire.
    pub fn label(&self) -> &str {
        match self {
            Activity::Edit => "edit",
            Activity::Inspect => "inspect",
            Activity::Search => "search",
            Activity::Test => "test",
            Activity::Build => "build",
            Activity::Check => "check",
            Activity::Run => "run",
            Activity::Vcs => "version control",
            Activity::Deploy => "deploy",
            Activity::Observe => "observe",
            Activity::Install => "install",
            Activity::Query => "query",
            Activity::Navigate => "navigate",
            Activity::Nothing => "(not work)",
            Activity::Other { name } => name,
        }
    }
}
