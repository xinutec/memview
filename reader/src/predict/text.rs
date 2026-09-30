//! The text tools a pipeline is made of, reimplemented: what `grep`, `head`,
//! `tail`, `cut`, `tr`, `uniq`, `sort`, `basename` and `dirname` print for
//! an input this knows. Safe because nothing here runs them. What differs
//! between the GNU and BSD tools, or with the locale, is refused by name:
//! `wc`'s padding, `sort`'s collation without `LC_ALL=C`, a case or a
//! character class over text that is not ASCII.

use super::sed;

/// Why a tool is not followed, as the census names it.
pub(super) type Refused = String;

/// One input to a tool: the file it came from (`None` for stdin), and its text.
pub(super) struct Input {
    pub name: Option<String>,
    pub text: String,
}

/// What a tool prints, and for `grep` whether it selected a line: its status.
pub(super) struct Printed {
    pub text: String,
    pub selected: Option<bool>,
}

fn printed(text: String) -> Printed {
    Printed {
        text,
        selected: None,
    }
}

/// The lines of a text, without their newlines; a last line without one is
/// still a line.
fn lines(text: &str) -> Vec<&str> {
    let body = text.strip_suffix('\n').unwrap_or(text);
    if text.is_empty() {
        Vec::new()
    } else {
        body.split('\n').collect()
    }
}

/// The operands of a tool that takes files, and its flags. Flags come first;
/// `--` ends them.
pub(super) fn split(args: &[String]) -> (Vec<&String>, Vec<&String>) {
    let mut flags = Vec::new();
    let mut operands = Vec::new();
    let mut ended = false;
    for arg in args {
        if !ended && arg == "--" {
            ended = true;
        } else if !ended && arg.starts_with('-') && arg.len() > 1 && operands.is_empty() {
            flags.push(arg);
        } else {
            operands.push(arg);
        }
    }
    (flags, operands)
}

/// `grep` over its inputs: the lines it prints and whether any was selected.
pub(super) fn grep(flags: &[&String], pattern: &str, inputs: &[Input]) -> Result<Printed, Refused> {
    let (mut quiet, mut invert, mut ignore_case, mut count, mut number, mut only) =
        (false, false, false, false, false, false);
    let (mut fixed, mut extended, mut whole, mut files_only) = (false, false, false, false);
    let mut names: Option<bool> = None;
    for flag in flags {
        for letter in flag[1..].chars() {
            match letter {
                'q' => quiet = true,
                // Errors about files unread: none, since each was read.
                's' => {}
                'v' => invert = true,
                'i' => ignore_case = true,
                'c' => count = true,
                'n' => number = true,
                'o' => only = true,
                'F' => fixed = true,
                'E' => extended = true,
                'x' => whole = true,
                'l' => files_only = true,
                'h' => names = Some(false),
                'H' => names = Some(true),
                other => return Err(format!("grep -{other}")),
            }
        }
    }
    if ignore_case && (!pattern.is_ascii() || inputs.iter().any(|input| !input.text.is_ascii())) {
        return Err("grep -i over text that is not ASCII".to_string());
    }
    if only && (invert || whole) {
        return Err("grep -o".to_string());
    }
    let matcher = if fixed {
        None
    } else {
        Some(sed::compile(pattern, extended, ignore_case).map_err(|why| format!("grep {why}"))?)
    };
    let folded = |text: &str| {
        if ignore_case {
            text.to_ascii_lowercase()
        } else {
            text.to_string()
        }
    };
    let needle = folded(pattern);
    let hits = |line: &str| -> Vec<(usize, usize)> {
        match &matcher {
            None => {
                let hay = folded(line);
                if whole {
                    return if hay == needle {
                        vec![(0, line.len())]
                    } else {
                        Vec::new()
                    };
                }
                if needle.is_empty() {
                    return vec![(0, 0)];
                }
                hay.match_indices(needle.as_str())
                    .map(|(at, _)| (at, at + needle.len()))
                    .collect()
            }
            // The match is leftmost and the longest from there, so the line is
            // matched whole exactly when it starts at 0 and reaches the end.
            Some(matcher) if whole => match matcher.find(line, 0) {
                Some((0, end)) if end == line.len() => vec![(0, end)],
                _ => Vec::new(),
            },
            Some(matcher) => {
                let mut found = Vec::new();
                let mut from = 0;
                while from <= line.len() {
                    let Some((start, end)) = matcher.find(line, from) else {
                        break;
                    };
                    found.push((start, end));
                    from = if end > start { end } else { end + 1 };
                    while from < line.len() && !line.is_char_boundary(from) {
                        from += 1;
                    }
                }
                found
            }
        }
    };
    let show_names = names.unwrap_or(inputs.len() > 1);
    let mut out = String::new();
    let mut any = false;
    for input in inputs {
        let name = input.name.as_deref().unwrap_or("(standard input)");
        let prefix = |n: usize| {
            let mut prefix = String::new();
            if show_names {
                prefix.push_str(name);
                prefix.push(':');
            }
            if number {
                prefix.push_str(&format!("{n}:"));
            }
            prefix
        };
        let mut selected = 0usize;
        for (at, line) in lines(&input.text).into_iter().enumerate() {
            let found = hits(line);
            if found.is_empty() == invert {
                selected += 1;
                if quiet || count || files_only {
                    continue;
                }
                if only {
                    for (start, end) in found.into_iter().filter(|(s, e)| e > s) {
                        out.push_str(&prefix(at + 1));
                        out.push_str(&line[start..end]);
                        out.push('\n');
                    }
                } else {
                    out.push_str(&prefix(at + 1));
                    out.push_str(line);
                    out.push('\n');
                }
            }
        }
        any |= selected > 0;
        if count && !quiet && !files_only {
            if show_names {
                out.push_str(name);
                out.push(':');
            }
            out.push_str(&format!("{selected}\n"));
        }
        if files_only && selected > 0 && !quiet {
            out.push_str(name);
            out.push('\n');
        }
    }
    Ok(Printed {
        text: if quiet { String::new() } else { out },
        selected: Some(any),
    })
}

/// A count as `head` and `tail` spell it: `-n N`, `-nN`, `-N`.
fn count(
    flags: &[&String],
    operands: &mut Vec<&String>,
    tool: &str,
) -> Result<(usize, bool), Refused> {
    let mut lines = 10usize;
    let mut from_start = false;
    let mut at = 0;
    while at < flags.len() {
        let flag = flags[at].as_str();
        let value = if flag == "-n" {
            at += 1;
            flags.get(at).map(|v| v.as_str()).or_else(|| {
                if operands.is_empty() {
                    None
                } else {
                    Some(operands.remove(0).as_str())
                }
            })
        } else if let Some(rest) = flag.strip_prefix("-n") {
            Some(rest)
        } else if flag[1..].chars().all(|c| c.is_ascii_digit()) {
            Some(&flag[1..])
        } else {
            return Err(format!("{tool} {flag}"));
        };
        let value = value.ok_or_else(|| format!("{tool} -n"))?;
        let (plus, digits) = match value.strip_prefix('+') {
            Some(digits) => (true, digits),
            None => (false, value),
        };
        if plus && tool == "head" {
            return Err("head -n +".to_string());
        }
        from_start = plus;
        lines = digits.parse().map_err(|_| format!("{tool} -n {value}"))?;
        at += 1;
    }
    Ok((lines, from_start))
}

/// The lines, each with the newline it had.
fn with_newlines(text: &str) -> Vec<&str> {
    text.split_inclusive('\n').collect()
}

pub(super) fn head(
    flags: &[&String],
    operands: &mut Vec<&String>,
    input: &str,
) -> Result<Printed, Refused> {
    let (n, _) = count(flags, operands, "head")?;
    Ok(printed(with_newlines(input).into_iter().take(n).collect()))
}

pub(super) fn tail(
    flags: &[&String],
    operands: &mut Vec<&String>,
    input: &str,
) -> Result<Printed, Refused> {
    let (n, from_start) = count(flags, operands, "tail")?;
    let all = with_newlines(input);
    let kept = if from_start {
        all.into_iter().skip(n.saturating_sub(1)).collect()
    } else {
        let skip = all.len().saturating_sub(n);
        all.into_iter().skip(skip).collect()
    };
    Ok(printed(kept))
}

/// A `cut` list: `N`, `N-M`, `N-`, `-M`, comma-separated, from 1.
fn ranges(list: &str) -> Result<Vec<(usize, usize)>, Refused> {
    list.split(',')
        .map(|part| {
            let bad = || format!("cut list {list}");
            let number = |text: &str, absent: usize| {
                if text.is_empty() {
                    Ok(absent)
                } else {
                    text.parse::<usize>()
                        .map_err(|_| bad())
                        .and_then(|n| if n == 0 { Err(bad()) } else { Ok(n) })
                }
            };
            match part.split_once('-') {
                Some((from, to)) => Ok((number(from, 1)?, number(to, usize::MAX)?)),
                None => {
                    let n = number(part, 0)?;
                    Ok((n, n))
                }
            }
        })
        .collect()
}

pub(super) fn cut(args: &[String], input: &str) -> Result<Printed, Refused> {
    let mut delimiter = '\t';
    let (mut fields, mut chars) = (None, None);
    let mut at = 0;
    let value = |at: &mut usize, flag: &str, rest: &str| -> Result<String, Refused> {
        if rest.is_empty() {
            *at += 1;
            args.get(*at).cloned().ok_or_else(|| format!("cut {flag}"))
        } else {
            Ok(rest.to_string())
        }
    };
    while at < args.len() {
        let arg = args[at].as_str();
        if let Some(rest) = arg.strip_prefix("-d") {
            let text = value(&mut at, "-d", rest)?;
            let mut it = text.chars();
            delimiter = match (it.next(), it.next()) {
                (Some(c), None) => c,
                _ => return Err("cut -d".to_string()),
            };
        } else if let Some(rest) = arg.strip_prefix("-f") {
            fields = Some(ranges(&value(&mut at, "-f", rest)?)?);
        } else if let Some(rest) = arg.strip_prefix("-c") {
            chars = Some(ranges(&value(&mut at, "-c", rest)?)?);
        } else {
            return Err(format!("cut {arg}"));
        }
        at += 1;
    }
    let within =
        |ranges: &[(usize, usize)], n: usize| ranges.iter().any(|(a, b)| *a <= n && n <= *b);
    let mut out = String::new();
    for line in lines(input) {
        match (&fields, &chars) {
            (Some(fields), None) => {
                if !line.contains(delimiter) {
                    out.push_str(line);
                } else {
                    let kept: Vec<&str> = line
                        .split(delimiter)
                        .enumerate()
                        .filter(|(n, _)| within(fields, n + 1))
                        .map(|(_, field)| field)
                        .collect();
                    out.push_str(&kept.join(&delimiter.to_string()));
                }
            }
            (None, Some(chars)) => {
                // GNU counts bytes, BSD characters: alike only for ASCII.
                if !line.is_ascii() {
                    return Err("cut -c over text that is not ASCII".to_string());
                }
                out.extend(
                    line.chars()
                        .enumerate()
                        .filter(|(n, _)| within(chars, n + 1))
                        .map(|(_, c)| c),
                );
            }
            _ => return Err("cut".to_string()),
        }
        out.push('\n');
    }
    Ok(printed(out))
}

/// A `tr` set, expanded: literal characters, ranges, a few escapes and the
/// ASCII classes.
fn set(spec: &str) -> Result<Vec<char>, Refused> {
    let class = |name: &str| -> Option<Vec<char>> {
        let range = |a: u8, b: u8| (a..=b).map(char::from).collect::<Vec<_>>();
        Some(match name {
            "upper" => range(b'A', b'Z'),
            "lower" => range(b'a', b'z'),
            "digit" => range(b'0', b'9'),
            "alpha" => [range(b'A', b'Z'), range(b'a', b'z')].concat(),
            "alnum" => [range(b'0', b'9'), range(b'A', b'Z'), range(b'a', b'z')].concat(),
            "space" => vec![' ', '\t', '\n', '\u{b}', '\u{c}', '\r'],
            _ => return None,
        })
    };
    let chars: Vec<char> = spec.chars().collect();
    let mut out = Vec::new();
    let mut at = 0;
    while at < chars.len() {
        if chars[at] == '[' && chars.get(at + 1) == Some(&':') {
            let end =
                (at + 2..chars.len()).find(|&i| chars[i] == ':' && chars.get(i + 1) == Some(&']'));
            let Some(end) = end else {
                return Err("tr class".to_string());
            };
            let name: String = chars[at + 2..end].iter().collect();
            out.extend(class(&name).ok_or_else(|| format!("tr [:{name}:]"))?);
            at = end + 2;
            continue;
        }
        let c = if chars[at] == '\\' {
            at += 1;
            match chars.get(at) {
                Some('n') => '\n',
                Some('t') => '\t',
                Some('\\') => '\\',
                _ => return Err("tr escape".to_string()),
            }
        } else {
            chars[at]
        };
        if chars.get(at + 1) == Some(&'-') && at + 2 < chars.len() {
            let end = chars[at + 2];
            if end < c {
                return Err("tr range".to_string());
            }
            out.extend(c..=end);
            at += 3;
        } else {
            out.push(c);
            at += 1;
        }
    }
    if !out.iter().all(char::is_ascii) {
        return Err("tr over characters that are not ASCII".to_string());
    }
    Ok(out)
}

pub(super) fn tr(args: &[String], input: &str) -> Result<Printed, Refused> {
    if !input.is_ascii() {
        return Err("tr over text that is not ASCII".to_string());
    }
    match args {
        [flag, from] if flag == "-d" => {
            let from = set(from)?;
            Ok(printed(
                input.chars().filter(|c| !from.contains(c)).collect(),
            ))
        }
        [from, to] if !from.starts_with('-') => {
            let (from, to) = (set(from)?, set(to)?);
            let Some(&last) = to.last() else {
                return Err("tr empty".to_string());
            };
            Ok(printed(
                input
                    .chars()
                    .map(|c| match from.iter().rposition(|f| *f == c) {
                        Some(at) => to.get(at).copied().unwrap_or(last),
                        None => c,
                    })
                    .collect(),
            ))
        }
        _ => Err(format!("tr {}", args.first().map_or("", String::as_str))),
    }
}

pub(super) fn uniq(flags: &[&String], input: &str) -> Result<Printed, Refused> {
    let (mut repeated, mut unique) = (false, false);
    for flag in flags {
        match flag.as_str() {
            "-d" => repeated = true,
            "-u" => unique = true,
            other => return Err(format!("uniq {other}")),
        }
    }
    let mut out = String::new();
    let all = lines(input);
    let mut at = 0;
    while at < all.len() {
        let run = all[at..]
            .iter()
            .take_while(|line| **line == all[at])
            .count();
        if (!repeated || run > 1) && (!unique || run == 1) {
            out.push_str(all[at]);
            out.push('\n');
        }
        at += run;
    }
    Ok(printed(out))
}

/// `sort`, in byte order: only under `LC_ALL=C`, where that is its order.
pub(super) fn sort(flags: &[&String], input: &str) -> Result<Printed, Refused> {
    let (mut reverse, mut unique) = (false, false);
    for flag in flags {
        for letter in flag[1..].chars() {
            match letter {
                'r' => reverse = true,
                'u' => unique = true,
                other => return Err(format!("sort -{other}")),
            }
        }
    }
    let mut all = lines(input);
    all.sort_unstable();
    if unique {
        all.dedup();
    }
    if reverse {
        all.reverse();
    }
    Ok(printed(
        all.into_iter().map(|line| format!("{line}\n")).collect(),
    ))
}

/// POSIX `basename NAME [SUFFIX]`.
pub(super) fn basename(args: &[String]) -> Result<Printed, Refused> {
    let (name, suffix) = match args {
        [name] => (name, None),
        [name, suffix] => (name, Some(suffix)),
        _ => return Err("basename".to_string()),
    };
    let trimmed = name.trim_end_matches('/');
    if trimmed.is_empty() {
        return Ok(printed(
            if name.is_empty() { "\n" } else { "/\n" }.to_string(),
        ));
    }
    let mut base = trimmed.rsplit('/').next().unwrap_or(trimmed).to_string();
    if let Some(suffix) = suffix
        && base != *suffix
        && let Some(stripped) = base.strip_suffix(suffix.as_str())
    {
        base = stripped.to_string();
    }
    Ok(printed(format!("{base}\n")))
}

/// POSIX `dirname NAME`.
pub(super) fn dirname(args: &[String]) -> Result<Printed, Refused> {
    let [name] = args else {
        return Err("dirname".to_string());
    };
    let trimmed = name.trim_end_matches('/');
    if trimmed.is_empty() {
        return Ok(printed(
            if name.is_empty() { ".\n" } else { "/\n" }.to_string(),
        ));
    }
    let dir = match trimmed.rfind('/') {
        None => ".",
        Some(at) => {
            let dir = trimmed[..at].trim_end_matches('/');
            if dir.is_empty() { "/" } else { dir }
        }
    };
    Ok(printed(format!("{dir}\n")))
}
