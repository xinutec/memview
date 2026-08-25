//! What the Python grammar makes of one program — the parse tree, printed.
//!
//!     cargo run --release -p reader --example python-probe -- "for p in files: open(p)"
//!
//! A probe, not a check, and the sibling of `sql-probe` for the same reason:
//! when a rule silently fails to match there is nothing in the reader's output
//! to look at, and guessing which one bailed is how an afternoon disappears.
//!
//! ⚠ **A statement is not always the element you would draw.** `for p in xs:`
//! is a `binder` holding only `for p` — the `in` and the iterable are separate
//! elements after it — which is why a rule about what a loop ranges over cannot
//! be written against the binder alone.
use pest::Parser;
use pest::iterators::Pair;

#[derive(pest_derive::Parser)]
#[grammar = "python.pest"]
struct P;

fn show(p: Pair<Rule>, depth: usize) {
    let text = p.as_str().replace('\n', "⏎");
    let text = if text.chars().count() > 48 {
        format!("{}…", text.chars().take(48).collect::<String>())
    } else {
        text
    };
    println!(
        "{:indent$}{:?}  {}",
        "",
        p.as_rule(),
        text,
        indent = depth * 2
    );
    for kid in p.into_inner() {
        show(kid, depth + 1);
    }
}

fn main() {
    let src = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "for p in glob.glob('captures/*.json'): open(p)".to_string());
    match P::parse(Rule::program, &src) {
        Ok(mut got) => show(got.next().expect("one program"), 0),
        Err(e) => println!("refused: {e}"),
    }
    println!("\n-- what the reader made of it --");
    let program = reader::python::read(&src);
    println!("uses:       {:?}", program.uses);
    println!("bounded:    {:?}", program.bounded);
    println!("located:    {:?}", program.located);
    println!("unresolved: {:?}", program.unresolved);
    println!("unknown:    {:?}", program.unknown);
}
