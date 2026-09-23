//! The Python tree: what it reads, what it refuses, and the round-trip law.

use reader::syntax::python::ast::{Expr, FPart, Stmt, StmtKind};
use reader::syntax::python::{Outcome, Reason, check, parse, print};

fn holds(text: &str) -> String {
    match check(text) {
        Outcome::Holds { printed } => printed,
        other => panic!("the law does not hold for {text:?}: {other:?}"),
    }
}

fn body(text: &str) -> Vec<Stmt> {
    parse(text).expect("parses").body
}

#[test]
fn a_script_of_the_corpus_shape_reads_and_prints_back() {
    let printed = holds(
        "from pathlib import Path\np=Path('src/a.rs')\ns=p.read_text()\ns=s.replace('one','two')\np.write_text(s)\nprint('done')\n",
    );
    assert_eq!(
        printed,
        "from pathlib import Path\np = Path('src/a.rs')\ns = p.read_text()\ns = s.replace('one', 'two')\np.write_text(s)\nprint('done')\n"
    );
}

#[test]
fn quoting_is_not_in_the_tree() {
    assert_eq!(body("'a'\n"), body("\"a\"\n"));
    assert_eq!(body("'a'\n"), body("'''a'''\n"));
}

#[test]
fn adjacent_literals_are_one_value() {
    assert_eq!(body("x = 'a' 'b'\n"), body("x = 'ab'\n"));
}

#[test]
fn elif_is_an_else_holding_an_if() {
    let tree = body("if a:\n    x\nelif b:\n    y\nelse:\n    z\n");
    let StmtKind::If { orelse, .. } = &tree[0].kind else {
        panic!("an if");
    };
    assert!(matches!(
        orelse.as_slice(),
        [Stmt {
            kind: StmtKind::If { .. },
            ..
        }]
    ));
    assert!(holds("if a:\n    x\nelif b:\n    y\nelse:\n    z\n").contains("elif b:"));
}

#[test]
fn an_f_string_holds_its_fields_as_expressions() {
    let tree = body("f'{a}-{b!r:>4}'\n");
    let StmtKind::Expr(Expr::FString(parts)) = &tree[0].kind else {
        panic!("an f-string");
    };
    assert!(
        matches!(parts.as_slice(), [FPart::Field { .. }, FPart::Text(t), FPart::Field { conversion: Some('r'), .. }] if t == "-")
    );
    holds("f'{a}-{b!r:>4}'\n");
}

#[test]
fn a_literal_inside_a_field_keeps_the_other_quote() {
    // Before 3.12 the outer quote ends the string, so the print must not reuse it.
    assert_eq!(holds("f\"{d['k']}\"\n"), "f'{d[\"k\"]}'\n");
}

#[test]
fn a_comment_after_a_block_stays_after_it() {
    let text = "for w in ws: print(w)\n# after\nx = 1\n";
    let tree = body(text);
    assert!(
        matches!(tree[1].kind, StmtKind::Comment(_)),
        "the comment is not in the loop"
    );
    holds(text);
}

#[test]
fn a_comment_indented_into_a_block_is_in_it() {
    let tree = body("def f():\n    a = 1\n    # inside\nb = 2\n");
    let StmtKind::FunctionDef { body, .. } = &tree[0].kind else {
        panic!("a def");
    };
    assert!(matches!(
        body.last().map(|s| &s.kind),
        Some(StmtKind::Comment(_))
    ));
}

#[test]
fn a_walrus_in_a_subscript_keeps_its_brackets() {
    assert!(holds("t[(y := 1, 2)]\n").contains("(y := 1)"));
}

#[test]
fn a_generator_as_the_only_argument_reads_as_one() {
    holds("print(all(x > 0 for x in xs))\n");
}

#[test]
fn precedence_survives_the_print() {
    for text in [
        "x = -a ** -b\n",
        "x = (a + b) * c\n",
        "x = a if b else (c if d else e)\n",
        "x = not (a and b) or c\n",
        "f(lambda y: y + 1)\n",
        "x = (a, b)[0]\n",
    ] {
        holds(text);
    }
}

#[test]
fn an_escaped_quote_in_a_field_is_a_program_that_never_ran() {
    let refused = parse("print(f\"{d[\\\"k\\\"]}\")\n").expect_err("CPython refuses it too");
    assert_eq!(refused.reason, Reason::EscapedQuoteInField);
}

#[test]
fn what_the_corpus_does_not_write_is_refused_by_name() {
    for (text, reason) in [
        ("class A:\n    pass\n", Reason::Class),
        ("async def f():\n    pass\n", Reason::Async),
        ("def f():\n    yield 1\n", Reason::Yield),
        ("x: int = 1\n", Reason::Annotation),
        ("f(a, # note\n  b)\n", Reason::CommentInBrackets),
    ] {
        assert_eq!(parse(text).expect_err(text).reason, reason, "{text}");
    }
}

#[test]
fn the_printer_is_a_pure_function_of_the_tree() {
    let tree = parse("x=1;y = 2  # set\n").expect("parses");
    assert_eq!(print(&tree), "x = 1\ny = 2  # set\n");
}
