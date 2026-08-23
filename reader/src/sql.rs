//! What the fleet's SQL did, in tables.
//!
//! The third carried language, and the first that names something other than a
//! file. `python.rs` and `javascript.rs` answer "which paths did this touch";
//! this answers "which tables did this read, and which did it change" — a
//! different kind of subject, kept in a different field for that reason.
//!
//! ⚠ **It contributes NOTHING to any file count, and that is a measurement
//! rather than a simplification.** Over 5,727 corpus commands carrying a SQL
//! client there is not one `INTO OUTFILE`, not one `LOAD DATA INFILE`, and not
//! one sqlite `.read`/`.output`/`.dump`. The forms are read anyway — a rule
//! apiece against a silent write — but a table is not a file and folding the two
//! together would inflate the figure the whole reader is judged on.
//!
//! ⚠ **The direction of a table is decided by the VERB, never by the clause it
//! sits in.** `SELECT … FROM x` reads `x`; `DELETE FROM x` changes it. A reader
//! that mapped `FROM` to "read" would report every deletion in the corpus as a
//! read of the table it emptied.

use std::collections::BTreeMap;

use pest::Parser;
use pest::iterators::Pair;
use pest_derive::Parser;

use crate::program::Use;

#[derive(Parser)]
#[grammar = "sql.pest"]
struct SqlParser;

/// What one SQL script did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Queried {
    /// Tables read, and how often.
    pub reads: BTreeMap<String, usize>,
    /// Tables changed, and how often.
    pub writes: BTreeMap<String, usize>,
    /// Files named by `INTO OUTFILE`, `INFILE`, `.read` or `.output`.
    ///
    /// Empty over this corpus. Kept because the alternative to reading these is
    /// a write nobody sees.
    pub uses: Vec<Use>,
    /// Statements by verb — the shape of the work, as `Op` is for the shell.
    pub verbs: BTreeMap<String, usize>,
    /// Statements whose verb the grammar recognised and this did not classify.
    /// The worklist, same as the other two readers keep.
    pub unknown: usize,
}

impl Queried {
    /// Whether anything at all was understood — used to tell "no SQL here" from
    /// "SQL that said nothing about tables".
    pub fn is_empty(&self) -> bool {
        self.reads.is_empty()
            && self.writes.is_empty()
            && self.uses.is_empty()
            && self.verbs.is_empty()
    }

    /// Distinct tables touched either way.
    pub fn tables(&self) -> usize {
        self.reads
            .keys()
            .chain(self.writes.keys())
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    }
}

/// What a statement's verb does to the tables it goes on to name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    /// `SELECT`, `SHOW`, `DESCRIBE` — every table named is read.
    Reads,
    /// `DELETE`, `TRUNCATE`, `DROP`, `ALTER`, `CREATE` — the table named by the
    /// statement's own clause is changed.
    ///
    /// ⚠ A `JOIN` under one of these is still a READ: `DELETE a FROM a JOIN b`
    /// empties `a` and only consults `b`.
    Changes,
    /// `INSERT`, `REPLACE`, `UPDATE` — the target is changed and anything it
    /// selects from is read, so the clause decides and the verb only says which
    /// clause is the target.
    Mixed,
}

/// What a verb does, and what to call it in a tally.
///
/// ⚠ **Takes the `Rule`, not its name.** This first took a `&str` and matched
/// literals — which meant formatting the rule with `{:?}`, lowercasing it, and
/// matching the result: a closed set laundered through a string and back, where
/// a renamed rule becomes a silent `other` instead of a compile error. The
/// grammar already gives us the enum.
// ⚠ NOT named `verb`: the grammar has a `verb` rule, so a binding by that name
// shadows `Rule::verb` and rustc denies it outright.
fn direction(kind: Rule) -> (Direction, &'static str) {
    match kind {
        Rule::select => (Direction::Reads, "select"),
        Rule::show => (Direction::Reads, "show"),
        Rule::describe => (Direction::Reads, "describe"),
        Rule::insert => (Direction::Mixed, "insert"),
        Rule::replace => (Direction::Mixed, "replace"),
        Rule::update => (Direction::Mixed, "update"),
        Rule::delete => (Direction::Changes, "delete"),
        Rule::truncate => (Direction::Changes, "truncate"),
        Rule::create_table => (Direction::Changes, "create"),
        Rule::drop_table => (Direction::Changes, "drop"),
        Rule::alter_table => (Direction::Changes, "alter"),
        _ => (Direction::Reads, "other"),
    }
}

/// Read a SQL script.
///
/// Never fails: `stray` accepts any token the grammar has no reading for, so
/// coverage is measured by what was *understood* rather than by whether it
/// parsed — the same rule the Python and JavaScript readers are grown by.
pub fn read(source: &str) -> Queried {
    let mut out = Queried::default();
    let Ok(mut parsed) = SqlParser::parse(Rule::script, source) else {
        return out;
    };
    let script = parsed.next().expect("script always yields one pair");
    for item in script.into_inner() {
        match item.as_rule() {
            Rule::general => statement(item, &mut out),
            Rule::targeted => targeted(item, &mut out),
            Rule::dot_command => dot_command(item, &mut out),
            _ => {}
        }
    }
    out
}

/// A statement whose target follows the verb directly: `UPDATE x`, `DROP TABLE
/// x`. The first table list is what it changes; anything after is read.
fn targeted(pair: Pair<Rule>, out: &mut Queried) {
    let mut inner = pair.into_inner();
    let Some(verb_pair) = inner.next() else {
        return;
    };
    let Some(verb_rule) = verb_pair.into_inner().next().map(|v| v.as_rule()) else {
        return;
    };
    let (_, name) = direction(verb_rule);
    *out.verbs.entry(name.to_string()).or_insert(0) += 1;

    let mut took_target = false;
    for part in inner {
        match part.as_rule() {
            Rule::if_exists => {}
            Rule::table_list if !took_target => {
                took_target = true;
                tables(part, out, true);
            }
            // ⚠ Everything after the target is CONSULTED, not changed:
            // `UPDATE a SET x = (SELECT y FROM b)` writes `a` and reads `b`.
            Rule::from | Rule::table_list => tables(part, out, false),
            Rule::join => tables(part, out, false),
            Rule::into_table => tables(part, out, true),
            Rule::into_outfile => file(part, out, true),
            Rule::load_infile => file(part, out, false),
            _ => {}
        }
    }
}

fn statement(pair: Pair<Rule>, out: &mut Queried) {
    let mut inner = pair.into_inner();
    let Some(verb_pair) = inner.next() else {
        return;
    };
    let Some(verb_rule) = verb_pair.into_inner().next().map(|v| v.as_rule()) else {
        return;
    };
    let (how, name) = direction(verb_rule);
    *out.verbs.entry(name.to_string()).or_insert(0) += 1;
    if name == "other" {
        out.unknown += 1;
    }

    for clause in inner {
        match clause.as_rule() {
            // ⚠ `FROM` is a read under `SELECT` and a WRITE under `DELETE`. This
            // one line is the reason `Direction` exists.
            Rule::from => {
                let writes = how == Direction::Changes;
                tables(clause, out, writes);
            }
            // A join is consulted, never targeted — under every verb.
            Rule::join => tables(clause, out, false),
            Rule::into_table => tables(clause, out, true),
            Rule::into_outfile => file(clause, out, true),
            Rule::load_infile => file(clause, out, false),
            _ => {}
        }
    }
}

/// Record every `table_ref` under a clause.
fn tables(clause: Pair<Rule>, out: &mut Queried, writes: bool) {
    for node in clause.into_inner().flatten() {
        if node.as_rule() != Rule::qualified {
            continue;
        }
        let name = qualified_name(node.as_str());
        if name.is_empty() {
            continue;
        }
        let side = if writes {
            &mut out.writes
        } else {
            &mut out.reads
        };
        *side.entry(name).or_insert(0) += 1;
    }
}

fn file(clause: Pair<Rule>, out: &mut Queried, write: bool) {
    for node in clause.into_inner() {
        if node.as_rule() == Rule::string {
            let path = unquote(node.as_str());
            if !path.is_empty() {
                out.uses.push(Use { path, write });
            }
        }
    }
}

/// sqlite3's dot commands. Only two name a file.
fn dot_command(pair: Pair<Rule>, out: &mut Queried) {
    let mut inner = pair.into_inner();
    let Some(name) = inner.next() else { return };
    let write = match name.as_str() {
        "read" => false,
        "output" | "once" | "dump" | "backup" => true,
        // `.tables`, `.schema`, `.headers`, `.mode` — no subject at all.
        _ => return,
    };
    *out.verbs.entry(format!(".{}", name.as_str())).or_insert(0) += 1;
    for arg in inner {
        let path = unquote(arg.as_str());
        // `.output stdout` names a stream, not a file — and `.dump` with no
        // argument writes to whatever is already open.
        if path.is_empty() || path == "stdout" || path == "stderr" {
            continue;
        }
        out.uses.push(Use { path, write });
    }
}

/// Strip the quoting a name or literal was written with.
///
/// ⚠ **Doubled quotes are an ESCAPE inside a SQL string** — `'it''s'` is one
/// value, not two — so they collapse rather than terminating.
fn qualified_name(text: &str) -> String {
    text.split('.').map(unquote).collect::<Vec<_>>().join(".")
}

fn unquote(text: &str) -> String {
    let text = text.trim();
    let mut chars = text.chars();
    match (chars.next(), text.chars().last()) {
        (Some('`'), Some('`')) if text.len() >= 2 => text[1..text.len() - 1].to_string(),
        (Some('\''), Some('\'')) if text.len() >= 2 => text[1..text.len() - 1].replace("''", "'"),
        (Some('"'), Some('"')) if text.len() >= 2 => text[1..text.len() - 1].replace("\"\"", "\""),
        _ => text.to_string(),
    }
}
