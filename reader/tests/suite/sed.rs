//! `sed -i`, followed where sed's regular expressions and Rust's agree.

use reader::predict::sed::{Invocation, apply, invocation};

fn argv(words: &[&str]) -> Vec<String> {
    words.iter().map(|w| w.to_string()).collect()
}

fn one(script: &str, text: &str) -> Result<String, String> {
    apply(&[script], false, text)
}

#[test]
fn the_three_spellings_of_in_place_are_read() {
    let gnu = invocation(&argv(&["sed", "-i", "s/a/b/", "f"])).unwrap();
    let bsd = invocation(&argv(&["sed", "-i", "", "s/a/b/", "f"])).unwrap();
    assert_eq!(gnu, bsd);
    assert_eq!(gnu.scripts, vec!["s/a/b/"]);
    assert_eq!(gnu.suffix, None);
    assert_eq!(gnu.operands, vec!["f"]);

    let backup = invocation(&argv(&["sed", "-i.bak", "s/a/b/", "f"])).unwrap();
    assert_eq!(backup.suffix.as_deref(), Some(".bak"));

    let joined = invocation(&argv(&[
        "sed", "-Ei", "", "-e", "s/a/b/", "-e", "s/c/d/", "f",
    ]))
    .unwrap();
    assert_eq!(
        joined,
        Invocation {
            scripts: vec!["s/a/b/".to_string(), "s/c/d/".to_string()],
            extended: true,
            suffix: None,
            operands: vec!["f".to_string()],
        }
    );
}

#[test]
fn flags_that_change_what_is_written_are_refused_by_name() {
    assert_eq!(
        invocation(&argv(&["sed", "-n", "-i", "s/a/b/p", "f"])),
        Err("-n".to_string())
    );
    assert_eq!(
        invocation(&argv(&["sed", "-i", ""])),
        Err("no script".to_string())
    );
}

#[test]
fn a_substitution_is_applied_per_line_first_match_or_all() {
    assert_eq!(one("s/a/b/", "aa\naa\n"), Ok("ba\nba\n".to_string()));
    assert_eq!(one("s/a/b/g", "aa\naa\n"), Ok("bb\nbb\n".to_string()));
    assert_eq!(one("s/a/b/2", "aaa\n"), Ok("aba\n".to_string()));
    assert_eq!(one("s/A/b/i", "aa\n"), Ok("ba\n".to_string()));
    assert_eq!(
        one("s/a/b/", "a"),
        Ok("b".to_string()),
        "a last line without a newline keeps none"
    );
}

#[test]
fn any_delimiter_and_the_escaped_delimiter_is_itself() {
    assert_eq!(
        one("s|/usr|/opt|", "/usr/bin\n"),
        Ok("/opt/bin\n".to_string())
    );
    assert_eq!(one("s#a\\#b#c#", "a#b\n"), Ok("c\n".to_string()));
    assert_eq!(
        one("s|a\\|b|c|", "a|b\n"),
        Ok("c\n".to_string()),
        "a bare | is a literal in basic syntax"
    );
    assert_eq!(
        apply(&["s|a\\|b|c|"], true, "a|b\n"),
        Ok("c\n".to_string()),
        "and in extended"
    );
}

#[test]
fn basic_syntax_swaps_operators_and_literals() {
    assert_eq!(one("s/(a)/x/", "(a)\n"), Ok("x\n".to_string()));
    assert_eq!(
        one("s/\\(a\\)\\(b\\)/\\2\\1/", "ab\n"),
        Ok("ba\n".to_string())
    );
    assert_eq!(one("s/a+/x/", "a+\n"), Ok("x\n".to_string()));
    assert_eq!(one("s/a\\+/x/", "aaa\n"), Ok("x\n".to_string()));
    assert_eq!(one("s/a\\{2\\}/x/", "aaa\n"), Ok("xa\n".to_string()));
    assert_eq!(one("s/a|b/x/", "a|b\n"), Ok("x\n".to_string()));
    assert_eq!(one("s/a\\&b/x/", "a&b\n"), Ok("x\n".to_string()));
    assert_eq!(
        apply(&["s/(a)(b)/\\2\\1/"], true, "ab\n"),
        Ok("ba\n".to_string())
    );
    assert_eq!(
        apply(&["s/\\(a\\)/x/"], true, "(a)\n"),
        Ok("x\n".to_string())
    );
}

#[test]
fn anchors_and_stars_where_basic_syntax_makes_them_literal() {
    assert_eq!(one("s/^a/x/", "aa\n"), Ok("xa\n".to_string()));
    assert_eq!(one("s/a$/x/", "aa\n"), Ok("ax\n".to_string()));
    assert_eq!(one("s/a^b/x/", "a^b\n"), Ok("x\n".to_string()));
    assert_eq!(one("s/a$b/x/", "a$b\n"), Ok("x\n".to_string()));
    assert_eq!(one("s/*a/x/", "*a\n"), Ok("x\n".to_string()));
    assert_eq!(one("s/\\.ts$/.js/", "a.ts\n"), Ok("a.js\n".to_string()));
}

#[test]
fn the_replacement_expands_the_match_and_groups() {
    assert_eq!(one("s/a/[&]/", "a\n"), Ok("[a]\n".to_string()));
    assert_eq!(one("s/a/\\&/", "a\n"), Ok("&\n".to_string()));
    assert_eq!(one("s/a/$x/", "a\n"), Ok("$x\n".to_string()));
    assert_eq!(one("s/a/b\\nc/", "a\n"), Ok("b\nc\n".to_string()));
    assert_eq!(one("s/a/\\//", "a\n"), Ok("/\n".to_string()));
    assert_eq!(one("s/a/\\1/", "a\n"), Err("group".to_string()));
    assert_eq!(one("s/a/\\Ux/", "a\n"), Err("case conversion".to_string()));
}

#[test]
fn bracket_expressions_are_characters_to_sed() {
    assert_eq!(one("s/[[:space:]]*$//", "a  \n"), Ok("a\n".to_string()));
    assert_eq!(one("s/[^a-c]/x/g", "abcd\n"), Ok("abcx\n".to_string()));
    assert_eq!(one("s/[]a]/x/g", "]a\n"), Ok("xx\n".to_string()));
    assert_eq!(one("s/[\\n]/x/", "a\n"), Err("bracket escape".to_string()));
}

#[test]
fn addresses_select_lines_and_ranges() {
    let text = "a\na\na\na\n";
    assert_eq!(one("2s/a/b/", text), Ok("a\nb\na\na\n".to_string()));
    assert_eq!(one("$s/a/b/", text), Ok("a\na\na\nb\n".to_string()));
    assert_eq!(one("2,3s/a/b/", text), Ok("a\nb\nb\na\n".to_string()));
    assert_eq!(
        one("3,2s/a/b/", text),
        Ok("a\na\nb\na\n".to_string()),
        "an end behind the start is one line"
    );
    assert_eq!(one("/^x/s/a/b/", "xa\na\n"), Ok("xb\na\n".to_string()));
    assert_eq!(
        one("/begin/,/stop/s/a/b/", "a\nbegin a\na\nstop a\na\n"),
        Ok("a\nbegin b\nb\nstop b\na\n".to_string())
    );
    assert_eq!(one("0,/a/s/a/b/", text), Err("address 0".to_string()));
    assert_eq!(one("2!s/a/b/", text), Err("!".to_string()));
}

#[test]
fn several_commands_run_in_order_on_each_line() {
    assert_eq!(one("s/a/b/; s/b/c/", "a\n"), Ok("c\n".to_string()));
    assert_eq!(one("s/a/b/\ns/b/c/", "a\n"), Ok("c\n".to_string()));
    assert_eq!(
        apply(&["s/a/b/", "s/b/c/"], false, "a\n"),
        Ok("c\n".to_string())
    );
    assert_eq!(one("# note\ns/a/b/", "a\n"), Ok("b\n".to_string()));
}

#[test]
fn what_is_not_followed_is_refused_by_name() {
    assert_eq!(one("2d", "a\n"), Err("d".to_string()));
    assert_eq!(one("1a text", "a\n"), Err("a".to_string()));
    assert_eq!(one("s/a/b/w out", "a\n"), Err("s flag w".to_string()));
    assert_eq!(one("s/a/b/p", "a\n"), Err("s flag p".to_string()));
    assert_eq!(one("s//b/", "a\n"), Err("empty pattern".to_string()));
    assert_eq!(one("s/a*/b/g", "a\n"), Err("empty match".to_string()));
    assert_eq!(one("s/a*/b/", "a\n"), Ok("b\n".to_string()));
    assert_eq!(one("s/a\\|b/x/", "a\n"), Err("alternation".to_string()));
    assert_eq!(
        apply(&["s/a|b/x/"], true, "a\n"),
        Err("alternation".to_string())
    );
    assert_eq!(
        one("s/\\(a\\)\\1/b/", "aa\n"),
        Err("backreference".to_string())
    );
    assert_eq!(one("s/\\<a/b/", "a\n"), Err("word edge".to_string()));
    assert_eq!(one("s/a/b", "a\n"), Err("unterminated".to_string()));
    assert_eq!(one("y/a/b/", "a\n"), Err("y".to_string()));
}
