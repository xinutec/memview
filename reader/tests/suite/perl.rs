//! `perl -pi -e`, followed where Perl's regular expressions and Rust's agree.
//!
//! Every expected text here is what `/usr/bin/perl` 5.34 left in a file when
//! given the same code over the same text, on 2026-10-05; none was written by
//! hand. The refusals are the constructs the module says it has no counterpart
//! for, each one with the perl behaviour that makes it a refusal rather than a
//! translation.

use reader::predict::perl::{Invocation, apply, invocation};

fn argv(words: &[&str]) -> Vec<String> {
    words.iter().map(|w| w.to_string()).collect()
}

fn line(code: &str, text: &str) -> Result<String, String> {
    apply(&[code], false, text)
}

fn slurp(code: &str, text: &str) -> Result<String, String> {
    apply(&[code], true, text)
}

#[test]
fn the_clusters_the_corpus_writes_are_read() {
    let pi = invocation(&argv(&["perl", "-pi", "-e", "s/a/b/", "f"])).unwrap();
    assert_eq!(
        pi,
        Invocation {
            code: vec!["s/a/b/".to_string()],
            slurp: false,
            suffix: None,
            operands: vec!["f".to_string()],
        }
    );
    let zero = invocation(&argv(&["perl", "-0pi", "-e", "s/a/b/", "f", "g"])).unwrap();
    assert!(zero.slurp);
    assert_eq!(zero.operands, vec!["f", "g"]);
    let ipe = invocation(&argv(&["perl", "-i", "-pe", "s/a/b/", "f"])).unwrap();
    assert_eq!(ipe.code, vec!["s/a/b/"]);
    let backup = invocation(&argv(&["perl", "-pi.bak", "-e", "s/a/b/", "f"])).unwrap();
    assert_eq!(backup.suffix.as_deref(), Some(".bak"));
    let slurped = invocation(&argv(&["perl", "-0777", "-pi", "-e", "s/a/b/", "f"])).unwrap();
    assert!(slurped.slurp);
    let two = invocation(&argv(&["perl", "-pi", "-e", "s/a/b/", "-e", "s/c/d/", "f"])).unwrap();
    assert_eq!(two.code, vec!["s/a/b/", "s/c/d/"]);
}

#[test]
fn flags_that_change_what_is_written_are_refused_by_name() {
    assert_eq!(
        invocation(&argv(&["perl", "-ni", "-e", "print unless /x/", "f"])),
        Err("-n".to_string())
    );
    assert_eq!(
        invocation(&argv(&["perl", "-pi", "-l", "-e", "s/a/b/", "f"])),
        Err("-l".to_string())
    );
    assert_eq!(
        invocation(&argv(&["perl", "-pe", "s/a/b/", "f"])),
        Err("no -i".to_string())
    );
    assert_eq!(
        invocation(&argv(&["perl", "-i", "-e", "s/a/b/", "f"])),
        Err("no -p".to_string())
    );
}

#[test]
fn a_substitution_on_each_line() {
    assert_eq!(line("s/a/b/", "a a\nca\n").unwrap(), "b a\ncb\n");
    assert_eq!(line("s/a/b/g", "a a\nca\n").unwrap(), "b b\ncb\n");
    assert_eq!(line("s/a/b/i", "A\n").unwrap(), "b\n");
    assert_eq!(line("s|/usr|/opt|", "/usr/bin\n").unwrap(), "/opt/bin\n");
    assert_eq!(line("s/\\./!/g", "a.b.c\n").unwrap(), "a!b!c\n");
    assert_eq!(line("s/a/b/; s/c/d/", "ac\n").unwrap(), "bd\n");
    assert_eq!(line("s/a/b/;s/c/d/;", "ac\n").unwrap(), "bd\n");
    assert_eq!(apply(&["s/a/b/", "s/c/d/"], false, "ac\n").unwrap(), "bd\n");
    assert_eq!(
        line("s/\\bfoo\\b/bar/g", "foo foobar foo\n").unwrap(),
        "bar foobar bar\n"
    );
}

#[test]
fn groups_in_the_replacement() {
    assert_eq!(
        line("s/(\\w+) (\\w+)/$2 $1/", "hello world\n").unwrap(),
        "world hello\n"
    );
    // A group that took no part is the empty string.
    assert_eq!(line("s/(a)(b)?/[$1$2]/", "a ab\n").unwrap(), "[a] ab\n");
    assert_eq!(line("s/(a)/[${1}]/", "a\n").unwrap(), "[a]\n");
    assert_eq!(line("s/(a)/[\\1]/", "a\n").unwrap(), "[a]\n");
    assert_eq!(line("s/a+/<$&>/", "xaay\n").unwrap(), "x<aa>y\n");
    // The corpus's commonest shape: an escaped `@` and `/` in the pattern,
    // a group kept in the replacement.
    assert_eq!(
        line(
            "s/(pin \"\\@angular\\/[a-z-]+\" )\"\\^22\\.1\\.[78]\"/$1\"^22.2.0\"/",
            "pin \"@angular/core\" \"^22.1.7\"\npin \"@angular/cli\" \"^22.1.9\"\n"
        )
        .unwrap(),
        "pin \"@angular/core\" \"^22.2.0\"\npin \"@angular/cli\" \"^22.1.9\"\n"
    );
}

#[test]
fn escapes_in_the_replacement() {
    assert_eq!(line("s/a/\\n/", "xay\n").unwrap(), "x\ny\n");
    assert_eq!(line("s/a/\\t\\\\\\$x/", "xay\n").unwrap(), "x\t\\$xy\n");
    assert_eq!(line("s/a/\\/\\@/", "a\n").unwrap(), "/@\n");
}

#[test]
fn anchors_see_the_record_without_its_final_newline() {
    assert_eq!(line("s/x$/y/", "x\nx").unwrap(), "y\ny");
    assert_eq!(line("s/^/> /", "a\nb\n").unwrap(), "> a\n> b\n");
    assert_eq!(line("s/$/!/", "a\nb").unwrap(), "a!\nb!");
    assert_eq!(line("s/^/>/mg", "a\n").unwrap(), ">a\n");
    assert_eq!(slurp("s/y$/z/", "x\ny\n").unwrap(), "x\nz\n");
    assert_eq!(slurp("s/^/> /", "a\nb\n").unwrap(), "> a\nb\n");
    assert_eq!(slurp("s/^a/b/mg", "a\na\n").unwrap(), "b\nb\n");
    assert_eq!(slurp("s/^/>/mg", "a\nb\n").unwrap(), ">a\n>b\n");
    assert_eq!(slurp("s/a$/Z/", "a\na\n").unwrap(), "a\nZ\n");
    assert_eq!(slurp("s/a$/Z/m", "a\na\n").unwrap(), "Z\na\n");
}

#[test]
fn a_pattern_that_can_match_a_newline_sees_the_whole_record() {
    // `\s+$` eats the newline: perl leaves `ab`. Under `-p` the record is
    // one line and the two engines agree on it; under `-0` the bare `$` is
    // refused before a final newline.
    assert_eq!(line("s/\\s+$//", "a  \nb\t\n").unwrap(), "ab");
    assert_eq!(
        slurp("s/\\s+$//", "a  \nb\t\n"),
        Err("$ before a final newline".to_string())
    );
    assert_eq!(slurp("s/\\s+$//", "a  \nb\t").unwrap(), "a  \nb");
    assert_eq!(
        slurp("s/a\\n  b/c/", "x\na\n  b\ny\n").unwrap(),
        "x\nc\ny\n"
    );
    assert_eq!(
        slurp("s/foo/bar/g", "foo\nfoo foo\n").unwrap(),
        "bar\nbar bar\n"
    );
    assert_eq!(slurp("s/a.b/X/", "a\nb\n").unwrap(), "a\nb\n");
    assert_eq!(slurp("s/a.b/X/s", "a\nb\n").unwrap(), "X\n");
    // Where the whole record is the haystack a bare `$` is refused: the two
    // engines place it differently before the final newline.
    assert_eq!(
        slurp("s/\\s$//", "a \n"),
        Err("$ before a final newline".to_string())
    );
    assert_eq!(
        slurp("s/^\\n//mg", "\na\n"),
        Err("^ after a final newline".to_string())
    );
}

#[test]
fn an_escaped_delimiter_is_the_bare_delimiter() {
    // `s|a\|b|X|` is an alternation to perl: it leaves `X|b`.
    assert_eq!(line("s|a\\|b|X|", "a|b\n").unwrap(), "X|b\n");
    assert_eq!(line("s/a\\/b/X/", "a/b\n").unwrap(), "X\n");
    assert_eq!(line("s#a\\#b#X#", "a#b\n").unwrap(), "X\n");
}

#[test]
fn what_rust_has_no_counterpart_for_is_refused_by_name() {
    assert_eq!(
        line("s/(a)\\1/b/", "aa\n"),
        Err("backreference".to_string())
    );
    assert_eq!(line("s/a(?=b)/c/", "ab\n"), Err("lookaround".to_string()));
    assert_eq!(line("s/a(?<!b)/c/", "ab\n"), Err("lookaround".to_string()));
    assert_eq!(
        line("s/a++/c/", "aa\n"),
        Err("possessive quantifier".to_string())
    );
    assert_eq!(line("s/a\\Z/c/", "a\n"), Err("\\Z".to_string()));
    assert_eq!(
        line("s/\\Qa.b\\E/c/", "a.b\n"),
        Err("quotemeta".to_string())
    );
    assert_eq!(line("s/a/\\Ub/", "a\n"), Err("case conversion".to_string()));
    assert_eq!(line("s/a/$x/", "a\n"), Err("variable".to_string()));
    assert_eq!(line("s/a/$ENV{HOME}/", "a\n"), Err("variable".to_string()));
    assert_eq!(line("s/a/@x/", "a\n"), Err("array".to_string()));
    assert_eq!(line("s/${1}//", "a\n"), Err("variable".to_string()));
    assert_eq!(line("s/a/b/e", "a\n"), Err("flag e".to_string()));
    assert_eq!(line("s/a/b/x", "a\n"), Err("flag x".to_string()));
    assert_eq!(line("s{a}{b}", "a\n"), Err("bracket delimiter".to_string()));
    assert_eq!(
        line("print unless /a/", "a\n"),
        Err("statement".to_string())
    );
    assert_eq!(
        line("if(!$d && s/a/b/){$d=1}", "a\n"),
        Err("statement".to_string())
    );
    assert_eq!(line("s/a/b", "a\n"), Err("unterminated".to_string()));
    assert_eq!(line("s/(a/b/", "a\n"), Err("pattern".to_string()));
}

#[test]
fn an_empty_match_under_g_is_refused() {
    // perl: `-a-a-a-\n-` for `s/x*/-/g` on `aaa`, and `-a--c-\n-` for `s/b*/-/g`
    // on `abc` — the empty matches around the newline are its own.
    assert_eq!(line("s/x*/-/g", "aaa\n"), Err("empty match".to_string()));
    assert_eq!(line("s/b*/-/g", "abc\n"), Err("empty match".to_string()));
    assert_eq!(line("s/$/!/g", "a\n"), Err("empty match".to_string()));
    // Without `g` the first match is where both engines find it.
    assert_eq!(line("s/x*/-/", "aaa\n").unwrap(), "-aaa\n");
}

#[test]
fn perl_reads_bytes_so_non_ascii_text_is_refused_under_what_reads_characters() {
    // perl: `s/./x/g` on `aé` gives THREE x — one per byte.
    assert_eq!(line("s/./x/g", "aé\n"), Err("non-ASCII text".to_string()));
    assert_eq!(line("s/\\w+/x/", "é\n"), Err("non-ASCII text".to_string()));
    assert_eq!(line("s/[a-z]/x/", "é\n"), Err("non-ASCII text".to_string()));
    assert_eq!(line("s/e/x/i", "É\n"), Err("non-ASCII text".to_string()));
    assert_eq!(
        line("s/é+/x/", "é\n"),
        Err("quantified non-ASCII literal".to_string())
    );
    // A literal is the same bytes to both: perl leaves `cafe e`.
    assert_eq!(line("s/é/e/g", "café é\n").unwrap(), "cafe e\n");
    assert_eq!(line("s/./x/g", "ab\n").unwrap(), "xx\n");
}
