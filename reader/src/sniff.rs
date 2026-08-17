//! What language a heredoc body is written in, guessed from a mark.
//!
//! ⚠ **A guess, and the report that carries it says so.** The value is the
//! ranking — is there a kilobyte of SQL here or a megabyte — never the label on
//! any one body. It lives in the library rather than in `bin/opacity.rs`
//! because a guess that decides what to build next is worth a test suite, and
//! [`looks_like`] returns the mark that fired so a wrong bucket can be opened
//! and read rather than argued about.

/// What a heredoc body looks like, **and the mark that decided it**.
///
/// ⚠ **Anchored, and rewritten once because the first version was mostly wrong.**
/// It tested for marks *anywhere* in the body: `contains("SELECT ")` filed Python
/// and shell under SQL because prose says "select", `starts_with('[')` filed an
/// INI file under JSON, and `contains("\n#")` filed Rust under shell on the
/// strength of `#[test]`. A sniff that can fire on a substring of a comment is
/// not a sniff. Every test below is anchored to the start of the body or to the
/// start of a line, and JSON is not sniffed at all — it is parsed.
///
/// ⚠ **The mark is returned because the first rewrite was still wrong and
/// nothing said so.** A bucket labelled "Python" holding TypeScript reads as a
/// megabyte of Python left unread, and that is what ranks the next reader — the
/// census is the instrument, so an unauditable guess inside it is worse than a
/// wrong number, because the number looks the same either way. `--why <label>`
/// prints the bodies under a label with the mark each one fired, which is how a
/// guess becomes checkable.
///
/// Order is by how distinctive the mark is, not by how common the language is.
/// The Python test carries a second shape because the corpus's commonest Python
/// heredoc does not open with an import: it opens with `p='some/path'` and goes
/// straight to `open(p).read()`.
pub fn looks_like(body: &str) -> (&'static str, &'static str) {
    let head = body.trim_start();
    let line_starts = |mark: &str| head.starts_with(mark) || body.contains(&format!("\n{mark}"));
    // The remainder of every line that opens with `mark`. ⚠ **`import ` is not a
    // language**: Python, TypeScript, Kotlin and Swift all open a line with it,
    // so the mark that decides has to be what comes AFTER it.
    let after = |mark: &'static str| {
        body.lines()
            .filter_map(move |line| line.trim_start().strip_prefix(mark))
    };
    // A module named in quotes or destructured in braces is ES syntax; Python
    // and Swift name a module bare, and Kotlin names it dotted.
    let es_import = || {
        after("import ")
            .any(|rest| rest.starts_with('{') || rest.starts_with('*') || rest.contains(" from "))
    };
    if head.starts_with("#!") {
        ("a script with a shebang", "#!")
    } else if serde_json::from_str::<serde_json::Value>(head).is_ok() {
        ("JSON", "parses as JSON")
    } else if starts_with_word(
        head,
        &[
            "SELECT", "CREATE", "INSERT", "UPDATE", "DELETE", "PRAGMA", "ATTACH", "BEGIN", "WITH",
        ],
    ) {
        ("SQL", "opens with a SQL verb")
    // Before Python, because Lean writes `def` too and this corpus is full of
    // it: the health port is Lean, and every one of its bodies was filed as
    // Python by that one keyword.
    } else if line_starts("theorem ") || line_starts("inductive ") || line_starts("@[") {
        ("Lean", "a line opens `theorem `, `inductive ` or `@[`")
    } else if line_starts("structure ") && body.contains(" where") {
        ("Lean", "a line opens `structure `, and holds ` where`")
    } else if after("def ").any(|rest| rest.contains(" := ") || rest.contains(" : ")) {
        ("Lean", "a `def ` line holds ` := ` or ` : `")
    } else if es_import() || line_starts("export ") || line_starts("const ") {
        (
            "TypeScript or JavaScript",
            "an ES import, or a line opens `export `/`const `",
        )
    // Before Python for the same reason as Lean: these open with `package` but
    // their second line is an `import`, and `import` was reached first.
    } else if line_starts("package ") || body.contains("kotlinx") || line_starts("fun ") {
        ("Kotlin or Java", "`package `, `kotlinx` or `fun `")
    } else if after("import ").any(|rest| APPLE_FRAMEWORKS.contains(&rest.trim_end())) {
        ("Swift", "imports an Apple framework")
    } else if after("import ").any(|rest| !rest.is_empty()) {
        ("Python", "a bare `import `")
    // ⚠ **`from ` alone is English.** It filed task bodies and design notes as
    // Python on the strength of a sentence that wrapped onto a line beginning
    // "from". Python's shape is `from X import Y`, and the `import` is the half
    // that carries the signal.
    } else if after("from ").any(|rest| rest.contains(" import ")) {
        ("Python", "a line opens `from … import `")
    } else if line_starts("def ") {
        ("Python", "a line opens `def `")
    } else if body.contains("open(") && body.contains(".read()") {
        ("Python", "`open(` and `.read()`")
    } else if body.contains(".read_text()") {
        ("Python", "`.read_text()`")
    } else if body.contains(".write_text(") {
        ("Python", "`.write_text(`")
    } else if line_starts("#[") || line_starts("fn ") || line_starts("pub fn ") {
        ("Rust", "a line opens `#[`, `fn ` or `pub fn `")
    } else if head.starts_with("let ") && body.contains(" in ") {
        ("Dhall", "opens `let `, and holds ` in `")
    } else if head.starts_with("---") || line_starts("apiVersion:") {
        ("YAML", "opens `---`, or a line opens `apiVersion:`")
    } else if head.starts_with('[') && body.contains('=') {
        ("INI", "opens `[`, and holds `=`")
    } else if head.starts_with('<') {
        ("markup", "opens `<`")
    } else if head.starts_with("set -e") || line_starts("for ") || line_starts("if [") {
        ("shell", "`set -e`, `for ` or `if [`")
    } else {
        ("prose, or nothing recognised", "nothing matched")
    }
}

/// Frameworks that are imported bare, exactly as Python imports a module, and
/// belong to no Python anybody writes. The list is what the corpus imports, not
/// the whole SDK — a name is added when a body turns up carrying it.
const APPLE_FRAMEWORKS: [&str; 8] = [
    "Foundation",
    "CoreAudio",
    "AVFoundation",
    "AppKit",
    "UIKit",
    "SwiftUI",
    "CoreGraphics",
    "Cocoa",
];
/// Whether the text opens with one of these words, as a word.
///
/// `starts_with("WITH")` would match `WITHOUT`, and case is not a signal here —
/// the corpus writes SQL both ways.
fn starts_with_word(head: &str, words: &[&str]) -> bool {
    let first: String = head
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect::<String>()
        .to_uppercase();
    words.contains(&first.as_str())
}
