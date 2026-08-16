//! Tree to text, reading nothing but the tree.
//!
//! ⚠ **The printer may not look at the source.** Condition (2) of the round-trip
//! law — that the generated form is a fixpoint — follows from condition (1) only
//! when `G` is a pure function of the tree, and the usual way to break it is a
//! printer that reaches back for the original spelling of a token. There is no
//! `&str` of source in this module's signatures, which is the enforcement.
//!
//! The form is canonical: one command per line, one space between words, and the
//! least quoting that reads back as the same tree. Two commands that differ only
//! in layout or quoting print identically, which is what makes the printed form
//! usable as an equivalence test.

use super::ast::{
    Anchor, AndOr, Assignment, Command, CommandKind, Conditional, Connector, ForLoop, Glob,
    Heredoc, Item, Parameter, ParameterOp, Pipeline, Redirect, RedirectOp, RedirectTarget, Script,
    Segment, SegmentKind, Simple, Subscript, Tilde, Timed, WhileLoop, Word,
};
use super::parse::{is_assignment, is_reserved};

pub fn print(script: &Script) -> String {
    let mut lines: Vec<String> = Vec::with_capacity(script.items.len());
    for item in &script.items {
        match item {
            Item::Comment(comment) => lines.push(format!("#{}", comment.text)),
            Item::List(list) => {
                // ⚠ **A heredoc body goes after the line, not after its
                // operator.** The printer collects them for the whole list —
                // `cat <<A | cat <<B` opens two on one line — and emits them in
                // the order the openers were written, which is the order bash
                // reads them back in.
                let mut bodies: Vec<String> = Vec::new();
                lines.push(print_and_or(list, &mut bodies));
                lines.extend(bodies);
            }
        }
    }
    lines.join("\n")
}

/// `a && b || c &` — connectors inline, `&` last.
///
/// One line, unlike `;`-separated lists which become one item each. That is
/// bash's own split: `declare -f` keeps `a && b` together and breaks `a; b`
/// apart, so following it keeps the printed form comparable with bash's.
fn print_and_or(list: &AndOr, bodies: &mut Vec<String>) -> String {
    let mut out = print_pipeline(&list.first, bodies);
    for link in &list.rest {
        out.push_str(match link.connector {
            Connector::And => " && ",
            Connector::Or => " || ",
        });
        out.push_str(&print_pipeline(&link.pipeline, bodies));
    }
    if list.background {
        out.push_str(" &");
    }
    out
}

/// ⚠ **`time` before `!`, whichever order they were written in.** That is the
/// order bash's own printer emits, and matching it is what lets the second gate
/// compare trees rather than argue about spelling.
fn print_pipeline(pipeline: &Pipeline, bodies: &mut Vec<String>) -> String {
    let mut parts: Vec<String> = Vec::new();
    match pipeline.time {
        Some(Timed::Plain) => parts.push("time".into()),
        Some(Timed::Posix) => parts.push("time -p".into()),
        None => {}
    }
    if pipeline.negated {
        parts.push("!".into());
    }
    let commands: Vec<String> = pipeline
        .commands
        .iter()
        .enumerate()
        .map(|(index, command)| print_command(command, index == 0, bodies))
        .collect();
    parts.push(commands.join(" | "));
    parts.retain(|part| !part.is_empty());
    parts.join(" ")
}

/// `head` is whether this command opens the pipeline — the only place `time`
/// and `!` are grammar, and therefore the only place a word spelling one has to
/// be quoted to stay a value.
fn print_command(command: &Command, head: bool, bodies: &mut Vec<String>) -> String {
    // ⚠ Words first, then redirections — bash's own order. `> out cat f` comes
    // back from `declare -f` as `cat f > out`, so putting them anywhere else
    // would be a spelling bash does not use and the tree does not record.
    let mut parts: Vec<String> = match &command.kind {
        CommandKind::Simple(simple) => print_simple(simple, head),
        CommandKind::For(loop_) => vec![print_for(loop_, bodies)],
        CommandKind::While(loop_) => vec![print_while(loop_, bodies)],
        CommandKind::If(conditional) => vec![print_if(conditional, bodies)],
        // ⚠ A subshell needs no terminator before its `)`, and a brace group
        // REQUIRES one before its `}` — `{ a }` is a syntax error where `( a )`
        // is not. Measured; `follow` supplies the right separator, including
        // none at all after a `&`.
        CommandKind::Subshell(items) => vec![format!("( {} )", print_body(items, bodies))],
        CommandKind::Group(items) => {
            vec![format!("{{ {} }}", terminated(&print_body(items, bodies)))]
        }
        // ⚠ **`function f ()` is bash's spelling, not ours.** `declare -f`
        // prints every definition that way whichever was written, so matching it
        // is what makes `f() { a; }` and `function f { a; }` one tree.
        CommandKind::Function(function) => vec![format!(
            "function {} () {{ {} }}",
            function.name,
            terminated(&print_body(&function.body, bodies))
        )],
    };
    parts.extend(
        command
            .redirects
            .iter()
            .map(|redirect| print_redirect(redirect, bodies)),
    );
    parts.join(" ")
}

/// `FOO=bar`, `FOO+=bar`, `FOO=` — the value spelled as a value.
///
/// An empty value is written as nothing at all rather than as `''`, which is
/// what the parser reads back: the two spellings bind the same thing and are one
/// tree.
fn print_assignment(assignment: &Assignment) -> String {
    let equals = if assignment.append { "+=" } else { "=" };
    let value = if assignment.value.segments.is_empty() {
        String::new()
    } else {
        print_word(&assignment.value, false)
    };
    format!("{}{equals}{value}", assignment.name)
}

/// `for f in a b; do x; y; done` — on ONE line, deliberately.
///
/// ⚠ **A compound cannot be printed across lines while heredoc bodies are
/// collected per line.** A body has to follow the line its `<<` was written on,
/// and `bodies` is gathered for the whole and-or; breaking the loop over several
/// lines would put the body after `done` instead. One line keeps both true, and
/// bash reads `for f in a; do cat <<EOF; done` followed by the body exactly as
/// it reads the multi-line spelling.
fn print_for(loop_: &ForLoop, bodies: &mut Vec<String>) -> String {
    let opener = if loop_.select { "select" } else { "for" };
    let words: Vec<String> = loop_
        .words
        .iter()
        .map(|word| print_word(word, false))
        .collect();
    let head = format!("{opener} {} in {}", loop_.name, words.join(" "));
    let with_do = follow(&head, "do");
    follow(
        &format!("{with_do} {}", print_body(&loop_.body, bodies)),
        "done",
    )
}

fn print_while(loop_: &WhileLoop, bodies: &mut Vec<String>) -> String {
    let opener = if loop_.until { "until" } else { "while" };
    let head = follow(
        &format!("{opener} {}", print_body(&loop_.condition, bodies)),
        "do",
    );
    follow(
        &format!("{head} {}", print_body(&loop_.body, bodies)),
        "done",
    )
}

/// `if cond; then body; else body; fi` — on one line, like the loops and for the
/// same reason: a heredoc body has to follow the line its `<<` was written on.
///
/// ⚠ **There is no `elif` to print.** The tree does not hold one — see
/// [`Conditional`] — so a chain comes out as the nested `else if …; fi; fi` bash
/// itself prints, which is what makes the two spellings compare equal.
fn print_if(conditional: &Conditional, bodies: &mut Vec<String>) -> String {
    let head = follow(
        &format!("if {}", print_body(&conditional.condition, bodies)),
        "then",
    );
    let mut out = format!("{head} {}", print_body(&conditional.then, bodies));
    if let Some(otherwise) = &conditional.otherwise {
        out = format!("{} {}", follow(&out, "else"), print_body(otherwise, bodies));
    }
    follow(&out, "fi")
}

/// Put a keyword after a command list, with the separator the list has earned.
///
/// ⚠ **A `&` already terminates its list, so no `;` may follow it.** Bash
/// accepts `if a; then b & fi` and refuses `if a; then b & ; fi` — measured —
/// and the same is true of every `do … done`. Shared rather than repeated,
/// because it was written out three times and got the loops wrong.
/// A command list with the terminator a closing `}` needs after it.
///
/// ⚠ **`{ a }` is a syntax error and `( a )` is not** — measured. A brace group
/// is a reserved word, so its last command has to be ended before the `}` can be
/// read as one. The exception is the same as everywhere else: a `&` has already
/// ended the list, and `{ a & ; }` is refused in turn.
fn terminated(list: &str) -> String {
    if list.ends_with('&') {
        list.to_string()
    } else {
        format!("{list};")
    }
}

fn follow(list: &str, keyword: &str) -> String {
    let separator = if list.ends_with('&') { " " } else { "; " };
    format!("{list}{separator}{keyword}")
}

/// A command list on one line.
///
/// ⚠ **A `&` already terminates its list, so no `;` may follow it.** `b & ; c`
/// is a syntax error where `b & c` is not, which is why the separator is chosen
/// from what came before rather than fixed.
fn print_body(items: &[Item], bodies: &mut Vec<String>) -> String {
    let mut out = String::new();
    for item in items {
        let text = match item {
            // Refused by the parser, so unreachable — and printed as a comment
            // would swallow the rest of the line if it ever were not.
            Item::Comment(comment) => format!("#{}", comment.text),
            Item::List(list) => print_and_or(list, bodies),
        };
        if !out.is_empty() {
            out.push_str(if out.ends_with('&') { " " } else { "; " });
        }
        out.push_str(&text);
    }
    out
}

/// ⚠ Assignments, then words — bash's own order, and it says so structurally:
/// `FOO=bar > out cmd` comes back from `declare -f` as `FOO=bar cmd > out`.
fn print_simple(simple: &Simple, head: bool) -> Vec<String> {
    let mut parts: Vec<String> = simple.assignments.iter().map(print_assignment).collect();
    parts.extend(
        simple
            .words
            .iter()
            .enumerate()
            .map(|(index, word)| print_word(word, index == 0 && head)),
    );
    parts
}

fn print_redirect(redirect: &Redirect, bodies: &mut Vec<String>) -> String {
    // The descriptor is written only when it is not the operator's own default,
    // which is how `>` stays `>` and `2>` stays `2>`.
    let fd = match redirect.fd {
        Some(fd) if Some(fd) != redirect.op.default_fd() => fd.to_string(),
        _ => String::new(),
    };
    let op = match redirect.op {
        RedirectOp::Read => "<",
        RedirectOp::Write => ">",
        RedirectOp::Append => ">>",
        RedirectOp::ReadWrite => "<>",
        RedirectOp::Clobber => ">|",
        RedirectOp::DupOut => ">&",
        RedirectOp::DupIn => "<&",
        RedirectOp::Both => "&>",
        RedirectOp::BothAppend => "&>>",
        RedirectOp::BothWord => ">&",
        RedirectOp::Here => "<<",
        RedirectOp::HereDash => "<<-",
    };
    match &redirect.target {
        // No space after a dup operator: `2>&1`, not `2>& 1`.
        RedirectTarget::Fd(target) => format!("{fd}{op}{target}"),
        RedirectTarget::Close => format!("{fd}{op}-"),
        RedirectTarget::File(word) => format!("{fd}{op} {}", print_word(word, false)),
        RedirectTarget::Here(here) => {
            bodies.push(format!("{}{}", here.body, here.delimiter));
            format!("{fd}{op}{}", print_delimiter(here))
        }
    }
}

/// The delimiter, spelled so it reads back with the same `quoted` bit.
///
/// ⚠ **Single quotes whenever it was quoted at all, which is bash's own
/// spelling.** `declare -f` prints `<<"EOF"`, `<<\EOF` and `<<E"O"F` all as
/// `<<'EOF'`, so a printer that chose differently would be the only thing in the
/// comparison saying those four texts are not one tree.
///
/// An unquoted delimiter goes out bare whatever is in it: nothing expands there,
/// so a `*` is an asterisk, and quoting it would flip the bit.
fn print_delimiter(here: &Heredoc) -> String {
    if here.quoted {
        quote(&here.delimiter)
    } else {
        here.delimiter.clone()
    }
}

/// One word. `first` is whether it opens the pipeline's head command, the only
/// position where the shell reads a word as grammar rather than as a value.
pub fn print_word(word: &Word, first: bool) -> String {
    // ⚠ **A word that would read back as grammar is quoted whole.** `time` at
    // the head of a command is a keyword and `FOO=bar` is a binding, so printing
    // either bare turns a value the tree holds into something the parser would
    // refuse — and a refusal on `t₂` is a round-trip failure. Quoting is what
    // says "this really is the name of a program", which is exactly what the
    // shell means by it.
    // ⚠ `time` is checked by name because it is NOT in `RESERVED` — it is
    // grammar only here, at a pipeline's head, and a plain program name after a
    // `|`. Printing this tree's `time` bare would turn a command into a keyword.
    if first
        && let Some(text) = word.as_literal()
        && (is_reserved(&text) || is_assignment(&text) || text == "time")
    {
        return quote(&text);
    }
    if word.segments.is_empty() {
        return "''".to_string();
    }
    // ⚠ **A quote may not touch a tilde prefix.** `~'/x'` is the literal `~/x`
    // to bash, not a home directory, so quoting the segment after a tilde says
    // something the tree does not. Found by the round-trip law on 319 commands
    // — `cat ~/.config/…/Local\ State`, where the space forced a quote right
    // where the prefix ends.
    // ⚠ **A bracket expression is a property of the WHOLE word, so the decision
    // to quote cannot be made one segment at a time.** `"[rc=$?]"` holds the
    // literal `[rc=`, which needs no quoting by itself, and the literal `]`,
    // which needs none either — printed bare they compose into `[rc="$?"]`, and
    // that reads back as a bracket expression rather than as this word. Found by
    // the round-trip law on 2 commands. So a literal holding a `[` is quoted
    // whenever a LATER literal holds the `]` that would close it.
    let closed_later = closing_brackets_after(word);
    let mut out = String::new();
    let mut after_tilde = false;
    for (index, segment) in word.segments.iter().enumerate() {
        match (&segment.kind, after_tilde) {
            (SegmentKind::Literal(text), true) => out.push_str(&print_after_tilde(text)),
            (SegmentKind::Literal(text), false) if text.contains('[') && closed_later[index] => {
                out.push_str(&quote(text));
            }
            // ⚠ A parameter is the one segment whose spelling depends on what
            // FOLLOWS it: `$x` beside the literal `y` has to be written `${x}y`
            // or the name reads as `xy`.
            (SegmentKind::Parameter(parameter), _) => {
                out.push_str(&print_parameter(parameter, word.segments.get(index + 1)));
            }
            _ => out.push_str(&print_segment(segment)),
        }
        after_tilde = matches!(segment.kind, SegmentKind::Tilde(_));
    }
    out
}

/// For each segment, does a `]` appear in a literal after it?
///
/// Only a literal can carry one: a glob, a tilde and a parameter each print as
/// characters the bracket reader stops at or steps over, and none of them can
/// contribute the `]`.
fn closing_brackets_after(word: &Word) -> Vec<bool> {
    let mut flags = vec![false; word.segments.len()];
    let mut seen = false;
    for (index, segment) in word.segments.iter().enumerate().rev() {
        flags[index] = seen;
        if let SegmentKind::Literal(text) = &segment.kind
            && text.contains(']')
        {
            seen = true;
        }
    }
    flags
}

/// `$x`, `${x}`, `"$x"` — the least spelling that reads back as this node.
fn print_parameter(parameter: &Parameter, next: Option<&Segment>) -> String {
    // ⚠ A subscript or an operator forces the braces, whatever follows: `$a[0]`
    // is the value of `a` beside the literal `[0]`, an entirely different word.
    let bare = if parameter.subscript.is_some() || parameter.op.is_some() {
        format!(
            "${{{}{}{}}}",
            print_prefix_op(parameter.op.as_ref()),
            parameter.name,
            print_subscript(parameter.subscript.as_ref()) + &print_suffix_op(parameter.op.as_ref())
        )
    } else if needs_braces(&parameter.name, next) {
        format!("${{{}}}", parameter.name)
    } else {
        format!("${}", parameter.name)
    };
    // ⚠ The quotes are the node, not decoration: without them the value would
    // be split into words and globbed, which is a different command.
    if parameter.quoted {
        format!("\"{bare}\"")
    } else {
        bare
    }
}

/// `[0]`, `[@]` — where the node names an array element.
fn print_subscript(subscript: Option<&Subscript>) -> String {
    match subscript {
        None => String::new(),
        Some(Subscript::All) => "[@]".to_string(),
        Some(Subscript::Joined) => "[*]".to_string(),
        Some(Subscript::Index(word)) => format!("[{}]", print_operand(word)),
    }
}

/// The two operators bash writes BEFORE the name.
fn print_prefix_op(op: Option<&ParameterOp>) -> &'static str {
    match op {
        Some(ParameterOp::Length) => "#",
        Some(ParameterOp::Indirect) => "!",
        _ => "",
    }
}

/// Everything else, which is written after the name and its subscript.
fn print_suffix_op(op: Option<&ParameterOp>) -> String {
    let colon = |c: bool| if c { ":" } else { "" };
    match op {
        None | Some(ParameterOp::Length | ParameterOp::Indirect) => String::new(),
        Some(ParameterOp::Default { colon: c, word }) => {
            format!("{}-{}", colon(*c), print_operand(word))
        }
        Some(ParameterOp::Assign { colon: c, word }) => {
            format!("{}={}", colon(*c), print_operand(word))
        }
        Some(ParameterOp::Error { colon: c, word }) => {
            format!("{}?{}", colon(*c), print_operand(word))
        }
        Some(ParameterOp::Alternate { colon: c, word }) => {
            format!("{}+{}", colon(*c), print_operand(word))
        }
        Some(ParameterOp::StripPrefix { longest, pattern }) => {
            format!(
                "{}{}",
                if *longest { "##" } else { "#" },
                print_operand(pattern)
            )
        }
        Some(ParameterOp::StripSuffix { longest, pattern }) => {
            format!(
                "{}{}",
                if *longest { "%%" } else { "%" },
                print_operand(pattern)
            )
        }
        Some(ParameterOp::Case { upper, every }) => {
            let c = if *upper { '^' } else { ',' };
            if *every {
                format!("{c}{c}")
            } else {
                c.to_string()
            }
        }
        Some(ParameterOp::Replace(replace)) => {
            let anchor = match replace.anchor {
                Some(Anchor::Start) => "#",
                Some(Anchor::End) => "%",
                None => "",
            };
            let tail = match &replace.replacement {
                Some(word) => format!("/{}", print_operand(word)),
                None => String::new(),
            };
            format!(
                "{}{anchor}{}{tail}",
                if replace.every { "//" } else { "/" },
                print_operand(&replace.pattern),
            )
        }
    }
}

/// A word inside `${…}`, where quoting works differently from a word outside.
///
/// ⚠ **No quoting is added.** The braces already delimit it — `${x:-a b}` is one
/// word to bash with the space bare — and a quote here would be read back as
/// part of the value. What must still be escaped is the handful of characters
/// that would end the expansion or change its operator.
fn print_operand(word: &Word) -> String {
    let mut out = String::new();
    for segment in &word.segments {
        match &segment.kind {
            SegmentKind::Literal(text) => {
                for c in text.chars() {
                    if matches!(c, '}' | '{' | '$' | '`' | '\\' | '"' | '\'' | '/') {
                        out.push('\\');
                    }
                    out.push(c);
                }
            }
            _ => out.push_str(&print_segment(segment)),
        }
    }
    out
}

/// Would the name run on into what comes next, or is it unspellable bare?
fn needs_braces(name: &str, next: Option<&Segment>) -> bool {
    // A special parameter is one character that cannot start a name, so nothing
    // can extend it: `$@abc` is `$@` and then `abc`.
    let special =
        name.len() == 1 && !name.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_');
    if special {
        return false;
    }
    // Only `${10}` names the tenth positional; `$10` is `$1` and a `0`.
    if name.len() > 1 && name.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }
    // Only a literal can extend a name — a glob, a tilde or another parameter
    // all start with a character a name cannot hold.
    matches!(
        next.map(|segment| &segment.kind),
        Some(SegmentKind::Literal(text))
            if text.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
    )
}

/// A literal that follows a tilde prefix, spelled so the prefix still ends where
/// it did.
///
/// The prefix runs to the first *unquoted* `/`, so letting that slash through
/// bare is enough to close it — after which ordinary quoting is safe again.
/// Without a leading slash there is nothing to close it with, and the only way
/// such a literal can arise is a backslash escape (`~a\ b`), so backslashes are
/// what put it back.
fn print_after_tilde(text: &str) -> String {
    if !needs_quoting(text) {
        return text.to_string();
    }
    if let Some(rest) = text.strip_prefix('/') {
        return format!("/{}", print_segment_text(rest));
    }
    text.chars()
        .map(|c| {
            if is_bare_safe(c) {
                c.to_string()
            } else {
                format!("\\{c}")
            }
        })
        .collect()
}

fn print_segment_text(text: &str) -> String {
    if needs_quoting(text) {
        quote(text)
    } else {
        text.to_string()
    }
}

fn print_segment(segment: &Segment) -> String {
    match &segment.kind {
        SegmentKind::Glob(Glob::Any) => "*".to_string(),
        SegmentKind::Glob(Glob::One) => "?".to_string(),
        SegmentKind::Tilde(Tilde::Home) => "~".to_string(),
        SegmentKind::Tilde(Tilde::Pwd) => "~+".to_string(),
        SegmentKind::Tilde(Tilde::OldPwd) => "~-".to_string(),
        SegmentKind::Tilde(Tilde::User(name)) => format!("~{name}"),
        SegmentKind::Literal(text) => {
            if needs_quoting(text) {
                quote(text)
            } else {
                text.clone()
            }
        }
        // Reached only where there is nothing after it to run into; `print_word`
        // handles the general case.
        SegmentKind::Parameter(parameter) => print_parameter(parameter, None),
        SegmentKind::Substitution(substitution) => {
            // ⚠ A substitution's own heredocs are refused by the parser, so
            // nothing can reach this vector — and if that ever changed, the body
            // would have to go somewhere this inline form has no room for.
            let mut inner = Vec::new();
            let body = print_body(&substitution.items, &mut inner);
            // ⚠ **`$((` is arithmetic, so a substitution holding a subshell
            // needs the space bash needs.** `$( (cd x) && y )` written without
            // it opens an arithmetic expansion instead — for bash as well as for
            // this parser, which is how the round-trip law caught it on 9
            // commands the moment grouping made the shape reachable.
            let text = if body.starts_with('(') {
                format!("$( {body})")
            } else {
                format!("$({body})")
            };
            if substitution.quoted {
                format!("\"{text}\"")
            } else {
                text
            }
        }
    }
}

/// Single quotes: a word in the tree is a *value*, and any spelling that could
/// expand would be a different claim about it.
fn quote(text: &str) -> String {
    if text.is_empty() {
        return "''".to_string();
    }
    // The one character single quotes cannot hold: close, escape it, reopen.
    format!("'{}'", text.replace('\'', r"'\''"))
}

fn needs_quoting(text: &str) -> bool {
    if text.is_empty() {
        return true;
    }
    if text.chars().any(|c| !is_bare_safe(c)) {
        return true;
    }
    // `[` and `]` are ordinary characters until they are both there, and then
    // they are a bracket expression. Keeping them bare in isolation is what lets
    // `[ -f x ]` — the corpus's commonest conditional — print as it was written.
    match text.find('[') {
        Some(open) => text[open..].contains(']'),
        None => false,
    }
}

/// Characters a bare word may hold without the shell reading anything into them.
///
/// Deliberately narrow. `~` expands at the head of a word, `#` opens a comment
/// there, `!` is history, and each is cheaper to quote everywhere than to reason
/// about by position — over-quoting costs a character and reads back the same,
/// while under-quoting is a wrong tree.
fn is_bare_safe(c: char) -> bool {
    c.is_ascii_alphanumeric()
        || matches!(
            c,
            '_' | '-' | '.' | '/' | ':' | ',' | '+' | '@' | '%' | '=' | '[' | ']'
        )
}
