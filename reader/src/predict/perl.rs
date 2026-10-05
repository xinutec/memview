//! `perl -pi -e 's/…/…/'` and `perl -0pi -e`, where Perl's regular expressions
//! and Rust's mean the same thing.
//!
//! The code is read as a sequence of `s` statements, split on `;` outside a
//! delimited part, and nothing else: a `print`, an `if`, a `-n` with a
//! condition are refused by name. `-p` makes each line a record, with its
//! newline; `-0` makes the whole text one — Perl's record separator becomes
//! NUL, which a text file does not hold, and one holding a NUL is refused — and
//! `-0777` the same by its own rule.
//!
//! **Perl and Rust agree on where a match starts and which alternative wins:**
//! both take the leftmost match and the first alternative that succeeds, which
//! is why no longest-match construction is needed here as it is for sed. They
//! part on three things, each refused by name: a backreference, lookaround or
//! a possessive quantifier, which Rust's engine has no counterpart for; an
//! empty match under `g`, which the two place differently around the ends of a
//! record; and non-ASCII text under anything that reads characters — `.`, a
//! class, a word edge, `i` — because `perl -pi` without `-C` reads BYTES, so
//! `.` on `é` matches one of its two bytes (measured: `s/./x/g` on `aé` gives
//! three `x`). A literal is the same bytes to both.
//!
//! **The anchors and the final newline.** Perl's `$` without `m` matches at
//! the end or before a final newline, Rust's only at the end; Perl's `^` under
//! `m` never matches after a final newline, where Rust's does. Under `-p` a
//! record is one line with at most one newline, at its end, so there Perl's
//! bare `$` is exactly Rust's `(?m:$)` and its bare `^` is `\A`, and the whole
//! record is the haystack — `\s+$` eats the newline, as perl's does
//! (measured). Under `-0` the record holds other newlines, so a pattern that
//! cannot match one is matched against the record with its final newline held
//! back and put back after, which is exact, and one that can (`\s+$`) or
//! reads the end (`\z`) is matched against the whole record, where a bare `$`
//! before a final newline is refused. A `^` under `m` is matched the same way
//! in both modes, and refused where the whole record is the haystack.
//!
//! An escaped delimiter is the bare delimiter handed to the regex engine, where
//! it means what it means: `s|a\|b|X|` is an alternation (measured). The
//! replacement is expanded by Perl's rules: `$1`, `${1}`, `\1`, `$&`, the
//! escapes, and a group that took no part as the empty string; a variable, an
//! array, `\Q`, `\U` and `\L` are refused.
//!
//! Held to what `/usr/bin/perl` 5.34 left in a file, in `reader/tests/suite/perl.rs`.

use regex::{Regex, RegexBuilder};
use regex_syntax::hir::{Class, Hir, HirKind};

/// Why a program is not followed, as the census names it after `perl `.
pub type Refused = String;

/// How `perl` was invoked: each `-e`, in order; whether `-0` made the whole
/// text one record; the backup suffix `-i` was given, if any; and the files.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Invocation {
    pub code: Vec<String>,
    pub slurp: bool,
    pub suffix: Option<String>,
    pub operands: Vec<String>,
}

/// Read `perl`'s flags as perl reads a cluster: `-pi`, `-0pi`, `-i -pe`. An `i`
/// takes the rest of its cluster as the backup suffix (`-pi.bak`), an `e` the
/// rest or the next word as code, a `0` the digits after it as the separator.
/// `-n`, `-l` and anything else this does not read are refused by name, and
/// so is a call without `-p`, `-i` or `-e`.
pub fn invocation(argv: &[String]) -> Result<Invocation, Refused> {
    let mut out = Invocation::default();
    let (mut print, mut in_place) = (false, false);
    let mut words = argv.iter().skip(1);
    while let Some(word) = words.next() {
        let Some(cluster) = word.strip_prefix('-').filter(|c| !c.is_empty()) else {
            out.operands.push(word.clone());
            continue;
        };
        if cluster == "-" {
            out.operands.extend(words.map(String::clone));
            break;
        }
        if cluster.starts_with('-') {
            return Err(format!("-{cluster}"));
        }
        let chars: Vec<char> = cluster.chars().collect();
        let mut at = 0;
        while at < chars.len() {
            match chars[at] {
                'p' => print = true,
                'w' | 'W' => {}
                'i' => {
                    in_place = true;
                    let rest: String = chars[at + 1..].iter().collect();
                    if !rest.is_empty() {
                        out.suffix = Some(rest);
                    }
                    break;
                }
                'e' | 'E' => {
                    let rest: String = chars[at + 1..].iter().collect();
                    let code = if rest.is_empty() {
                        words.next().ok_or_else(|| "no -e".to_string())?.clone()
                    } else {
                        rest
                    };
                    out.code.push(code);
                    break;
                }
                '0' => {
                    out.slurp = true;
                    let mut digits = String::new();
                    while chars.get(at + 1).is_some_and(char::is_ascii_digit) {
                        at += 1;
                        digits.push(chars[at]);
                    }
                    if !digits.is_empty() && digits != "777" && digits != "0" {
                        return Err(format!("-0{digits}"));
                    }
                }
                other => return Err(format!("-{other}")),
            }
            at += 1;
        }
    }
    if !print {
        return Err("no -p".to_string());
    }
    if !in_place {
        return Err("no -i".to_string());
    }
    if out.code.is_empty() {
        return Err("no -e".to_string());
    }
    Ok(out)
}

/// One piece of a replacement.
enum Part {
    Text(String),
    Group(usize),
}

/// One `s` statement, compiled.
struct Substitute {
    regex: Regex,
    parts: Vec<Part>,
    global: bool,
    /// The pattern can match a newline or reads the end, so the whole record
    /// must be the haystack.
    whole_record: bool,
    slurp: bool,
    multiline: bool,
    has_dollar: bool,
    has_caret: bool,
    /// Reads characters — `.`, a class, a word edge, `i` — where Perl reads bytes.
    reads_chars: bool,
}

/// What `-p` or `-0 -p` leaves of `text` after `code` ran on each record.
pub fn apply(code: &[&str], slurp: bool, text: &str) -> Result<String, Refused> {
    let mut statements = Vec::new();
    for code in code {
        statements.extend(parse(code, slurp)?);
    }
    if slurp {
        if text.contains('\0') {
            return Err("NUL in the text".to_string());
        }
        return run(&statements, text);
    }
    let mut out = String::with_capacity(text.len());
    for record in text.split_inclusive('\n') {
        out.push_str(&run(&statements, record)?);
    }
    Ok(out)
}

fn run(statements: &[Substitute], record: &str) -> Result<String, Refused> {
    let mut now = record.to_string();
    for statement in statements {
        now = statement.run(&now)?;
    }
    Ok(now)
}

impl Substitute {
    fn run(&self, record: &str) -> Result<String, Refused> {
        if self.reads_chars && !record.is_ascii() {
            return Err("non-ASCII text".to_string());
        }
        // The haystack: the record whole, or with its final newline held back
        // and put back after — the module head says which when.
        let whole = if self.multiline {
            if self.whole_record && self.has_caret {
                return Err("^ after a final newline".to_string());
            }
            self.whole_record
        } else if self.slurp {
            if self.whole_record && self.has_dollar && record.ends_with('\n') {
                return Err("$ before a final newline".to_string());
            }
            self.whole_record
        } else {
            true
        };
        let (haystack, newline) = if whole {
            (record, "")
        } else {
            match record.strip_suffix('\n') {
                Some(body) => (body, "\n"),
                None => (record, ""),
            }
        };
        let expand = |captures: &regex::Captures<'_>| -> String {
            self.parts
                .iter()
                .map(|part| match part {
                    Part::Text(text) => text.clone(),
                    Part::Group(n) => captures
                        .get(*n)
                        .map(|m| m.as_str().to_string())
                        .unwrap_or_default(),
                })
                .collect()
        };
        let replaced = if self.global {
            self.regex.replace_all(haystack, expand)
        } else {
            self.regex.replace(haystack, expand)
        };
        Ok(format!("{replaced}{newline}"))
    }
}

/// The `s` statements of one `-e`, in order.
fn parse(code: &str, slurp: bool) -> Result<Vec<Substitute>, Refused> {
    let chars: Vec<char> = code.chars().collect();
    let mut at = 0;
    let mut out = Vec::new();
    loop {
        while at < chars.len() && (chars[at].is_whitespace() || chars[at] == ';') {
            at += 1;
        }
        if at >= chars.len() {
            break;
        }
        if chars[at] != 's' {
            return Err("statement".to_string());
        }
        at += 1;
        let delimiter = *chars.get(at).ok_or_else(|| "delimiter".to_string())?;
        if delimiter.is_alphanumeric() || delimiter.is_whitespace() || delimiter == '\\' {
            return Err("delimiter".to_string());
        }
        if "([{<".contains(delimiter) {
            return Err("bracket delimiter".to_string());
        }
        at += 1;
        let pattern = delimited(&chars, &mut at, delimiter)?;
        let replacement = delimited(&chars, &mut at, delimiter)?;
        let mut flags = String::new();
        while at < chars.len() && chars[at].is_ascii_alphabetic() {
            flags.push(chars[at]);
            at += 1;
        }
        while at < chars.len() && chars[at].is_whitespace() {
            at += 1;
        }
        if at < chars.len() && chars[at] != ';' {
            return Err("statement".to_string());
        }
        out.push(substitute(&pattern, &replacement, &flags, slurp)?);
    }
    Ok(out)
}

/// The text up to the next unescaped delimiter. An escaped delimiter is the
/// bare delimiter; any other escape is kept for the pattern or replacement
/// to read.
fn delimited(chars: &[char], at: &mut usize, delimiter: char) -> Result<String, Refused> {
    let mut out = String::new();
    while *at < chars.len() {
        let c = chars[*at];
        *at += 1;
        if c == delimiter {
            return Ok(out);
        }
        if c == '\\' {
            let next = *chars.get(*at).ok_or_else(|| "unterminated".to_string())?;
            *at += 1;
            if next == delimiter {
                out.push(delimiter);
            } else {
                out.push('\\');
                out.push(next);
            }
            continue;
        }
        out.push(c);
    }
    Err("unterminated".to_string())
}

fn substitute(
    pattern: &str,
    replacement: &str,
    flags: &str,
    slurp: bool,
) -> Result<Substitute, Refused> {
    let (mut global, mut fold_case, mut multiline, mut dot_all) = (false, false, false, false);
    for flag in flags.chars() {
        match flag {
            'g' => global = true,
            'i' => fold_case = true,
            'm' => multiline = true,
            's' => dot_all = true,
            other => return Err(format!("flag {other}")),
        }
    }
    let translated = translate(pattern, multiline, slurp)?;
    let regex = RegexBuilder::new(&translated.source)
        .case_insensitive(fold_case)
        .multi_line(multiline)
        .dot_matches_new_line(dot_all)
        .build()
        .map_err(|_| "pattern".to_string())?;
    let hir = regex_syntax::ParserBuilder::new()
        .case_insensitive(fold_case)
        .multi_line(multiline)
        .dot_matches_new_line(dot_all)
        .build()
        .parse(&translated.source)
        .map_err(|_| "pattern".to_string())?;
    // An empty match that can only be at a line's start is placed alike by
    // both engines; any other the two place differently around the ends of a
    // record (perl: `-a--c-\n-` for `s/b*/-/g` on `abc`).
    let at_start_only = (translated.source.starts_with("\\A")
        || translated.source.starts_with('^'))
        && !translated.source.contains('|');
    if global && hir.properties().minimum_len() == Some(0) && !at_start_only {
        return Err("empty match".to_string());
    }
    let parts = template(replacement, regex.captures_len())?;
    Ok(Substitute {
        regex,
        parts,
        global,
        whole_record: matches_newline(&hir) || translated.reads_end,
        slurp,
        multiline,
        has_dollar: translated.has_dollar,
        has_caret: translated.has_caret,
        reads_chars: translated.reads_chars || fold_case,
    })
}

/// Whether the pattern can match a newline anywhere in it.
fn matches_newline(hir: &Hir) -> bool {
    match hir.kind() {
        HirKind::Literal(literal) => literal.0.contains(&b'\n'),
        HirKind::Class(Class::Unicode(class)) => class
            .ranges()
            .iter()
            .any(|range| range.start() <= '\n' && '\n' <= range.end()),
        HirKind::Class(Class::Bytes(class)) => class
            .ranges()
            .iter()
            .any(|range| range.start() <= b'\n' && b'\n' <= range.end()),
        HirKind::Repetition(repetition) => matches_newline(&repetition.sub),
        HirKind::Capture(capture) => matches_newline(&capture.sub),
        HirKind::Concat(parts) | HirKind::Alternation(parts) => parts.iter().any(matches_newline),
        HirKind::Empty | HirKind::Look(_) => false,
    }
}

/// A pattern translated for Rust, and what it reads.
#[derive(Default)]
struct Translated {
    source: String,
    has_dollar: bool,
    has_caret: bool,
    reads_chars: bool,
    reads_end: bool,
}

/// Perl's pattern as Rust reads it: the escapes both know pass through, a
/// named group's opener is respelled, and what Rust has no counterpart for is
/// refused by name.
fn translate(pattern: &str, multiline: bool, slurp: bool) -> Result<Translated, Refused> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut at = 0;
    let mut out = Translated::default();
    // Inside a class, and whether a `]` here would still be its first member.
    let mut class: Option<bool> = None;
    // The last thing written was an unescaped quantifier, so a `+` now would
    // be possessive; or a bare non-ASCII literal, so a quantifier now would
    // repeat its last byte in Perl and the whole character in Rust.
    let (mut after_quantifier, mut after_non_ascii) = (false, false);
    while at < chars.len() {
        let c = chars[at];
        at += 1;
        let (quantifier, non_ascii) = (false, false);
        match (c, class) {
            ('\\', _) => {
                let escaped = *chars.get(at).ok_or_else(|| "pattern".to_string())?;
                at += 1;
                let spelled = escape(escaped, class.is_some(), &chars, &mut at, &mut out)?;
                out.source.push_str(&spelled);
                class = class.map(|_| false);
            }
            ('[', None) => {
                out.source.push('[');
                if chars.get(at) == Some(&'^') {
                    out.source.push('^');
                    at += 1;
                }
                out.reads_chars = true;
                class = Some(true);
            }
            (']', Some(true)) => {
                out.source.push_str("\\]");
                class = Some(false);
            }
            (']', Some(false)) => {
                out.source.push(']');
                class = None;
            }
            ('[', Some(_)) => {
                // A POSIX class, `[:alpha:]`, which Rust reads the same way.
                if chars.get(at) == Some(&':') {
                    let close = chars[at..]
                        .windows(2)
                        .position(|w| w == [':', ']'])
                        .ok_or_else(|| "class".to_string())?;
                    out.source.push('[');
                    out.source.extend(&chars[at..at + close + 2]);
                    at += close + 2;
                } else {
                    out.source.push_str("\\[");
                }
                class = Some(false);
            }
            ('&' | '~', Some(_)) => {
                out.source.push('\\');
                out.source.push(c);
                class = Some(false);
            }
            ('-', Some(_)) if chars.get(at) == Some(&'-') => return Err("class".to_string()),
            ('$', None) => match chars.get(at) {
                Some(next) if next.is_alphanumeric() || matches!(next, '_' | '{' | '&') => {
                    return Err("variable".to_string());
                }
                _ => {
                    out.has_dollar = true;
                    // Perl's bare `$` on a one-line record — see the module head.
                    out.source
                        .push_str(if multiline || slurp { "$" } else { "(?m:$)" });
                }
            },
            ('@', None) => {
                if chars
                    .get(at)
                    .is_some_and(|next| next.is_alphanumeric() || matches!(next, '_' | '{'))
                {
                    return Err("array".to_string());
                }
                out.source.push('@');
            }
            ('^', None) => {
                out.has_caret = true;
                out.source.push_str(if multiline { "^" } else { "\\A" });
            }
            ('.', None) => {
                out.reads_chars = true;
                out.source.push('.');
            }
            ('(', None) => {
                if chars.get(at) == Some(&'?') {
                    let rest: String = chars[at + 1..].iter().take(2).collect();
                    let first = rest.chars().next().unwrap_or(' ');
                    if first == '=' || first == '!' || rest == "<=" || rest == "<!" {
                        return Err("lookaround".to_string());
                    }
                    if first == '#' {
                        return Err("comment".to_string());
                    }
                    if first == '>' {
                        return Err("atomic group".to_string());
                    }
                    if matches!(first, '|' | '(' | 'R' | '&' | 'P' | '^') || first.is_ascii_digit()
                    {
                        return Err("group".to_string());
                    }
                    if first == '<' {
                        out.source.push_str("(?P<");
                        at += 2;
                        continue;
                    }
                }
                out.source.push('(');
            }
            ('+', None) if after_quantifier => return Err("possessive quantifier".to_string()),
            ('*' | '+' | '?' | '{', None) if after_non_ascii => {
                return Err("quantified non-ASCII literal".to_string());
            }
            ('*' | '+' | '?' | '}', None) => {
                out.source.push(c);
                after_quantifier = true;
                after_non_ascii = false;
                continue;
            }
            (c, Some(_)) => {
                if !c.is_ascii() {
                    out.reads_chars = true;
                }
                out.source.push(c);
                class = Some(false);
            }
            (c, None) => {
                out.source.push(c);
                after_quantifier = false;
                after_non_ascii = !c.is_ascii();
                continue;
            }
        }
        after_quantifier = quantifier;
        after_non_ascii = non_ascii;
    }
    if class.is_some() {
        return Err("class".to_string());
    }
    Ok(out)
}

/// One escape of the pattern as Rust spells it, or why it has no counterpart.
fn escape(
    escaped: char,
    in_class: bool,
    chars: &[char],
    at: &mut usize,
    out: &mut Translated,
) -> Result<String, Refused> {
    Ok(match escaped {
        '1'..='9' if !in_class => return Err("backreference".to_string()),
        'g' | 'k' => return Err("backreference".to_string()),
        '0' => return Err("escape".to_string()),
        'A' if !in_class => "\\A".to_string(),
        'z' if !in_class => {
            out.reads_end = true;
            "\\z".to_string()
        }
        'Z' => return Err("\\Z".to_string()),
        'b' if in_class => "\\x08".to_string(),
        'b' | 'B' if !in_class => {
            out.reads_chars = true;
            format!("\\{escaped}")
        }
        'd' | 'D' | 's' | 'S' | 'w' | 'W' => {
            out.reads_chars = true;
            format!("\\{escaped}")
        }
        'n' | 't' | 'r' | 'f' => format!("\\{escaped}"),
        'a' => "\\x07".to_string(),
        'e' => "\\x1B".to_string(),
        'x' => {
            let code = hex(chars, at)?;
            if code > 0x7F {
                return Err("non-ASCII escape".to_string());
            }
            format!("\\x{code:02X}")
        }
        'Q' | 'E' => return Err("quotemeta".to_string()),
        'U' | 'L' | 'u' | 'l' => return Err("case conversion".to_string()),
        c if c.is_ascii_alphanumeric() => return Err(format!("\\{c}")),
        // Perl reads any other escaped character as itself.
        c => regex::escape(&c.to_string()),
    })
}

/// The code of a `\x` escape: `\xHH`, or `\x{H…}`.
fn hex(chars: &[char], at: &mut usize) -> Result<u32, Refused> {
    let digits: String = if chars.get(*at) == Some(&'{') {
        let close = chars[*at..]
            .iter()
            .position(|c| *c == '}')
            .ok_or_else(|| "escape".to_string())?;
        let inner: String = chars[*at + 1..*at + close].iter().collect();
        *at += close + 1;
        inner
    } else {
        let taken: String = chars[*at..]
            .iter()
            .take(2)
            .take_while(|c| c.is_ascii_hexdigit())
            .collect();
        *at += taken.len();
        taken
    };
    u32::from_str_radix(&digits, 16).map_err(|_| "escape".to_string())
}

/// A replacement by Perl's rules.
fn template(replacement: &str, groups: usize) -> Result<Vec<Part>, Refused> {
    let chars: Vec<char> = replacement.chars().collect();
    let mut at = 0;
    let mut parts = Vec::new();
    let mut text = String::new();
    let group = |n: usize, text: &mut String, parts: &mut Vec<Part>| -> Result<(), Refused> {
        if n >= groups {
            return Err("group".to_string());
        }
        parts.push(Part::Text(std::mem::take(text)));
        parts.push(Part::Group(n));
        Ok(())
    };
    while at < chars.len() {
        let c = chars[at];
        at += 1;
        match c {
            '$' => match chars.get(at) {
                Some(d) if d.is_ascii_digit() => {
                    let mut digits = String::new();
                    while chars.get(at).is_some_and(char::is_ascii_digit) {
                        digits.push(chars[at]);
                        at += 1;
                    }
                    group(
                        digits.parse().map_err(|_| "group".to_string())?,
                        &mut text,
                        &mut parts,
                    )?;
                }
                Some('{') => {
                    let close = chars[at..]
                        .iter()
                        .position(|c| *c == '}')
                        .ok_or_else(|| "variable".to_string())?;
                    let inner: String = chars[at + 1..at + close].iter().collect();
                    at += close + 1;
                    let n = inner.parse().map_err(|_| "variable".to_string())?;
                    group(n, &mut text, &mut parts)?;
                }
                Some('&') => {
                    at += 1;
                    group(0, &mut text, &mut parts)?;
                }
                Some(next) if next.is_alphanumeric() || matches!(next, '_' | '`' | '\'' | '+') => {
                    return Err("variable".to_string());
                }
                _ => text.push('$'),
            },
            '@' => {
                if chars
                    .get(at)
                    .is_some_and(|next| next.is_alphanumeric() || matches!(next, '_' | '{'))
                {
                    return Err("array".to_string());
                }
                text.push('@');
            }
            '\\' => {
                let escaped = *chars.get(at).ok_or_else(|| "escape".to_string())?;
                at += 1;
                let literal = match escaped {
                    'n' => '\n',
                    't' => '\t',
                    'r' => '\r',
                    'f' => '\u{c}',
                    'a' => '\u{7}',
                    'e' => '\u{1b}',
                    '1'..='9' => {
                        let n = escaped.to_digit(10).expect("a digit") as usize;
                        group(n, &mut text, &mut parts)?;
                        continue;
                    }
                    '0' => return Err("escape".to_string()),
                    'x' => {
                        let code = hex(&chars, &mut at)?;
                        if code > 0x7F {
                            return Err("non-ASCII escape".to_string());
                        }
                        char::from_u32(code).ok_or_else(|| "escape".to_string())?
                    }
                    'Q' | 'E' | 'U' | 'L' | 'u' | 'l' => {
                        return Err("case conversion".to_string());
                    }
                    c if c.is_ascii_alphanumeric() => return Err(format!("\\{c}")),
                    c => c,
                };
                text.push(literal);
            }
            c => text.push(c),
        }
    }
    parts.push(Part::Text(text));
    Ok(parts)
}
