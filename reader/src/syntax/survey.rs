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
//! One thing is looked past rather than into: **a heredoc body.** It is data — a
//! commit message, Python, YAML — and scanning prose as shell invents constructs
//! nobody wrote. Everything else is descended into, including a substitution's
//! interior and a process substitution's, because the parser reads those and
//! what it refuses in there is a construct this command genuinely needs.
//!
//! ⚠ **An EXTRA finding is as wrong as a missing one, and only the extras
//! hide** — the invariant below pins one direction, so an over-report on a
//! command that was refused anyway passes silently. It cost the survey's only
//! product once already: see the `)` in [`Survey::process_substitution`].
//!
//! ⚠ The survey can only ever be *approximately* right, so it is pinned to the
//! parser by an invariant rather than trusted: whatever [`parse`] refuses must
//! appear in the survey's set. `reader/tests/syntax.rs` asserts it, and the
//! corpus report re-checks it on every row.

use std::collections::BTreeSet;

use super::parse::{
    Bracket, Reason, brace_expansion, bracket_expression, classify_expansion, opens_assignment,
    reserved_word,
};

pub fn survey(text: &str) -> BTreeSet<Reason> {
    scan(text).found
}

/// The whole scanner state after a run, not just the set it found.
///
/// ⚠ **A substitution needs more than the set.** Its interior is a script the
/// parser reads, so every construct in there is reported by descending — but the
/// parser also refuses a comment *because* it is inside one, which is not a
/// finding on its own. That fact comes back as a flag.
fn scan(text: &str) -> Survey<'_> {
    Survey {
        bytes: text.as_bytes(),
        text,
        at: 0,
        found: BTreeSet::new(),
        saw_comment: false,
    }
    .run()
}

struct Survey<'t> {
    bytes: &'t [u8],
    text: &'t str,
    at: usize,
    found: BTreeSet<Reason>,
    /// Did a comment appear? Refused inside a substitution for the same reason
    /// a comment in a loop body is: the printer writes both inline.
    saw_comment: bool,
}

impl Survey<'_> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn peek_at(&self, ahead: usize) -> Option<u8> {
        self.bytes.get(self.at + ahead).copied()
    }

    fn run(mut self) -> Self {
        // Heredocs opened on this line, skipped once the line ends: the
        // delimiter, and whether `<<-` lets a terminator be tab-indented.
        let mut heredocs: Vec<(String, bool)> = Vec::new();
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
        // Is the word being read a `NAME=value` prefix rather than a word?
        let mut word_is_binding = false;
        // ⚠ Where a loop is, so the two shapes the parser refuses inside one can
        // be reported: a comment (the printer puts a loop on one line, so there
        // is nowhere to put it) and a redirection in a `for` header (a syntax
        // error to bash as well). Neither is visible without knowing that a
        // loop is open.
        let mut loop_header = false;
        let mut loop_depth = 0usize;
        // ⚠ Where the `if` chain stands — see [`conditional_keyword`]. The
        // keywords are modelled now, so an `if` is not a finding, but the shapes
        // bash refuses still are and nothing else here can see them.
        let mut if_stack: Vec<bool> = Vec::new();
        // Open `( … )` and `{ … ; }`. An unmatched one is a refusal the parser
        // makes and nothing else here can see.
        let mut parens = 0usize;
        let mut braces = 0usize;
        // ⚠ Where each open `case` stands, innermost last — see [`CaseStage`].
        // It is the only thing that can tell an arm's `)` apart from an
        // unmatched one, and the `(` in front of a pattern from a subshell.
        let mut cases: Vec<CaseStage> = Vec::new();
        // How far through a `for NAME in` header the scan is: 1 after `for`,
        // 2 after the name. A word other than `in` at 2 is a header bash refuses
        // — `for p /style.css; do …` — and the parser refuses it as a loop.
        let mut for_stage = 0u8;
        // Did the word just finished open a new command list — `do`, `then`?
        let mut word_opens_list = false;
        // ⚠ Was the word just finished a command NAME? `probe () { … }` is a
        // definition with a blank before the parens, so by the time the `(` is
        // read the name is behind us and `at_command_start` has been cleared.
        let mut after_name = false;

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
                    // ⚠ Here rather than on the whitespace path alone, because a
                    // keyword can end a word at a `;` or a newline too:
                    // `for f in a\ndo\n…` left the header open, and every `>` in
                    // the body then read as a malformed header.
                    // ⚠ A `for` header that never says `in` is a syntax error,
                    // and the parser calls it an unreadable loop. Outside the
                    // `at_command_start` guard, because only the FIRST word of a
                    // command is at a command start — the name and the `in` that
                    // follow it are not, and inside the guard this never ran
                    // past the keyword itself.
                    match for_stage {
                        1 => for_stage = 2,
                        2 => {
                            if word != "in" {
                                self.found.insert(Reason::Loop);
                            }
                            for_stage = 0;
                        }
                        _ => {}
                    }
                    word_opens_list = false;
                    // ⚠ `case x in` — the same shape as a `for` header, and
                    // outside the `at_command_start` guard for the same reason:
                    // the subject and the `in` are not at a command start. After
                    // it, the arms are — which is what lets a bare
                    // `case $x in esac` find its own closing keyword.
                    match cases.last_mut() {
                        Some(stage @ CaseStage::Subject) => *stage = CaseStage::In,
                        Some(stage @ CaseStage::In) => {
                            if word != "in" {
                                self.found.insert(Reason::Case);
                            }
                            *stage = CaseStage::Pattern;
                            word_opens_list = true;
                        }
                        _ => {}
                    }
                    if at_command_start && !word_quoted {
                        if let Some(branch) = branch(&word) {
                            conditional_keyword(&mut if_stack, &mut self.found, branch);
                        }
                        match word.as_str() {
                            // ⚠ Only `for` and `select`. A `while` condition is
                            // an ordinary command list and may redirect —
                            // `while a > out; do x; done` is legal, and flagging
                            // it claimed a construct on 1977 commands the parser
                            // reads perfectly well.
                            "for" | "select" => {
                                loop_header = true;
                                for_stage = 1;
                            }
                            "while" | "until" => loop_header = false,
                            // ⚠ A new command list starts after these, so the
                            // next word is a command NAME. Without it, `do [[ …`
                            // read the `[[` as an argument and the survey missed
                            // a construct the parser refuses.
                            "do" => {
                                loop_header = false;
                                word_opens_list = true;
                                loop_depth += 1;
                            }
                            "done" => loop_depth = loop_depth.saturating_sub(1),
                            // A command list starts after each of these too; the
                            // chain itself is tracked above.
                            "if" | "then" | "elif" | "else" => {
                                loop_header = false;
                                word_opens_list = true;
                            }
                            // Modelled now. What is still reported is a header
                            // that never says `in`, an `esac` closing nothing,
                            // and — at the end of the run — a `case` that never
                            // closes.
                            "case" => {
                                loop_header = false;
                                cases.push(CaseStage::Subject);
                            }
                            // An `esac` closing nothing, or closing a header
                            // that never reached its arms.
                            "esac"
                                if !matches!(
                                    cases.pop(),
                                    Some(CaseStage::Pattern | CaseStage::Body)
                                ) =>
                            {
                                self.found.insert(Reason::Case);
                            }
                            _ => {}
                        }
                    }
                    finish_word(
                        &mut self.found,
                        &word,
                        at_command_start && !word_is_binding,
                        at_pipeline_head,
                        word_quoted,
                    );
                    in_word = false;
                    word.clear();
                    word_quoted = false;
                    word_is_binding = false;
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
                    // ⚠ A binding keeps the command START open and closes the
                    // pipeline HEAD: `A=1 time x` runs /usr/bin/time, because
                    // `time` is grammar only before a whole pipeline.
                    let was_binding = word_is_binding;
                    finish!();
                    // ⚠ A prefix word does not start the command either: after
                    // `time`, the NEXT word is the command name, and treating it
                    // as an argument hid the assignment in
                    // `time PYTHONPATH=… python -m x`.
                    after_name = at_command_start && !keeps_head && !was_binding;
                    at_command_start = keeps_head || was_binding || word_opens_list;
                    at_pipeline_head = keeps_head || word_opens_list;
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
                after_name = false;
                // `for f; do …` is legal — the header simply ends here. Written
                // as a test so it reads the value it clears, which is also what
                // says the clear is deliberate rather than a leftover.
                if for_stage != 0 {
                    for_stage = 0;
                }
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
            // ⚠ At the word's start and from the raw bytes, exactly as the
            // parser does it: whether the NAME is quoted is what decides it, and
            // a finished word cannot answer that.
            //
            // A binding is modelled now, so it is no longer a finding — but the
            // word it opens is not the command NAME either, and the scan has to
            // keep looking for one. Without that, `time PYTHONPATH=/x if` read
            // the value as the name and the `if` after it as an argument.
            if at_command_start && !in_word && opens_assignment(self.bytes, self.at) {
                word_is_binding = true;
            }
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
                    for (delimiter, strip_tabs) in std::mem::take(&mut heredocs) {
                        // A body that runs to the end of the text is what bash
                        // makes of a missing delimiter, and the parser reads it
                        // the same way — so there is nothing to report.
                        self.skip_heredoc_body(&delimiter, strip_tabs);
                    }
                }
                b';' => {
                    separator!();
                    // `;;`, `;&` and `;;&` end an arm's body, so what follows is
                    // the next arm's PATTERN — where a `(` opens nothing and a
                    // keyword is an ordinary word. Seen twice for `;;`, which is
                    // why the second visit finds the stage already moved.
                    if matches!(self.peek_at(1), Some(b';' | b'&'))
                        && let Some(stage @ CaseStage::Body) = cases.last_mut()
                    {
                        *stage = CaseStage::Pattern;
                    }
                    self.at += 1;
                }
                b'#' if !in_word => {
                    self.saw_comment = true;
                    // Anywhere inside a compound: the printer puts one on a
                    // single line, so a comment in there has nowhere to go and
                    // the parser refuses it.
                    if loop_header
                        || loop_depth > 0
                        || !if_stack.is_empty()
                        || parens > 0
                        || braces > 0
                    {
                        self.found.insert(Reason::CommentInList);
                    }
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
                    // ⚠ **A process substitution is a word SEGMENT, so neither
                    // rule below applies to it.** It is legal in a `for` header
                    // and legal glued into a word — `diff x<(a)` is one word —
                    // and testing it after those two claimed a redirection on
                    // every glued one.
                    let opens_process = self.peek_at(1) == Some(b'(');
                    // A redirection in a `for` header is a syntax error to bash,
                    // and the parser refuses the loop rather than the operator.
                    if loop_header && !opens_process {
                        self.found.insert(Reason::Loop);
                    }
                    // ⚠ **Glued into a word, it is not a redirection at all.**
                    // `awk 'NF>10'` unquoted puts a `>` in the middle of a word,
                    // which the parser refuses by that name; a descriptor is the
                    // one thing that may precede the operator, and it is digits.
                    if in_word && !opens_process && !word.chars().all(|c| c.is_ascii_digit()) {
                        self.found.insert(Reason::Redirection);
                    }
                    end_word!();
                    // ⚠ Four different constructs share these two characters,
                    // and only one of them is "a redirection to a file". Counted
                    // apart because they are not one build: a heredoc's operand
                    // is on the FOLLOWING lines, and a process substitution is a
                    // word whose value is a path the shell invents — modelled
                    // now, with its interior still scanned, because the parser
                    // reads it.
                    if opens_process {
                        self.process_substitution();
                    } else if byte == b'<'
                        && self.peek_at(1) == Some(b'<')
                        && self.peek_at(2) == Some(b'<')
                    {
                        // Modelled now: a redirection whose operand is a word on
                        // this line. Only the GLUED form is still a refusal, and
                        // the rule above has already reported it.
                        self.at += 3;
                    } else if byte == b'<' && self.peek_at(1) == Some(b'<') {
                        // Modelled now, so the heredoc itself is not a finding —
                        // only the delimiter has to be tracked, so the body can
                        // be stepped over rather than scanned as shell.
                        self.at += 2;
                        let strip_tabs = self.peek() == Some(b'-');
                        if strip_tabs {
                            self.at += 1;
                        }
                        // An unreadable delimiter leaves the body's extent
                        // unknown, so nothing is tracked and the body is scanned
                        // as shell — whatever made it unreadable is recorded by
                        // `take_delimiter` itself, so the invariant still holds.
                        if let Some(delimiter) = self.take_delimiter() {
                            heredocs.push((delimiter, strip_tabs));
                        }
                    } else {
                        // Every other `<`/`>` form is modelled now.
                        self.at += 1;
                    }
                }
                // ⚠ **Three constructs share these four characters and only one
                // of them is still unread.** `( … )`, `{ … ; }` and `name() { … }`
                // are modelled; a brace inside a WORD is expansion, which is
                // word-level work and a different build; and an unmatched one is
                // neither. Counting them together said "how many commands hold a
                // brace".
                // ⚠ **A subshell opens only where a COMMAND does.** `echo (` is
                // refused by the parser and by bash, and `at_command_start` is
                // the flag that says which `(` this is — "not inside a word" is
                // not the same test, and using it read every parenthesis in a
                // prose argument as a subshell.
                // ⚠ **The `(` in front of a case PATTERN opens nothing.** Bash
                // allows `(a) b;;` and prints it back as `a) b;;`, so counting
                // it as a subshell left the group unbalanced and reported a
                // grouping refusal on every case written that way.
                b'(' if !in_word && cases.last() == Some(&CaseStage::Pattern) => {
                    self.at += 1;
                }
                b'(' => {
                    // ⚠ `((…))` is arithmetic, not two subshells — `((a))`
                    // evaluates where `( (a) )` runs a command called `a`.
                    // Stepped over whole: the interior is an expression, so
                    // nothing in it is a construct this scanner reports, and a
                    // `<` in there is a comparison rather than a redirection.
                    // `for ((…))` is the C-style loop: the same doubled paren,
                    // one word past a command start, which `for_stage` is what
                    // knows about.
                    if (at_command_start || for_stage == 1)
                        && !in_word
                        && self.peek_at(1) == Some(b'(')
                    {
                        // Cleared before the separator rather than after: this
                        // `for` has no word list to be in the middle of, so the
                        // header state it set must not outlive the `((`.
                        loop_header = false;
                        separator!();
                        self.skip_balanced(b'(', b')');
                    } else if self.closes_immediately()
                        && (after_name || (at_command_start && in_word))
                    {
                        // `name()` — a definition. Its body is scanned as the
                        // list it is.
                        separator!();
                    } else if at_command_start && !in_word {
                        separator!();
                        parens += 1;
                        self.at += 1;
                    } else {
                        in_word = true;
                        self.found.insert(Reason::Grouping);
                        self.at += 1;
                    }
                }
                b')' if cases.last() == Some(&CaseStage::Pattern) => {
                    // ⚠ **A pattern is not a command position.** `in)` and `do)`
                    // are ordinary patterns to bash, so the word ending here
                    // must not be read as a command name — which is what would
                    // report a construct for one that spells a keyword.
                    at_command_start = false;
                    separator!();
                    // This closes an arm's pattern, which is the case grammar
                    // rather than a paren with a partner or without one.
                    if let Some(stage) = cases.last_mut() {
                        *stage = CaseStage::Body;
                    }
                    self.at += 1;
                }
                b')' => {
                    separator!();
                    if parens > 0 {
                        parens -= 1;
                    } else {
                        self.found.insert(Reason::Grouping);
                    }
                    self.at += 1;
                }

                // A `{` opens a group only where a word could not start, which
                // is bash's own rule: `{ a; }` is a group, `{a,b}` is a word.
                b'{' if at_command_start
                    && !in_word
                    && matches!(self.peek_at(1), Some(b' ' | b'\t' | b'\n' | b'\r')) =>
                {
                    separator!();
                    braces += 1;
                    self.at += 1;
                }
                b'}' if !in_word && braces > 0 => {
                    separator!();
                    braces -= 1;
                    self.at += 1;
                }
                // ⚠ Whether a `{` opens an expansion has ONE answer, and it
                // comes from the parser's own lookahead — see `brace_expansion`.
                // A brace with nothing to expand (`{a}`) is ordinary text to
                // bash and so to both readers.
                b'{' | b'}' => {
                    in_word = true;
                    word.push(byte as char);
                    match brace_expansion(self.bytes, self.at) {
                        Some(shape) => self.at = shape.end(),
                        None => self.at += 1,
                    }
                }
                b'$' | b'`' => {
                    in_word = true;
                    match classify_expansion(self.bytes, self.at, false) {
                        // Modelled now, so not a finding — but the word still
                        // has to record that something was here, or `if$x` would
                        // finish as the reserved word `if` and the survey would
                        // claim a construct the parser accepted.
                        Some(Reason::Parameter) => {
                            word.push('$');
                            self.skip_expansion();
                        }
                        // Modelled now. What is inside still has to be looked
                        // at, because the parser reads it — the two shapes it
                        // refuses in there are a heredoc and a comment.
                        Some(Reason::CommandSubstitution) => {
                            word.push('$');
                            self.substitution();
                        }
                        // Modelled now; the interior is arithmetic, which holds
                        // no construct this scanner reports on its own.
                        Some(Reason::Arithmetic) => self.skip_expansion(),
                        // Modelled now, and it resolves to a LITERAL — so the
                        // word has to record it, and the one escape the parser
                        // still refuses is reported from inside.
                        Some(Reason::AnsiQuote) => {
                            word.push('$');
                            word_quoted = true;
                            self.ansi_quote();
                        }
                        Some(reason) => {
                            self.found.insert(reason);
                            self.skip_expansion();
                        }
                        // An ordinary dollar sign, which the parser reads as
                        // literal text — so it is part of the word, not a find.
                        None => {
                            word.push('$');
                            self.at += 1;
                        }
                    }
                }
                b'~' if !in_word => {
                    // Modelled at the head of a word. The forms that are not —
                    // a quoted prefix, or a directory-stack entry like `~+2` —
                    // still are not.
                    in_word = true;
                    self.at += 1;
                    let from = self.at;
                    while self.peek().is_some_and(|b| b != b'/' && !is_stop(b)) {
                        self.at += 1;
                    }
                    let name = &self.text[from..self.at];
                    let modelled = matches!(name, "" | "+" | "-")
                        || name
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'));
                    if !modelled || matches!(self.peek(), Some(b'\'') | Some(b'"')) {
                        self.found.insert(Reason::Tilde);
                    }
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
                // ⚠ The same three answers the parser gets, from the same
                // function — a `[` that closes nothing is the test builtin or
                // ordinary text, one this reader cannot own is a finding, and a
                // set it can own is not.
                b'[' => {
                    in_word = true;
                    match bracket_expression(self.text, self.at) {
                        Bracket::Class(_, end) => self.at = end,
                        Bracket::Unread => {
                            self.found.insert(Reason::BracketExpression);
                            self.at += 1;
                        }
                        Bracket::Literal => {
                            word.push('[');
                            self.at += 1;
                        }
                    }
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
        // Direct, not `finish!` — nothing reads the word state after this, and
        // clearing it here is two dead stores the linter is right about.
        if in_word {
            // ⚠ The last word of a text is a keyword too, and it is the one that
            // matters most here: `fi` closes the chain, and a trailing `if`
            // opens one nothing can close. Missed while this path skipped the
            // bookkeeping, and `time PYTHONPATH=/x if` is the test.
            if at_command_start
                && !word_quoted
                && let Some(branch) = branch(&word)
            {
                conditional_keyword(&mut if_stack, &mut self.found, branch);
            }
            // ⚠ And the same for a `case`, whose closing keyword is the LAST
            // word of every command that holds one: `case $x in a) b;; esac`
            // ends on it, so skipping the bookkeeping here reported an unclosed
            // case for every well-formed one.
            let closes = at_command_start && !word_quoted && word == "esac";
            match cases.last_mut() {
                Some(stage @ CaseStage::Subject) => *stage = CaseStage::In,
                Some(stage @ CaseStage::In) => {
                    if word != "in" {
                        self.found.insert(Reason::Case);
                    }
                    *stage = CaseStage::Pattern;
                }
                // Whatever is left here is an open case's arms, which this
                // closes; an `esac` with nothing open closes nothing.
                Some(_) if closes => {
                    cases.pop();
                }
                None if closes => {
                    self.found.insert(Reason::Case);
                }
                _ => {}
            }
            finish_word(
                &mut self.found,
                &word,
                at_command_start,
                at_pipeline_head,
                word_quoted,
            );
        }
        // A group the text never closed, which the parser refuses by name.
        if parens > 0 || braces > 0 {
            self.found.insert(Reason::Grouping);
        }
        // An `if` the text never closed. The parser reaches the end looking for
        // a `fi` and refuses the conditional, so the set has to hold one.
        if !if_stack.is_empty() {
            self.found.insert(Reason::Conditional);
        }
        // A `case` the text never closed, for the same reason — and a header
        // that ran out before its `in`.
        if !cases.is_empty() {
            self.found.insert(Reason::Case);
        }
        self
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

    /// Is the `(` under the cursor immediately closed — the `()` of a
    /// definition rather than the opening of a subshell?
    fn closes_immediately(&self) -> bool {
        let mut at = self.at + 1;
        while self
            .bytes
            .get(at)
            .is_some_and(|b| matches!(b, b' ' | b'\t'))
        {
            at += 1;
        }
        self.bytes.get(at) == Some(&b')')
    }
    /// The word after `<<` with its quoting removed, or `None` where the
    /// delimiter is not readable and the body's extent is therefore unknown.
    ///
    /// The stop set is the parser's, not [`is_stop`]: `*`, `?`, `[` and the
    /// brace and paren characters are ordinary text in a delimiter, because
    /// nothing expands there.
    fn take_delimiter(&mut self) -> Option<String> {
        while matches!(self.peek(), Some(b' ') | Some(b'\t')) {
            self.at += 1;
        }
        let mut delimiter = String::new();
        let mut read_anything = false;
        while let Some(byte) = self.peek() {
            match byte {
                b' ' | b'\t' | b'\r' | b'\n' | b';' | b'|' | b'&' | b'<' | b'>' | b')' => break,
                b'$' | b'`' => match classify_expansion(self.bytes, self.at, false) {
                    Some(reason) => {
                        self.found.insert(reason);
                        self.skip_expansion();
                        return None;
                    }
                    None => {
                        read_anything = true;
                        delimiter.push('$');
                        self.at += 1;
                    }
                },
                b'\'' | b'"' => {
                    read_anything = true;
                    let from = self.at + 1;
                    let closed = if byte == b'\'' {
                        self.skip_single_quote()
                    } else {
                        self.skip_double_quote()
                    };
                    if !closed {
                        self.found.insert(Reason::UnterminatedQuote);
                        return None;
                    }
                    delimiter.push_str(&self.text[from..self.at - 1]);
                }
                b'\\' => {
                    read_anything = true;
                    self.at += 1;
                    if let Some(escaped) = self.peek() {
                        delimiter.push(escaped as char);
                        self.at += 1;
                    }
                }
                b => {
                    read_anything = true;
                    delimiter.push(b as char);
                    self.at += 1;
                }
            }
        }
        // `<<''` is a legal, empty delimiter; `<<` with nothing after it is not.
        if read_anything {
            return Some(delimiter);
        }
        self.found.insert(Reason::EmptyOperand);
        None
    }

    /// Step over a heredoc body, which is data rather than shell.
    ///
    /// ⚠ **The terminator is matched exactly, with no trimming of trailing
    /// whitespace.** Bash does not trim either — `EOF ` does not end a heredoc —
    /// and a survey that were lenient here would step over a line the parser
    /// keeps reading, which is the direction that breaks the invariant.
    fn skip_heredoc_body(&mut self, delimiter: &str, strip_tabs: bool) {
        while self.at < self.bytes.len() {
            let line_start = self.at;
            while self.peek().is_some_and(|b| b != b'\n') {
                self.at += 1;
            }
            let line = &self.text[line_start..self.at];
            if self.peek() == Some(b'\n') {
                self.at += 1;
            }
            let line = if strip_tabs {
                line.trim_start_matches('\t')
            } else {
                line
            };
            if line == delimiter {
                return;
            }
        }
    }

    /// Step over `$( … )`, reporting what the parser will refuse inside one.
    ///
    /// ⚠ **Descended into**, because a construct in there is one the parser will
    /// meet and refuse — a backtick inside a substitution is still a backtick —
    /// so looking past it would miss exactly the refusals the invariant exists
    /// to catch. Guessing from the raw text instead reported a comment for every
    /// `#` in a path or a flag: 326 commands the parser reads perfectly well.
    ///
    /// One shape in there is refused for *where* it is rather than for what it
    /// is: a comment, which an inline print has nowhere to put. A heredoc used
    /// to be the other, and is not any more — the printer gives it the lines it
    /// needs.
    fn substitution(&mut self) {
        self.at += 1;
        let open = self.at;
        if !self.skip_balanced(b'(', b')') {
            // ⚠ The parser refuses this by that name, so the survey has to say
            // it or the invariant that pins the two together fails. The shape it
            // most often takes is a heredoc body that runs past the `)` and
            // swallows it — which bash refuses too.
            self.found.insert(Reason::UnterminatedExpansion);
        }
        // The text between the parens, which is a script in its own right.
        let interior = self
            .text
            .get(open + 1..self.at.saturating_sub(1))
            .unwrap_or_default();
        let inner = scan(interior);
        self.found.extend(inner.found);
        if inner.saw_comment {
            self.found.insert(Reason::CommentInList);
        }
    }

    /// Step over `<( … )` or `>( … )`, whose interior is a command list too.
    ///
    /// ⚠ **The whole run, not the two opening characters.** Stepping over `<(`
    /// alone left its `)` to be read as an unmatched paren, so every process
    /// substitution reported `Grouping` as well — an over-report the invariant
    /// cannot see, since it only pins that the parser's refusal is *in* the set.
    /// The cost was the figure the survey exists to produce: 213 commands sat in
    /// "needs 2 constructs" and the construct itself never appeared on the
    /// "build one and this many become readable" list at all.
    fn process_substitution(&mut self) {
        self.at += 1; // the `<` or `>`
        let open = self.at;
        if !self.skip_balanced(b'(', b')') {
            self.found.insert(Reason::UnterminatedExpansion);
        }
        let interior = self
            .text
            .get(open + 1..self.at.saturating_sub(1))
            .unwrap_or_default();
        // Descended into for the reason a substitution's interior is: what is in
        // there has to be built before this command can be read.
        let inner = scan(interior);
        self.found.extend(inner.found);
        if inner.saw_comment {
            self.found.insert(Reason::CommentInList);
        }
    }

    /// Step over `$'…'`, reporting the escapes the parser still refuses.
    ///
    /// ⚠ **`\'` does not close it**, so a naive single-quote skip ends the word
    /// in the middle of one — and the two readers then disagree about where
    /// everything after it is.
    fn ansi_quote(&mut self) {
        self.at += 2; // `$'`
        while let Some(byte) = self.peek() {
            match byte {
                b'\'' => {
                    self.at += 1;
                    return;
                }
                b'\\' => {
                    // `\u` and `\U` are refused rather than decoded, and a NUL
                    // because bash carries none; every other escape resolves to
                    // a character the tree holds.
                    if matches!(self.peek_at(1), Some(b'u' | b'U')) || self.is_nul_escape() {
                        self.found.insert(Reason::AnsiQuote);
                    }
                    self.at += 2;
                }
                _ => self.at += 1,
            }
        }
        // The text ended inside it, which the parser names.
        self.found.insert(Reason::UnterminatedQuote);
    }

    /// Does the escape under the cursor spell a NUL — `\0`, `\x0`, `\000`?
    ///
    /// The parser refuses one, so the survey has to see it. Read from the digits
    /// rather than by decoding: every spelling of zero is zero.
    fn is_nul_escape(&self) -> bool {
        let (radix, from) = match self.peek_at(1) {
            Some(b'x') => (16u32, self.at + 2),
            Some(b'0'..=b'7') => (8, self.at + 1),
            _ => return false,
        };
        let most = if radix == 16 { 2 } else { 3 };
        let digits: Vec<u8> = self.bytes[from..]
            .iter()
            .take(most)
            .copied()
            .take_while(|byte| (*byte as char).is_digit(radix))
            .collect();
        !digits.is_empty() && digits.iter().all(|byte| *byte == b'0')
    }

    /// Is `word` sitting at the cursor, with a shell boundary on both sides?
    fn at_word(&self, word: &str) -> bool {
        if self.at > 0 && !is_stop(self.bytes[self.at - 1]) {
            return false;
        }
        let end = self.at + word.len();
        self.text.get(self.at..end) == Some(word)
            && self.bytes.get(end).is_none_or(|byte| is_stop(*byte))
    }

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

    /// Step over a nested, quote-aware bracketed run, and say whether it closed.
    ///
    /// Quote-aware because `$(echo ")")` is one substitution and a naive counter
    /// ends it early — and heredoc-aware for the same reason, twice over: a body
    /// is prose, so both `)` and a lone apostrophe are ordinary characters in
    /// one. Without that, `$(git commit -m "$(cat <<'EOF' … EOF)")` ended at the
    /// first `)` a commit message happened to hold, and the rest of the message
    /// was scanned as shell — 182 commands reporting constructs nobody wrote.
    ///
    /// And `case`-aware for a third: an arm's `)` closes nothing, so
    /// `$(case $y in a) echo b;; esac)` ends at the LAST paren rather than the
    /// first. Only the keywords are tracked here — where the arm boundaries are
    /// is [`Survey::run`]'s business, and this only needs to know not to stop.
    fn skip_balanced(&mut self, open: u8, close: u8) -> bool {
        let mut depth = 0usize;
        // Openers waiting for the newline that starts their bodies.
        let mut heredocs: Vec<(String, bool)> = Vec::new();
        let mut cases = 0usize;
        while let Some(byte) = self.peek() {
            match byte {
                b'c' if self.at_word("case") => {
                    cases += 1;
                    self.at += 4;
                }
                b'e' if self.at_word("esac") => {
                    cases = cases.saturating_sub(1);
                    self.at += 4;
                }
                b if b == open => {
                    depth += 1;
                    self.at += 1;
                }
                // A pattern's terminator, not this run's: an arm leaves its `)`
                // unmatched, so the one that would close here is that instead.
                b if b == close && cases > 0 && depth == 1 => self.at += 1,
                b if b == close => {
                    depth -= 1;
                    self.at += 1;
                    if depth == 0 {
                        return true;
                    }
                }
                b'\n' => {
                    self.at += 1;
                    for (delimiter, strip_tabs) in std::mem::take(&mut heredocs) {
                        self.skip_heredoc_body(&delimiter, strip_tabs);
                    }
                }
                // `<<` and not `<<<`, which is a here-string with its operand on
                // the line.
                b'<' if self.peek_at(1) == Some(b'<') && self.peek_at(2) != Some(b'<') => {
                    self.at += 2;
                    let strip_tabs = self.peek() == Some(b'-');
                    if strip_tabs {
                        self.at += 1;
                    }
                    if let Some(delimiter) = self.take_delimiter() {
                        heredocs.push((delimiter, strip_tabs));
                    }
                }
                b'\'' => {
                    if !self.skip_single_quote() {
                        return false;
                    }
                }
                b'"' => {
                    if !self.skip_double_quote() {
                        return false;
                    }
                }
                b'\\' => self.at += 2,
                _ => self.at += 1,
            }
        }
        false
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
                b'$' | b'`' => match classify_expansion(self.bytes, self.at, true) {
                    // Modelled: a parameter, a substitution, and arithmetic —
                    // whose interior is an expression rather than a list, so
                    // there is nothing in there for this scanner to report.
                    Some(Reason::Parameter | Reason::Arithmetic) => self.skip_expansion(),
                    Some(Reason::CommandSubstitution) => self.substitution(),
                    Some(reason) => {
                        self.found.insert(reason);
                        self.skip_expansion();
                    }
                    None => self.at += 1,
                },
                _ => self.at += 1,
            }
        }
        false
    }
}

/// How far through `case word in` an open case has got.
///
/// ⚠ **Three words, and two of them can be wrong** — a case with no subject and
/// one that never says `in` are both syntax errors bash refuses, and the parser
/// names each of them `Case`. Nothing else in this scan can see either.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaseStage {
    /// After `case`, waiting for the word to match on.
    Subject,
    /// After the subject, waiting for `in`.
    In,
    /// Where an arm's patterns are read — after `in`, and after every arm
    /// terminator. A `(` here is the optional one bash allows in front of a
    /// pattern rather than a subshell, and the `)` after it closes no group.
    Pattern,
    /// An arm's body, which is an ordinary command list: a `(` in here really
    /// does open a subshell.
    Body,
}

/// A keyword of an `if` chain — a closed set, so not a string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Branch {
    If,
    Then,
    Elif,
    Else,
    Fi,
}

/// The boundary where a word becomes one of them, and the only place the
/// spelling is read.
fn branch(word: &str) -> Option<Branch> {
    Some(match word {
        "if" => Branch::If,
        "then" => Branch::Then,
        "elif" => Branch::Elif,
        "else" => Branch::Else,
        "fi" => Branch::Fi,
        _ => return None,
    })
}

/// Where the `if` chain stands, one entry per open `if` — `true` while that arm
/// is still waiting for the `then` that has to follow it.
///
/// ⚠ **This is the whole of what the survey knows about conditionals**, and it
/// exists because every shape it reports is one bash refuses: `if a then b; fi`
/// never says `then` where a command begins, `fi` on its own closes nothing,
/// `if a; then b` leaves the chain open, and `if a; else b; fi` branches before
/// it has tested anything. The parser refuses each of them by name, so the
/// survey's set has to hold a `Conditional` for each.
///
/// A depth counter is not enough: it balances on `if a then b; fi`, where the
/// `then` is an argument to `a` rather than the keyword.
fn conditional_keyword(stack: &mut Vec<bool>, found: &mut BTreeSet<Reason>, branch: Branch) {
    match branch {
        Branch::If => stack.push(true),
        Branch::Then => match stack.last_mut() {
            Some(waiting) => *waiting = false,
            None => {
                found.insert(Reason::Conditional);
            }
        },
        // `elif` re-opens the wait — it has a `then` of its own — and `else`
        // ends it. Either one before any `then` is a shape bash refuses.
        Branch::Elif | Branch::Else => match stack.last_mut() {
            Some(waiting) if *waiting => {
                found.insert(Reason::Conditional);
            }
            Some(waiting) => *waiting = branch == Branch::Elif,
            None => {
                found.insert(Reason::Conditional);
            }
        },
        Branch::Fi => match stack.pop() {
            // A chain closes only where it reached its `then`.
            Some(false) => {}
            Some(true) | None => {
                found.insert(Reason::Conditional);
            }
        },
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
    if at_command_start
        && !quoted
        && let Some(reason) = reserved_word(word)
        // ⚠ The loop, conditional and case keywords are modelled now, so they
        // are not findings on their own — the caller reports the shapes of those
        // the parser still refuses. They stay in `reserved_word`, because the
        // printer has to quote a word that spells one to keep it a value.
        && !matches!(reason, Reason::Loop | Reason::Conditional | Reason::Case)
    {
        found.insert(reason);
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
