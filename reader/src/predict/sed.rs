//! `sed -i`, where sed's regular expressions and Rust's mean the same thing.
//!
//! The script is parsed as sed parses it — commands split on newlines and `;`
//! outside a delimited pattern — and only `s` is followed, under a line-number,
//! `$` or `/re/` address or a range of them. A basic regular expression is
//! translated, not copied: `\(`, `\)`, `\{`, `\}`, `\+` and `\?` are the
//! operators and the bare characters are literals, the reverse of Rust's, and
//! `-E` turns that around; inside a bracket expression a backslash is a
//! character, which Rust would read as an escape.
//!
//! **A match is the longest, by construction.** POSIX picks the leftmost
//! match and then the longest; Rust's engine picks the leftmost and then the
//! first its pattern reaches, so `\(a*\)\(ab\)*` on `aab` is `aab` to sed and
//! `aa` to Rust. So Rust is asked only questions both engines answer alike:
//! where a match can start, and whether a whole span matches. The leftmost
//! start is found, then the longest span from it that the anchored pattern
//! accepts. Alternation is refused, since the two also assign groups
//! differently under it.
//!
//! What has no counterpart — a backreference in the pattern, a word edge, an
//! empty pattern reusing the last, a pattern matching the empty string under
//! `g` (the two place such matches differently), a `w`, `e`, `p` or `M` flag,
//! any command but `s` — is refused by name, never approximated. The
//! replacement is expanded by sed's rules: `&`, `\1`…`\9`, `\n`, `\t`.
//!
//! Lines are kept as sed keeps them: a final line without a newline stays
//! without one, which is GNU's behaviour; BSD's sed adds it, and the live check
//! is what says which ran.

use regex::{Regex, RegexBuilder};

/// Why a script is not followed, as the census names it after `sed `.
pub type Refused = String;

/// A line longer than this is not rewritten, nor tested by an address: finding
/// the longest match costs a pattern test per possible end.
const LONGEST_LINE: usize = 4096;

/// How `sed` was invoked: the scripts in order, whether `-E`, and the backup
/// suffix `-i` was given, if any.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Invocation {
    pub scripts: Vec<String>,
    pub extended: bool,
    pub suffix: Option<String>,
    /// Operands after the flags: the script when no `-e` gave one, then files.
    pub operands: Vec<String>,
}

/// Read `sed`'s flags. `-i` alone is GNU's (no backup); `-i ''` is BSD's, the
/// same; `-i.bak` keeps the old text under that suffix. `-n` changes what is
/// written and is refused, as is anything else this does not read.
pub fn invocation(argv: &[String]) -> Result<Invocation, Refused> {
    let mut out = Invocation::default();
    let mut words = argv.iter().skip(1).peekable();
    while let Some(word) = words.next() {
        match word.as_str() {
            "--" => {
                out.operands.extend(words.map(String::clone));
                break;
            }
            "-E" | "-r" | "--regexp-extended" => out.extended = true,
            "-n" | "--quiet" | "--silent" => return Err("-n".to_string()),
            "-i" | "--in-place" => {
                // BSD's mandatory suffix argument, given empty.
                if words.peek().is_some_and(|next| next.is_empty()) {
                    words.next();
                }
            }
            "-e" | "--expression" => match words.next() {
                Some(script) => out.scripts.push(script.clone()),
                None => return Err("-e".to_string()),
            },
            flag if flag.starts_with("--in-place=") => {
                out.suffix = Some(flag["--in-place=".len()..].to_string());
            }
            flag if flag.starts_with("--expression=") => {
                out.scripts.push(flag["--expression=".len()..].to_string());
            }
            flag if flag.starts_with('-') && flag.len() > 1 => {
                // Short flags may be joined: `-ne`, `-Ei`, `-i.bak`, `-es/a/b/`.
                let mut rest = flag[1..].chars();
                while let Some(c) = rest.next() {
                    match c {
                        'E' | 'r' => out.extended = true,
                        'i' => {
                            let suffix: String = rest.by_ref().collect();
                            if suffix.is_empty() {
                                if words.peek().is_some_and(|next| next.is_empty()) {
                                    words.next();
                                }
                            } else {
                                out.suffix = Some(suffix);
                            }
                        }
                        'e' => {
                            let script: String = rest.by_ref().collect();
                            if script.is_empty() {
                                match words.next() {
                                    Some(script) => out.scripts.push(script.clone()),
                                    None => return Err("-e".to_string()),
                                }
                            } else {
                                out.scripts.push(script);
                            }
                        }
                        other => return Err(format!("-{other}")),
                    }
                }
            }
            operand => out.operands.push(operand.to_string()),
        }
    }
    if out.scripts.is_empty() {
        if out.operands.is_empty() {
            return Err("no script".to_string());
        }
        out.scripts.push(out.operands.remove(0));
    }
    Ok(out)
}

/// A sed pattern, matched the way sed matches it.
#[derive(Debug)]
pub(super) struct Matcher {
    /// The pattern without its anchors, for where a match can start.
    core: Regex,
    /// The same, anchored at both ends, for whether a span is a match.
    full: Regex,
    at_start: bool,
    at_end: bool,
    /// Whether it can match the empty string.
    empty: bool,
}

impl Matcher {
    /// The leftmost match starting at or after `from`, and the longest from
    /// there.
    pub(super) fn find(&self, text: &str, from: usize) -> Option<(usize, usize)> {
        let len = text.len();
        let boundaries = |range: std::ops::RangeInclusive<usize>| {
            range.filter(move |at| text.is_char_boundary(*at))
        };
        let starts: Vec<usize> = match (self.at_start, self.at_end) {
            (true, _) if from == 0 => vec![0],
            (true, _) => return None,
            // The leftmost start is the same under either rule.
            (false, false) => vec![self.core.find_at(text, from)?.start()],
            (false, true) => boundaries(from..=len).collect(),
        };
        for start in starts {
            let ends: Vec<usize> = if self.at_end {
                vec![len]
            } else {
                boundaries(start..=len).rev().collect()
            };
            for end in ends {
                if self.full.is_match(&text[start..end]) {
                    return Some((start, end));
                }
            }
        }
        None
    }

    pub(super) fn is_match(&self, text: &str) -> bool {
        self.find(text, 0).is_some()
    }

    /// `text[start..end]` matched, expanded through `template` onto `out`.
    fn expand(&self, text: &str, start: usize, end: usize, template: &str, out: &mut String) {
        self.full
            .captures(&text[start..end])
            .expect("a span the anchored pattern accepted")
            .expand(template, out);
    }
}

/// One address of a command.
#[derive(Debug)]
enum Address {
    Line(usize),
    Last,
    Matches(Matcher),
}

impl Address {
    fn matches(&self, line: usize, last: bool, text: &str) -> bool {
        match self {
            Address::Line(n) => *n == line,
            Address::Last => last,
            Address::Matches(matcher) => matcher.is_match(text),
        }
    }
}

/// `s`, with what it applies to and how.
#[derive(Debug)]
struct Substitute {
    from: Option<Address>,
    to: Option<Address>,
    /// Inside a range, from the line `from` matched until `to` matches.
    active: bool,
    matcher: Matcher,
    template: String,
    /// Every match, or only the nth.
    which: Which,
}

#[derive(Debug, Clone, Copy)]
enum Which {
    First,
    All,
    Nth(usize),
}

/// `scripts`, applied in order to each line of `text`, as `sed` leaves it.
pub fn apply(scripts: &[&str], extended: bool, text: &str) -> Result<String, Refused> {
    let mut commands: Vec<Substitute> = Vec::new();
    for script in scripts {
        commands.extend(parse(script, extended)?);
    }
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let mut out = String::with_capacity(text.len());
    for (at, line) in lines.iter().enumerate() {
        let number = at + 1;
        let last = at + 1 == lines.len();
        let (body, newline) = match line.strip_suffix('\n') {
            Some(body) => (body, true),
            None => (*line, false),
        };
        if body.len() > LONGEST_LINE {
            return Err("long line".to_string());
        }
        let mut space = body.to_string();
        for command in &mut commands {
            if command.selects(number, last, &space) {
                space = command.run(&space)?;
            }
        }
        out.push_str(&space);
        if newline {
            out.push('\n');
        }
    }
    Ok(out)
}

impl Substitute {
    /// Whether this line is addressed, keeping the range state as sed does.
    fn selects(&mut self, line: usize, last: bool, text: &str) -> bool {
        match (&self.from, &self.to) {
            (None, _) => true,
            (Some(from), None) => from.matches(line, last, text),
            (Some(from), Some(to)) => {
                if self.active {
                    // A numeric end already behind the range's start ends it at
                    // once, as sed does; otherwise the end line is included.
                    if match to {
                        Address::Line(n) => *n <= line,
                        other => other.matches(line, last, text),
                    } {
                        self.active = false;
                    }
                    true
                } else if from.matches(line, last, text) {
                    self.active = !matches!(to, Address::Line(n) if *n <= line);
                    true
                } else {
                    false
                }
            }
        }
    }

    fn run(&self, space: &str) -> Result<String, Refused> {
        let wanted = match self.which {
            Which::First => 1,
            Which::Nth(n) => n,
            Which::All => usize::MAX,
        };
        if !matches!(self.which, Which::First) && self.matcher.empty {
            return Err("empty match".to_string());
        }
        let mut out = String::with_capacity(space.len());
        let (mut from, mut seen) = (0usize, 0usize);
        while let Some((start, end)) = self.matcher.find(space, from) {
            seen += 1;
            out.push_str(&space[from..start]);
            if matches!(self.which, Which::All) || seen == wanted {
                self.matcher
                    .expand(space, start, end, &self.template, &mut out);
            } else {
                out.push_str(&space[start..end]);
            }
            from = end;
            if seen == wanted || end == space.len() {
                break;
            }
        }
        out.push_str(&space[from..]);
        Ok(out)
    }
}

/// A script's commands, in order. Only `s` is read.
fn parse(script: &str, extended: bool) -> Result<Vec<Substitute>, Refused> {
    let chars: Vec<char> = script.chars().collect();
    let mut at = 0;
    let mut out = Vec::new();
    while at < chars.len() {
        match chars[at] {
            ' ' | '\t' | '\n' | ';' => {
                at += 1;
                continue;
            }
            '#' => {
                while at < chars.len() && chars[at] != '\n' {
                    at += 1;
                }
                continue;
            }
            _ => {}
        }
        let from = address(&chars, &mut at, extended)?;
        let to = if from.is_some() && chars.get(at) == Some(&',') {
            at += 1;
            Some(address(&chars, &mut at, extended)?.ok_or_else(|| "address".to_string())?)
        } else {
            None
        };
        while chars.get(at).is_some_and(|c| *c == ' ') {
            at += 1;
        }
        match chars.get(at) {
            Some('s') => {
                at += 1;
                out.push(substitute(&chars, &mut at, from, to, extended)?);
            }
            Some('!') => return Err("!".to_string()),
            Some(other) => return Err(other.to_string()),
            None => return Err("address".to_string()),
        }
    }
    Ok(out)
}

/// An address at `at`, if one is written there.
fn address(chars: &[char], at: &mut usize, extended: bool) -> Result<Option<Address>, Refused> {
    match chars.get(*at) {
        Some('$') => {
            *at += 1;
            Ok(Some(Address::Last))
        }
        Some(c) if c.is_ascii_digit() => {
            let start = *at;
            while chars.get(*at).is_some_and(char::is_ascii_digit) {
                *at += 1;
            }
            let n: usize = chars[start..*at]
                .iter()
                .collect::<String>()
                .parse()
                .map_err(|_| "address".to_string())?;
            if n == 0 {
                return Err("address 0".to_string());
            }
            if chars.get(*at) == Some(&'~') {
                return Err("address ~".to_string());
            }
            Ok(Some(Address::Line(n)))
        }
        Some('/') => {
            *at += 1;
            let pattern = delimited(chars, at, '/', Delimited::Pattern { extended })?;
            if chars.get(*at) == Some(&'I') {
                return Err("address I".to_string());
            }
            Ok(Some(Address::Matches(compile(&pattern, extended, false)?)))
        }
        Some('\\') => Err("address \\".to_string()),
        _ => Ok(None),
    }
}

/// What a delimited text is read as, which decides how `\delim` is spelled
/// back: the character itself, as a literal under the syntax in force.
#[derive(Clone, Copy)]
enum Delimited {
    Pattern { extended: bool },
    Replacement,
}

/// The text up to the next unescaped `delim`, with `\delim` read as the
/// delimiter itself and every other escape kept for the regex or template.
fn delimited(
    chars: &[char],
    at: &mut usize,
    delim: char,
    read_as: Delimited,
) -> Result<String, Refused> {
    let mut out = String::new();
    loop {
        match chars.get(*at) {
            None => return Err("unterminated".to_string()),
            Some(c) if *c == delim => {
                *at += 1;
                return Ok(out);
            }
            Some('\\') => match chars.get(*at + 1) {
                Some(c) if *c == delim => {
                    let escaped = match read_as {
                        Delimited::Pattern { extended } => {
                            ".*[]^$".contains(delim) || (extended && SWAPPED.contains(delim))
                        }
                        Delimited::Replacement => false,
                    };
                    if escaped {
                        out.push('\\');
                    }
                    out.push(delim);
                    *at += 2;
                }
                Some('\n') => {
                    out.push('\n');
                    *at += 2;
                }
                Some(c) => {
                    out.push('\\');
                    out.push(*c);
                    *at += 2;
                }
                None => return Err("unterminated".to_string()),
            },
            Some(c) => {
                out.push(*c);
                *at += 1;
            }
        }
    }
}

/// `s` after its letter: delimiter, pattern, replacement, flags.
fn substitute(
    chars: &[char],
    at: &mut usize,
    from: Option<Address>,
    to: Option<Address>,
    extended: bool,
) -> Result<Substitute, Refused> {
    let delim = match chars.get(*at) {
        Some(c) if !matches!(c, '\\' | '\n') => *c,
        _ => return Err("s delimiter".to_string()),
    };
    *at += 1;
    let pattern = delimited(chars, at, delim, Delimited::Pattern { extended })?;
    let replacement = delimited(chars, at, delim, Delimited::Replacement)?;
    let mut which = Which::First;
    let mut ignore_case = false;
    loop {
        match chars.get(*at) {
            None | Some(';' | '\n') => break,
            Some('g') => which = Which::All,
            Some('i' | 'I') => ignore_case = true,
            Some(c) if c.is_ascii_digit() => {
                let start = *at;
                while chars.get(*at).is_some_and(char::is_ascii_digit) {
                    *at += 1;
                }
                let n: usize = chars[start..*at]
                    .iter()
                    .collect::<String>()
                    .parse()
                    .map_err(|_| "s flag".to_string())?;
                if n == 0 {
                    return Err("s flag 0".to_string());
                }
                which = Which::Nth(n);
                continue;
            }
            Some(' ') => {}
            Some('}') => return Err("}".to_string()),
            Some(c) => return Err(format!("s flag {c}")),
        }
        *at += 1;
    }
    if pattern.is_empty() {
        return Err("empty pattern".to_string());
    }
    let matcher = compile(&pattern, extended, ignore_case)?;
    let template = template(&replacement, matcher.full.captures_len() - 1)?;
    Ok(Substitute {
        from,
        to,
        active: false,
        matcher,
        template,
        which,
    })
}

/// A sed pattern as a [`Matcher`] that finds the same spans.
pub(super) fn compile(
    pattern: &str,
    extended: bool,
    ignore_case: bool,
) -> Result<Matcher, Refused> {
    let (translated, at_start, at_end) = translate(pattern, extended)?;
    let hir = regex_syntax::Parser::new()
        .parse(&translated)
        .map_err(|_| "pattern".to_string())?;
    let build = |source: &str| {
        RegexBuilder::new(source)
            .case_insensitive(ignore_case)
            .dot_matches_new_line(true)
            .build()
            .map_err(|_| "pattern".to_string())
    };
    Ok(Matcher {
        core: build(&translated)?,
        full: build(&format!("\\A(?:{translated})\\z"))?,
        at_start,
        at_end,
        empty: hir.properties().minimum_len() == Some(0),
    })
}

/// The characters that are operators in extended syntax and literals in basic.
const SWAPPED: &str = "(){}+?|";

/// The pattern in Rust's syntax without its anchors, and whether it had one at
/// either end. An anchor anywhere else is refused: basic syntax makes it a
/// literal there and extended syntax an anchor, and neither is worth a rule.
fn translate(pattern: &str, extended: bool) -> Result<(String, bool, bool), Refused> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::new();
    let mut at = 0;
    let at_start = chars.first() == Some(&'^');
    if at_start {
        at += 1;
    }
    let at_end = chars.len() > at
        && chars.last() == Some(&'$')
        && chars[..chars.len() - 1]
            .iter()
            .rev()
            .take_while(|c| **c == '\\')
            .count()
            % 2
            == 0;
    let stop = if at_end { chars.len() - 1 } else { chars.len() };
    // Where a `*` is a literal in basic syntax: at the start or after `\(`.
    let mut opens = true;
    while at < stop {
        let c = chars[at];
        at += 1;
        match c {
            '\\' => {
                let Some(next) = chars.get(at).copied().filter(|_| at < stop) else {
                    return Err("escape".to_string());
                };
                at += 1;
                match next {
                    '|' if !extended => return Err("alternation".to_string()),
                    c if SWAPPED.contains(c) => {
                        if extended {
                            out.push('\\');
                            out.push(c);
                        } else {
                            out.push(c);
                        }
                        opens = c == '(';
                        continue;
                    }
                    '.' | '*' | '[' | ']' | '^' | '$' | '\\' | '/' | '-' => {
                        out.push('\\');
                        out.push(next);
                    }
                    'n' => out.push_str("\\n"),
                    't' => out.push_str("\\t"),
                    // GNU's literal ampersand, which Rust needs no escape for.
                    '&' => out.push('&'),
                    // A boundary is judged against a span, not the line.
                    'b' | 'B' => return Err("word boundary".to_string()),
                    's' | 'S' | 'w' | 'W' => {
                        out.push('\\');
                        out.push(next);
                    }
                    '1'..='9' => return Err("backreference".to_string()),
                    '<' | '>' | '`' | '\'' => return Err("word edge".to_string()),
                    other => return Err(format!("escape \\{other}")),
                }
            }
            '|' if extended => return Err("alternation".to_string()),
            c if SWAPPED.contains(c) => {
                if extended {
                    out.push(c);
                    opens = c == '(';
                    continue;
                }
                out.push('\\');
                out.push(c);
            }
            '*' if opens && !extended => out.push_str("\\*"),
            '^' if extended => return Err("anchor".to_string()),
            '$' if extended => return Err("anchor".to_string()),
            '^' => out.push_str("\\^"),
            '$' => out.push_str("\\$"),
            '[' => {
                out.push('[');
                if chars.get(at) == Some(&'^') {
                    out.push('^');
                    at += 1;
                }
                // A `]` first is a member, not the end.
                if chars.get(at) == Some(&']') {
                    out.push_str("\\]");
                    at += 1;
                }
                loop {
                    let Some(member) = chars.get(at).copied() else {
                        return Err("bracket".to_string());
                    };
                    at += 1;
                    match member {
                        ']' => break,
                        '[' if chars.get(at) == Some(&':') => {
                            let Some(end) = chars[at..].iter().position(|c| *c == ']') else {
                                return Err("bracket".to_string());
                            };
                            let class: String = chars[at..at + end].iter().collect();
                            out.push('[');
                            out.push_str(&class);
                            out.push(']');
                            at += end + 1;
                        }
                        '[' if matches!(chars.get(at), Some('.' | '=')) => {
                            return Err("collating".to_string());
                        }
                        // A character to sed and an escape to Rust; GNU reads a
                        // few of them its own way, so none is guessed at.
                        '\\' => return Err("bracket escape".to_string()),
                        '[' | '&' | '~' => {
                            out.push('\\');
                            out.push(member);
                        }
                        other => out.push(other),
                    }
                }
                out.push(']');
                opens = false;
                continue;
            }
            other => out.push(other),
        }
        opens = false;
    }
    if at_end && at > stop {
        return Err("escape".to_string());
    }
    Ok((out, at_start, at_end))
}

/// A sed replacement as a Rust template: `&` is the match, `\N` a group,
/// `\n` and `\t` themselves, and a `$` is spelled `$$`.
fn template(replacement: &str, groups: usize) -> Result<String, Refused> {
    let mut out = String::new();
    let mut chars = replacement.chars();
    while let Some(c) = chars.next() {
        match c {
            '&' => out.push_str("${0}"),
            '$' => out.push_str("$$"),
            '\\' => match chars.next() {
                None => return Err("replacement".to_string()),
                Some('&') => out.push('&'),
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('\\') => out.push('\\'),
                Some(d @ '0'..='9') => {
                    let n = d.to_digit(10).expect("a digit") as usize;
                    if n > groups {
                        return Err("group".to_string());
                    }
                    out.push_str(&format!("${{{n}}}"));
                }
                Some('L' | 'U' | 'E' | 'l' | 'u') => return Err("case conversion".to_string()),
                Some(other) => out.push(other),
            },
            other => out.push(other),
        }
    }
    Ok(out)
}
