//! What a person actually said in a conversation.
//!
//!     cargo run --release -p reader --example said -- <transcript.jsonl>
//!
//! The thing memview#1215 says everyone hand-rolls. It is here as the worked
//! example of [`reader::transcript::human_turns`] and as its check against a
//! real file: the count and the character lengths can be read against the
//! console's own `accepted N characters to send` log, which is an independent
//! witness written by a different process.
use reader::transcript::human_turns;

fn main() -> anyhow::Result<()> {
    let path = std::env::args()
        .nth(1)
        .expect("usage: said <transcript.jsonl>");
    let bytes = std::fs::read(&path)?;
    let turns = human_turns(&bytes);
    println!("{} human turns", turns.len());
    for turn in &turns {
        let mark = if turn.queued { "queued" } else { "      " };
        let text = turn.text.replace('\n', " ");
        let text: String = text.chars().take(88).collect();
        println!(
            "  {}  {mark} {:>5}  {text}",
            turn.at,
            turn.text.chars().count()
        );
    }
    Ok(())
}
