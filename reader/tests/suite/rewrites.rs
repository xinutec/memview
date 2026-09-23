//! What a Python program replaced in the files it wrote back.

use reader::python::{Replaced, Rewrite, rewrites};

fn pair(old: &str, new: &str) -> Replaced {
    Replaced {
        old: old.to_string(),
        new: new.to_string(),
    }
}

#[test]
fn a_file_read_replaced_and_written_back_is_an_edit() {
    let source =
        "p = Path('src/a.rs')\ns = p.read_text()\ns = s.replace('one', 'two')\np.write_text(s)\n";
    assert_eq!(
        rewrites(source),
        vec![Rewrite {
            path: "src/a.rs".to_string(),
            replaced: vec![pair("one", "two")],
        }]
    );
}

#[test]
fn replacements_chain_in_the_order_written() {
    let source = "s = open('a.txt').read()\nopen('a.txt', 'w').write(s.replace('a', 'b').replace(\"c\", '''d\ne'''))\n";
    assert_eq!(
        rewrites(source),
        vec![Rewrite {
            path: "a.txt".to_string(),
            replaced: vec![pair("a", "b"), pair("c", "d\ne")],
        }]
    );
}

#[test]
fn each_file_of_a_script_that_moves_on_is_its_own_edit() {
    let source = "p = 'a.txt'\ns = open(p).read()\nopen(p, 'w').write(s.replace('x', 'y'))\np = 'b.txt'\ns = open(p).read()\nopen(p, 'w').write(s.replace('u', 'v'))\n";
    let found = rewrites(source);
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].path, "a.txt");
    assert_eq!(found[1].path, "b.txt");
    assert_eq!(found[1].replaced, vec![pair("u", "v")]);
}

#[test]
fn a_check_on_the_text_does_not_stop_the_reading() {
    let source = "s = open('a').read()\nassert s.count('x') == 1\nopen('a', 'w').write(s.replace('x', 'y'))\n";
    assert_eq!(rewrites(source).len(), 1);
}

#[test]
fn anything_indented_makes_the_order_unknowable() {
    // The replace might run once, never, or for every item.
    let source = "s = open('a').read()\nif 'x' in s:\n    s = s.replace('x', 'y')\nopen('a', 'w').write(s)\n";
    assert!(rewrites(source).is_empty());
}

#[test]
fn a_condition_on_one_line_is_still_a_condition() {
    let source =
        "s = open('a').read()\nif 'x' in s: s = s.replace('x', 'y')\nopen('a', 'w').write(s)\n";
    assert!(rewrites(source).is_empty());
}

#[test]
fn a_computed_replacement_is_not_a_known_one() {
    let source = "s = open('a').read()\nnew = make()\nopen('a', 'w').write(s.replace('x', new))\n";
    assert!(rewrites(source).is_empty());
}

#[test]
fn text_written_to_another_file_is_not_an_edit_of_either() {
    let source = "s = open('a').read()\nopen('b', 'w').write(s.replace('x', 'y'))\n";
    assert!(rewrites(source).is_empty());
}

#[test]
fn a_formatted_string_has_no_value_here() {
    let source = "s = open('a').read()\nopen('a', 'w').write(s.replace('x', f'{y}'))\n";
    assert!(rewrites(source).is_empty());
}

#[test]
fn a_replacement_limited_by_count_is_refused() {
    let source = "s = open('a').read()\nopen('a', 'w').write(s.replace('x', 'y', 1))\n";
    assert!(rewrites(source).is_empty());
}

#[test]
fn a_heredoc_edit_is_resolved_where_the_shell_was() {
    let command = "cd /repo && python3 - <<'EOF'\ns = open('src/a.rs').read()\nopen('src/a.rs', 'w').write(s.replace('one', 'two'))\nEOF";
    let parsed = reader::project::read(command).expect("parses");
    let found = reader::shell_files::extract_knowing(&parsed, Some("/elsewhere"), "/home/me", &[]);
    assert_eq!(
        found.rewrites,
        vec![Rewrite {
            path: "/repo/src/a.rs".to_string(),
            replaced: vec![pair("one", "two")],
        }]
    );
}
