//! What the SQL grammar makes of one script — the parse tree, printed.
//!
//! A probe, not a check: when a clause silently fails to match there is nothing
//! in the reader's output to look at, and guessing which rule bailed is how an
//! afternoon disappears.
use pest::Parser;
use pest::iterators::Pair;

#[derive(pest_derive::Parser)]
#[grammar = "sql.pest"]
struct P;

fn show(p: Pair<Rule>, depth: usize) {
    let text = p.as_str().replace('\n', "⏎");
    let text = if text.len() > 48 { format!("{}…", &text[..48]) } else { text };
    println!("{:indent$}{:?}  {}", "", p.as_rule(), text, indent = depth * 2);
    for kid in p.into_inner() {
        show(kid, depth + 1);
    }
}

fn main() {
    let src = std::env::args().nth(1).unwrap_or_else(|| {
        "SELECT * FROM report INTO OUTFILE '/tmp/report.tsv'".to_string()
    });
    match P::parse(Rule::script, &src) {
        Ok(mut got) => show(got.next().expect("one script"), 0),
        Err(e) => println!("refused: {e}"),
    }
    println!("\n-- what the reader made of it --");
    let q = reader::sql::read(&src);
    println!("reads:  {:?}", q.reads);
    println!("writes: {:?}", q.writes);
    println!("uses:   {:?}", q.uses);
    println!("verbs:  {:?}", q.verbs);
}
