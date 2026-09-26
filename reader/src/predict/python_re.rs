//! Python's `re.sub`, where Python's `re` and Rust's `regex` mean the same thing.
//!
//! The pattern is translated, not copied: an escaped punctuation character is a
//! literal in Python and re-escaped for Rust, `\Z` is Rust's `\z`, and inside a
//! class `[`, `&` and `~` are literals in Python and set syntax in Rust. What has
//! no counterpart — a backreference, verbose mode, a pattern that can match the
//! empty string, where the two place empty matches differently — is refused by
//! name, never approximated. The replacement is expanded by Python's own rules.

use regex::{Regex, RegexBuilder};

/// `re.I`, `re.M`, `re.S`, `re.X` and `re.A`, as Python numbers them.
pub(super) const IGNORECASE: i64 = 2;
pub(super) const MULTILINE: i64 = 8;
pub(super) const DOTALL: i64 = 16;
const UNICODE: i64 = 32;
const VERBOSE: i64 = 64;
const ASCII: i64 = 256;

/// A flag's value, from its name in the `re` module.
pub(super) fn flag(name: &str) -> Option<i64> {
    Some(match name {
        "I" | "IGNORECASE" => IGNORECASE,
        "M" | "MULTILINE" => MULTILINE,
        "S" | "DOTALL" => DOTALL,
        "U" | "UNICODE" => UNICODE,
        "X" | "VERBOSE" => VERBOSE,
        "A" | "ASCII" => ASCII,
        _ => return None,
    })
}

/// A pattern Rust will match exactly as Python would.
#[derive(Debug, Clone)]
pub(super) struct Pattern {
    regex: Regex,
    /// A `$` without `re.M`, which Python also matches before a final newline.
    bare_dollar: bool,
}

/// Why a substitution is not followed, as the census names it.
pub(super) type Refused = &'static str;

pub(super) fn compile(source: &str, flags: i64) -> Result<Pattern, Refused> {
    if flags & !(IGNORECASE | MULTILINE | DOTALL | UNICODE) != 0 {
        return Err("re flag");
    }
    let (translated, bare_dollar) = translate(source, flags & MULTILINE != 0)?;
    let hir = regex_syntax::Parser::new()
        .parse(&translated)
        .map_err(|_| "re pattern")?;
    if hir.properties().minimum_len() == Some(0) {
        return Err("re empty match");
    }
    let regex = RegexBuilder::new(&translated)
        .case_insensitive(flags & IGNORECASE != 0)
        .multi_line(flags & MULTILINE != 0)
        .dot_matches_new_line(flags & DOTALL != 0)
        .build()
        .map_err(|_| "re pattern")?;
    Ok(Pattern { regex, bare_dollar })
}

/// `pattern.sub(template, text, count)`, with how many were replaced; `count` 0
/// is every match.
pub(super) fn sub(
    pattern: &Pattern,
    template: &str,
    text: &str,
    count: usize,
) -> Result<(String, usize), Refused> {
    if pattern.bare_dollar && text.ends_with('\n') {
        return Err("re $ before a final newline");
    }
    let parts = parse_template(template, pattern)?;
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    let mut done = 0;
    for captures in pattern.regex.captures_iter(text) {
        if count != 0 && done == count {
            break;
        }
        let whole = captures.get(0).expect("group 0 always matches");
        out.push_str(&text[last..whole.start()]);
        for part in &parts {
            match part {
                Part::Text(literal) => out.push_str(literal),
                Part::Group(index) => {
                    out.push_str(captures.get(*index).map_or("", |group| group.as_str()));
                }
            }
        }
        last = whole.end();
        done += 1;
    }
    out.push_str(&text[last..]);
    Ok((out, done))
}

/// `re.escape`: the characters Python escapes since 3.7.
pub(super) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if "()[]{}?*+-|^$\\.&~# \t\n\r\u{b}\u{c}".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// The pattern as Rust spells it, and whether it holds a `$` that Python would
/// also match before a final newline.
fn translate(source: &str, multiline: bool) -> Result<(String, bool), Refused> {
    let mut out = String::with_capacity(source.len());
    let mut bare_dollar = false;
    let mut chars = source.chars().peekable();
    // Inside a class, and whether a `]` here would still be its first member.
    let mut class: Option<bool> = None;
    while let Some(c) = chars.next() {
        match (c, class) {
            ('\\', _) => {
                let Some(escaped) = chars.next() else {
                    return Err("re pattern");
                };
                out.push_str(&translate_escape(escaped, class.is_some())?);
                class = class.map(|_| false);
            }
            ('[', None) => {
                out.push('[');
                if chars.peek() == Some(&'^') {
                    out.push(chars.next().expect("peeked"));
                }
                class = Some(true);
            }
            (']', Some(true)) => {
                out.push_str("\\]");
                class = Some(false);
            }
            (']', Some(false)) => {
                out.push(']');
                class = None;
            }
            ('[' | '&' | '~', Some(_)) => {
                out.push('\\');
                out.push(c);
                class = Some(false);
            }
            ('-', Some(_)) if chars.peek() == Some(&'-') => return Err("re class"),
            ('$', None) => {
                bare_dollar |= !multiline;
                out.push('$');
            }
            ('(', None) if source_follows(&mut chars, "?P=") => return Err("re backreference"),
            ('(', None) if source_follows(&mut chars, "?#") => return Err("re comment"),
            (c, Some(_)) => {
                out.push(c);
                class = Some(false);
            }
            (c, None) => out.push(c),
        }
    }
    Ok((out, bare_dollar))
}

/// Whether the text after the current character starts with `prefix`.
fn source_follows(chars: &mut std::iter::Peekable<std::str::Chars<'_>>, prefix: &str) -> bool {
    chars.clone().take(prefix.len()).eq(prefix.chars())
}

fn translate_escape(c: char, in_class: bool) -> Result<String, Refused> {
    Ok(match c {
        '1'..='9' if !in_class => return Err("re backreference"),
        '0'..='9' => return Err("re escape"),
        'Z' if !in_class => "\\z".to_string(),
        'b' if in_class => "\\x08".to_string(),
        'A' | 'b' | 'B' if !in_class => format!("\\{c}"),
        'd' | 'D' | 's' | 'S' | 'w' | 'W' | 'n' | 't' | 'r' | 'f' | 'v' | 'a' | 'x' | 'u' | 'U' => {
            format!("\\{c}")
        }
        c if c.is_ascii_alphanumeric() => return Err("re escape"),
        // Python reads any other escaped character as itself.
        c => regex::escape(&c.to_string()),
    })
}

/// One piece of a replacement.
enum Part {
    Text(String),
    Group(usize),
}

/// A replacement by Python's rules: `\1`, `\g<name>`, the standard escapes, and
/// a backslash before anything else kept as written.
fn parse_template(template: &str, pattern: &Pattern) -> Result<Vec<Part>, Refused> {
    let groups = pattern.regex.captures_len();
    let group = |index: usize| {
        if index < groups {
            Ok(Part::Group(index))
        } else {
            Err("re replacement group")
        }
    };
    let mut parts = Vec::new();
    let mut text = String::new();
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            text.push(c);
            continue;
        }
        let Some(escaped) = chars.next() else {
            return Err("re replacement escape");
        };
        let literal = match escaped {
            'g' => {
                if chars.next() != Some('<') {
                    return Err("re replacement group");
                }
                let name: String = chars.by_ref().take_while(|c| *c != '>').collect();
                let index = match name.parse::<usize>() {
                    Ok(index) => index,
                    Err(_) => pattern
                        .regex
                        .capture_names()
                        .position(|named| named == Some(name.as_str()))
                        .ok_or("re replacement group")?,
                };
                parts.push(Part::Text(std::mem::take(&mut text)));
                parts.push(group(index)?);
                continue;
            }
            '0' => {
                let mut code = 0u32;
                for _ in 0..2 {
                    match chars.peek().and_then(|c| c.to_digit(8)) {
                        Some(digit) => {
                            code = code * 8 + digit;
                            chars.next();
                        }
                        None => break,
                    }
                }
                char::from_u32(code).ok_or("re replacement escape")?
            }
            '1'..='9' => {
                let mut digits = escaped.to_string();
                if let Some(next) = chars.peek().copied().filter(char::is_ascii_digit) {
                    chars.next();
                    let octal = |c: char| c.is_digit(8);
                    match chars.peek().copied() {
                        Some(third) if octal(escaped) && octal(next) && octal(third) => {
                            chars.next();
                            let code = u32::from_str_radix(&format!("{escaped}{next}{third}"), 8)
                                .map_err(|_| "re replacement escape")?;
                            if code > 0o377 {
                                return Err("re replacement escape");
                            }
                            text.push(char::from_u32(code).ok_or("re replacement escape")?);
                            continue;
                        }
                        _ => digits.push(next),
                    }
                }
                parts.push(Part::Text(std::mem::take(&mut text)));
                parts.push(group(digits.parse().map_err(|_| "re replacement group")?)?);
                continue;
            }
            'a' => '\u{7}',
            'b' => '\u{8}',
            'f' => '\u{c}',
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            'v' => '\u{b}',
            '\\' => '\\',
            c if c.is_ascii_alphabetic() => return Err("re replacement escape"),
            c => {
                text.push('\\');
                c
            }
        };
        text.push(literal);
    }
    parts.push(Part::Text(text));
    Ok(parts)
}
