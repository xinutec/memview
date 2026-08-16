//! Which constructs does a command need, not which one stopped us first.
//!
//! ⚠ **The refusal ranking is not a work queue on its own.** [`parse`] stops at
//! the first thing it cannot read, so a command holding a pipe and a redirection
//! is counted once, under whichever the scan reached first. Reading those
//! percentages as "what building X would unlock" is wrong in both directions: it
//! over-credits whatever appears leftmost and hides that a command needs *every*
//! construct in it modelled before it can be read at all.
//!
//! This is the survey that fixes it — a scan that keeps going and returns the
//! whole set. `survey(t)` empty means [`parse`] would accept `t`.
//!
//! **It is a scanner, not a parser**, and the difference is deliberate: it needs
//! to answer a question about text this parser cannot read, so it cannot be
//! built out of that parser. It shares the quoting rules and nothing else.
//!
//! Two things are looked past rather than into, and both are stated as findings
//! rather than descended into:
//!
//! - **a substitution's interior.** `$(git log | head)` reports `Expansion`, not
//!   `Expansion` and `Pipe`. At this layer the substitution is what blocks the
//!   read; what is inside it is a question for the layer that gets one.
//! - **a heredoc body.** It is data — a commit message, Python, YAML — and
//!   scanning prose as shell invents constructs nobody wrote.
//!
//! ⚠ The survey can only ever be *approximately* right, so it is pinned to the
//! parser by an invariant rather than trusted: whatever [`parse`] refuses must
//! appear in the survey's set. `reader/tests/syntax.rs` asserts it, and the
//! corpus report re-checks it on every row.

use std::collections::BTreeSet;

use super::parse::{Reason, is_assignment, is_reserved};

pub fn survey(text: &str) -> BTreeSet<Reason> {
    Survey {
        bytes: text.as_bytes(),
        text,
        at: 0,
        found: BTreeSet::new(),
    }
    .run()
}

struct Survey<'t> {
    bytes: &'t [u8],
    text: &'t str,
    at: usize,
    found: BTreeSet<Reason>,
}

impl Survey<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn peek_at(&self, ahead: usize) -> Option<u8> {
        self.bytes.get(self.at + ahead).copied()
    }

    fn run(mut self) -> BTreeSet<Reason> {
        // Delimiters of heredocs opened on this line, skipped once the line ends.
        let mut heredocs: Vec<String> = Vec::new();
        // A word is only grammar at the head of a command, so the scan has to
        // know where commands begin — which is after every separator, not only
        // at the start of the text.
        let mut at_command_start = true;
        // `time` and `!` are grammar only where a pipeline begins. Every other
        // separator opens one; a `|` does not.
        let mut at_pipeline_head = true;
        // `-p` is a prefix only after `time`; on its own it is an argument.
        let mut seen_time = false;
        let mut word = String::new();
        let mut word_quoted = false;
        let mut in_word = false;

        // ⚠ **Finishing a word clears `at_command_start`, and only finishing a
        // word does.** Preserving it here instead made every word on a line look
        // like a command name, so `ssh -o BatchMode=yes …` reported an
        // assignment prefix that is plainly an argument — 191 commands where the
        // survey claimed a construct the parser had accepted. A separator sets
        // the flag back on afterwards; whitespace must not.
        // Three ways a word can end, and they differ only in what the shell is
        // at afterwards. Split rather than parameterised because a conditional
        // store followed by an unconditional one is a dead store the linter is
        // right about — and because naming them says what each separator means.
        macro_rules! finish {
            () => {
                if in_word {
                    finish_word(
                        &mut self.found,
                        &word,
                        at_command_start,
                        at_pipeline_head,
                        word_quoted,
                    );
                    in_word = false;
                    word.clear();
                    word_quoted = false;
                }
            };
        }
        // Whitespace: still inside the same command, unless a word just ended.
        macro_rules! end_word {
            () => {
                if in_word {
                    // ⚠ Decided BEFORE `finish!`, assigned after. `finish_word`
                    // reads both flags, so clearing them first makes every
                    // command name look like an argument — `FOO=bar cmd` then
                    // reports nothing at all.
                    //
                    // A pipeline's prefix words do not end its head: in
                    // `time ! a` the head is still open and `a` is the name.
                    let keeps_head = at_pipeline_head
                        && (word == "!" || word == "time" || (seen_time && word == "-p"));
                    if at_pipeline_head && word == "time" {
                        seen_time = true;
                    }
                    finish!();
                    // ⚠ A prefix word does not start the command either: after
                    // `time`, the NEXT word is the command name, and treating it
                    // as an argument hid the assignment in
                    // `time PYTHONPATH=… python -m x`.
                    at_command_start = keeps_head;
                    at_pipeline_head = keeps_head;
                }
            };
        }
        // `;`, a newline, `&`, `&&`, `||`, a group: a new pipeline starts.
        macro_rules! separator {
            () => {
                finish!();
                at_command_start = true;
                at_pipeline_head = true;
                seen_time = false;
            };
        }
        // A single `|`: a new command, but the SAME pipeline — which is what
        // makes `! a | b` legal and `a | ! b` a syntax error.
        macro_rules! pipe {
            () => {
                finish!();
                at_command_start = true;
                at_pipeline_head = false;
                seen_time = false;
            };
        }

        while let Some(byte) = self.peek() {
            match byte {
                b' ' | b'\t' | b'\r' => {
                    end_word!();
                    self.at += 1;
                }
                b'\\' if self.peek_at(1) == Some(b'\n') => {
                    self.at += 2;
                }
                b'\n' => {
                    separator!();
                    self.at += 1;
                    for delimiter in heredocs.drain(..) {
                        self.skip_heredoc_body(&delimiter);
                    }
                }
                b';' => {
                    separator!();
                    self.at += 1;
                }
                b'#' if !in_word => {
                    while self.peek().is_some_and(|b| b != b'\n') {
                        self.at += 1;
                    }
                }
                b'|' => {
                    if self.peek_at(1) == Some(b'|') {
                        separator!();
                        self.at += 2;
                        self.after_connector();
                    } else {
                        pipe!();
                        // A pipe is modelled, so it is not a finding — but the
                        // command after it is NOT a pipeline head, which is
                        // what makes `a | ! b` a refusal and `! a | b` fine.
                        // A newline after it continues the pipeline, so it must
                        // be stepped over here or the next line looks like a
                        // fresh command.
                        self.at += 1;
                        while matches!(
                            self.peek(),
                            Some(b' ') | Some(b'\t') | Some(b'\r') | Some(b'\n')
                        ) {
                            self.at += 1;
                        }
                        if matches!(self.peek(), None | Some(b';') | Some(b'|')) {
                            self.found.insert(Reason::EmptyOperand);
                        }
                    }
                }
                b'&' => {
                    separator!();
                    if self.peek_at(1) == Some(b'&') {
                        self.at += 2;
                        self.after_connector();
                    } else {
                        // A bare `&` backgrounds the list and ends it; both are
                        // modelled, so neither is a finding.
                        self.at += 1;
                    }
                }
                b'<' | b'>' => {
                    end_word!();
                    // ⚠ Four different constructs share these two characters,
                    // and only one of them is "a redirection to a file". Counted
                    // apart because they are not one build: a heredoc's operand
                    // is on the FOLLOWING lines, and a process substitution is a
                    // whole command.
                    if self.peek_at(1) == Some(b'(') {
                        self.found.insert(Reason::ProcessSubstitution);
                        self.at += 2;
                    } else if byte == b'<'
                        && self.peek_at(1) == Some(b'<')
                        && self.peek_at(2) == Some(b'<')
                    {
                        self.found.insert(Reason::HereString);
                        self.at += 3;
                    } else if byte == b'<' && self.peek_at(1) == Some(b'<') {
                        self.found.insert(Reason::Heredoc);
                        self.at += 2;
                        if self.peek() == Some(b'-') {
                            self.at += 1;
                        }
                        if let Some(delimiter) = self.take_delimiter() {
                            heredocs.push(delimiter);
                        }
                    } else {
                        // Every other `<`/`>` form is modelled now.
                        self.at += 1;
                    }
                }
                b'(' | b')' | b'{' | b'}' => {
                    separator!();
                    self.found.insert(Reason::Grouping);
                    self.at += 1;
                }
                b'$' | b'`' => {
                    self.found.insert(Reason::Expansion);
                    in_word = true;
                    self.skip_expansion();
                }
                b'~' if !in_word => {
                    self.found.insert(Reason::Tilde);
                    in_word = true;
                    self.at += 1;
                }
                b'\'' => {
                    in_word = true;
                    word_quoted = true;
                    let from = self.at + 1;
                    if !self.skip_single_quote() {
                        self.found.insert(Reason::UnterminatedQuote);
                        break;
                    }
                    word.push_str(&self.text[from..self.at - 1]);
                }
                b'"' => {
                    in_word = true;
                    word_quoted = true;
                    if !self.skip_double_quote() {
                        self.found.insert(Reason::UnterminatedQuote);
                        break;
                    }
                }
                b'[' if in_word || self.closes_bracket() => {
                    // `[` alone is the test builtin; `[…]` inside a word is a
                    // bracket expression. Only the second is a construct.
                    if self.closes_bracket() {
                        self.found.insert(Reason::BracketExpression);
                    }
                    in_word = true;
                    word.push('[');
                    self.at += 1;
                }
                b'*' | b'?' => {
                    in_word = true;
                    self.at += 1;
                }
                b'\\' => match self.peek_at(1) {
                    Some(next) => {
                        in_word = true;
                        word.push(next as char);
                        self.at += 2;
                    }
                    None => {
                        self.found.insert(Reason::DanglingEscape);
                        break;
                    }
                },
                _ => {
                    in_word = true;
                    let from = self.at;
                    while let Some(next) = self.peek() {
                        if is_stop(next) {
                            break;
                        }
                        self.at += 1;
                    }
                    word.push_str(&self.text[from..self.at]);
                }
            }
        }
        // Not `end_word!` — nothing reads the scanner's state after this, and
        // assigning it here is three dead stores the linter is right about.
        // Direct, not `finish!` — nothing reads the word state after this, and
        // clearing it here is two dead stores.
        if in_word {
            finish_word(
                &mut self.found,
                &word,
                at_command_start,
                at_pipeline_head,
                word_quoted,
            );
        }
        self.found
    }

    /// Step over the blanks and newlines a list connector allows after it, and
    /// record what the parser would refuse there — a comment it cannot keep, or
    /// nothing at all.
    fn after_connector(&mut self) {
        while matches!(
            self.peek(),
            Some(b' ') | Some(b'\t') | Some(b'\r') | Some(b'\n')
        ) {
            self.at += 1;
        }
        match self.peek() {
            Some(b'#') => {
                self.found.insert(Reason::CommentInList);
            }
            None | Some(b';') | Some(b'|') | Some(b'&') => {
                self.found.insert(Reason::EmptyOperand);
            }
            _ => {}
        }
    }

    fn closes_bracket(&self) -> bool {
        let mut at = self.at + 1;
        while let Some(&byte) = self.bytes.get(at) {
            match byte {
                b']' => return true,
                b' ' | b'\t' | b'\n' | b'\r' | b';' => return false,
                _ => at += 1,
            }
        }
        false
    }

    /// The word after `<<`, unquoted, which ends the heredoc.
    fn take_delimiter(&mut self) -> Option<String> {
        while matches!(self.peek(), Some(b' ') | Some(b'\t')) {
            self.at += 1;
        }
        let mut delimiter = String::new();
        while let Some(byte) = self.peek() {
            match byte {
                b'\'' | b'"' => self.at += 1,
                b'\\' => self.at += 1,
                b if is_stop(b) => break,
                b => {
                    delimiter.push(b as char);
                    self.at += 1;
                }
            }
        }
        (!delimiter.is_empty()).then_some(delimiter)
    }

    /// Step over a heredoc body, which is data rather than shell.
    fn skip_heredoc_body(&mut self, delimiter: &str) {
        while self.at < self.bytes.len() {
            let line_start = self.at;
            while self.peek().is_some_and(|b| b != b'\n') {
                self.at += 1;
            }
            let line = &self.text[line_start..self.at];
            if self.peek() == Some(b'\n') {
                self.at += 1;
            }
            // `<<-` strips leading tabs from the terminator too.
            if line.trim_start_matches('\t').trim_end() == delimiter {
                return;
            }
        }
    }

    /// `$name`, `${…}`, `$(…)`, `$((…))` or a backtick run.
    fn skip_expansion(&mut self) {
        match (self.peek(), self.peek_at(1)) {
            (Some(b'`'), _) => {
                self.at += 1;
                while let Some(byte) = self.peek() {
                    self.at += 1;
                    if byte == b'`' {
                        return;
                    }
                }
            }
            (Some(b'$'), Some(b'(')) => {
                self.at += 1;
                self.skip_balanced(b'(', b')');
            }
            (Some(b'$'), Some(b'{')) => {
                self.at += 1;
                self.skip_balanced(b'{', b'}');
            }
            _ => {
                self.at += 1;
                while self
                    .peek()
                    .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_')
                {
                    self.at += 1;
                }
            }
        }
    }

    /// Step over a nested, quote-aware bracketed run. Quote-aware because
    /// `$(echo ")")` is one substitution and a naive counter ends it early.
    fn skip_balanced(&mut self, open: u8, close: u8) {
        let mut depth = 0usize;
        while let Some(byte) = self.peek() {
            match byte {
                b if b == open => {
                    depth += 1;
                    self.at += 1;
                }
                b if b == close => {
                    depth -= 1;
                    self.at += 1;
                    if depth == 0 {
                        return;
                    }
                }
                b'\'' => {
                    if !self.skip_single_quote() {
                        return;
                    }
                }
                b'"' => {
                    if !self.skip_double_quote() {
                        return;
                    }
                }
                b'\\' => self.at += 2,
                _ => self.at += 1,
            }
        }
    }

    /// `true` if the quote closed.
    fn skip_single_quote(&mut self) -> bool {
        self.at += 1;
        while let Some(byte) = self.peek() {
            self.at += 1;
            if byte == b'\'' {
                return true;
            }
        }
        false
    }

    fn skip_double_quote(&mut self) -> bool {
        self.at += 1;
        while let Some(byte) = self.peek() {
            match byte {
                b'"' => {
                    self.at += 1;
                    return true;
                }
                b'\\' => self.at += 2,
                b'$' | b'`' => {
                    self.found.insert(Reason::Expansion);
                    self.skip_expansion();
                }
                _ => self.at += 1,
            }
        }
        false
    }
}

/// A finished word is only grammar where a command begins, and only unquoted:
/// `'time'` at the head of a command is a program, not the keyword.
fn finish_word(
    found: &mut BTreeSet<Reason>,
    word: &str,
    at_command_start: bool,
    at_pipeline_head: bool,
    quoted: bool,
) {
    // `!` opening a pipeline is grammar the tree models; the same `!` after a
    // `|` is a syntax error bash refuses, and the parser refuses it too.
    if at_pipeline_head && word == "!" {
        return;
    }
    if at_command_start && !quoted {
        if is_reserved(word) {
            found.insert(Reason::ReservedWord);
        } else if is_assignment(word) {
            found.insert(Reason::Assignment);
        }
    }
}

fn is_stop(byte: u8) -> bool {
    matches!(
        byte,
        b' ' | b'\t'
            | b'\r'
            | b'\n'
            | b';'
            | b'\''
            | b'"'
            | b'\\'
            | b'|'
            | b'&'
            | b'<'
            | b'>'
            | b'('
            | b')'
            | b'{'
            | b'}'
            | b'$'
            | b'`'
            | b'*'
            | b'?'
            | b'['
    )
}
