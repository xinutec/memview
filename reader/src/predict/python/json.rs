//! JSON as Python's `json` module reads and writes it, over the evaluator's
//! values: what `json.load` makes of a shown file, and the exact text
//! `json.dump` writes. Written here rather than borrowed so that its rules are
//! Python's: an object keeps its keys in file order, a repeated key keeps its
//! first place and its last value, and `json.dumps` escapes as CPython does.
//! A number with a fraction or an exponent is refused, since the evaluator has
//! no floats, and so are `NaN` and `Infinity`, which Python accepts.

use super::Value;

/// `json.loads(text)`, or why it is refused. `Err(None)` is text Python
/// itself would reject: the call raises.
pub(super) fn parse(text: &str) -> Result<Value, Option<String>> {
    let mut reader = Reader {
        chars: text.chars().collect(),
        at: 0,
    };
    reader.space();
    let value = reader.value()?;
    reader.space();
    if reader.at < reader.chars.len() {
        return Err(None);
    }
    Ok(value)
}

struct Reader {
    chars: Vec<char>,
    at: usize,
}

impl Reader {
    fn space(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t' | '\n' | '\r')) {
            self.at += 1;
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn eat(&mut self, word: &str) -> bool {
        let end = self.at + word.chars().count();
        if end <= self.chars.len() && self.chars[self.at..end].iter().copied().eq(word.chars()) {
            self.at = end;
            true
        } else {
            false
        }
    }

    fn value(&mut self) -> Result<Value, Option<String>> {
        match self.peek() {
            Some('{') => self.object(),
            Some('[') => self.array(),
            Some('"') => self.string().map(Value::Str),
            Some('-' | '0'..='9') => self.number(),
            _ if self.eat("true") => Ok(Value::Bool(true)),
            _ if self.eat("false") => Ok(Value::Bool(false)),
            _ if self.eat("null") => Ok(Value::None),
            _ if self.eat("NaN") || self.eat("Infinity") || self.eat("-Infinity") => {
                Err(Some("json float".to_string()))
            }
            _ => Err(None),
        }
    }

    fn object(&mut self) -> Result<Value, Option<String>> {
        self.at += 1;
        let mut pairs: Vec<(Value, Value)> = Vec::new();
        self.space();
        if self.eat("}") {
            return Ok(Value::Dict(pairs));
        }
        loop {
            self.space();
            if self.peek() != Some('"') {
                return Err(None);
            }
            let key = Value::Str(self.string()?);
            self.space();
            if !self.eat(":") {
                return Err(None);
            }
            self.space();
            let value = self.value()?;
            match pairs.iter_mut().find(|(seen, _)| *seen == key) {
                Some((_, held)) => *held = value,
                None => pairs.push((key, value)),
            }
            self.space();
            if self.eat("}") {
                return Ok(Value::Dict(pairs));
            }
            if !self.eat(",") {
                return Err(None);
            }
        }
    }

    fn array(&mut self) -> Result<Value, Option<String>> {
        self.at += 1;
        let mut items = Vec::new();
        self.space();
        if self.eat("]") {
            return Ok(Value::Tuple(items));
        }
        loop {
            self.space();
            items.push(self.value()?);
            self.space();
            if self.eat("]") {
                return Ok(Value::Tuple(items));
            }
            if !self.eat(",") {
                return Err(None);
            }
        }
    }

    fn number(&mut self) -> Result<Value, Option<String>> {
        let start = self.at;
        self.eat("-");
        if self.eat("Infinity") {
            return Err(Some("json float".to_string()));
        }
        match self.peek() {
            Some('0') => self.at += 1,
            Some('1'..='9') => {
                while matches!(self.peek(), Some('0'..='9')) {
                    self.at += 1;
                }
            }
            _ => return Err(None),
        }
        if matches!(self.peek(), Some('.' | 'e' | 'E')) {
            return Err(Some("json float".to_string()));
        }
        let digits: String = self.chars[start..self.at].iter().collect();
        digits
            .parse()
            .map(Value::Int)
            .map_err(|_| Some("json int".to_string()))
    }

    /// A string's text; `Err(None)` where Python raises (a raw control
    /// character, an unknown escape, a string never closed).
    fn string(&mut self) -> Result<String, Option<String>> {
        self.at += 1;
        let mut out = String::new();
        loop {
            let c = self.peek().ok_or(None)?;
            self.at += 1;
            match c {
                '"' => return Ok(out),
                '\\' => {
                    let escaped = self.peek().ok_or(None)?;
                    self.at += 1;
                    match escaped {
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        '/' => out.push('/'),
                        'b' => out.push('\u{8}'),
                        'f' => out.push('\u{c}'),
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'u' => {
                            let high = self.hex()?;
                            let unit = if (0xD800..0xDC00).contains(&high) && self.eat("\\u") {
                                let low = self.hex()?;
                                if (0xDC00..0xE000).contains(&low) {
                                    0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00)
                                } else {
                                    // Python keeps a lone surrogate, which a
                                    // Rust string cannot hold.
                                    return Err(Some("json surrogate".to_string()));
                                }
                            } else {
                                high
                            };
                            out.push(
                                char::from_u32(unit)
                                    .ok_or_else(|| Some("json surrogate".to_string()))?,
                            );
                        }
                        _ => return Err(None),
                    }
                }
                c if (c as u32) < 0x20 => return Err(None),
                c => out.push(c),
            }
        }
    }

    fn hex(&mut self) -> Result<u32, Option<String>> {
        let end = self.at + 4;
        let digits: String = self.chars.get(self.at..end).ok_or(None)?.iter().collect();
        self.at = end;
        u32::from_str_radix(&digits, 16).map_err(|_| None)
    }
}

/// How `json.dump` lays text out.
pub(super) struct Layout {
    /// `indent=`: `None` for one line, else the text each level repeats.
    pub indent: Option<String>,
    /// `separators=`: between items and between a key and its value.
    pub item: String,
    pub key: String,
    pub sort_keys: bool,
    pub ensure_ascii: bool,
}

impl Layout {
    /// Python's defaults for these: `', '` between items on one line, `','`
    /// once an indent breaks them.
    pub fn new(indent: Option<String>) -> Self {
        Self {
            item: if indent.is_some() { "," } else { ", " }.to_string(),
            indent,
            key: ": ".to_string(),
            sort_keys: false,
            ensure_ascii: true,
        }
    }
}

/// `json.dumps(value)` under `layout`, or the construct refused.
pub(super) fn dump(value: &Value, layout: &Layout) -> Result<String, String> {
    let mut out = String::new();
    write(value, layout, 0, &mut out)?;
    Ok(out)
}

fn write(value: &Value, layout: &Layout, depth: usize, out: &mut String) -> Result<(), String> {
    match value {
        Value::None => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Int(n) => out.push_str(&n.to_string()),
        Value::Str(text) => quote(text, layout.ensure_ascii, out),
        Value::Tuple(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return Ok(());
            }
            out.push('[');
            for (at, item) in items.iter().enumerate() {
                open_line(layout, depth + 1, at > 0, out);
                write(item, layout, depth + 1, out)?;
            }
            close_line(layout, depth, out);
            out.push(']');
        }
        Value::Dict(pairs) => {
            if pairs.is_empty() {
                out.push_str("{}");
                return Ok(());
            }
            let mut keyed = Vec::with_capacity(pairs.len());
            for (key, value) in pairs {
                keyed.push((key_text(key)?, value));
            }
            // Python sorts by the keys themselves, before they become text.
            if layout.sort_keys {
                if !pairs.iter().all(|(key, _)| matches!(key, Value::Str(_))) {
                    return Err("json sort_keys".to_string());
                }
                keyed.sort_by(|a, b| a.0.cmp(&b.0));
            }
            out.push('{');
            for (at, (key, value)) in keyed.iter().enumerate() {
                open_line(layout, depth + 1, at > 0, out);
                quote(key, layout.ensure_ascii, out);
                out.push_str(&layout.key);
                write(value, layout, depth + 1, out)?;
            }
            close_line(layout, depth, out);
            out.push('}');
        }
        Value::Unknown(_) => return Err("json value".to_string()),
        _ => return Err("json type".to_string()),
    }
    Ok(())
}

/// A key as JSON writes it: a string as itself, and the scalars Python
/// converts. A key of another type raises in Python.
fn key_text(key: &Value) -> Result<String, String> {
    match key {
        Value::Str(text) => Ok(text.clone()),
        Value::Int(n) => Ok(n.to_string()),
        Value::Bool(true) => Ok("true".to_string()),
        Value::Bool(false) => Ok("false".to_string()),
        Value::None => Ok("null".to_string()),
        _ => Err("json key".to_string()),
    }
}

fn open_line(layout: &Layout, depth: usize, after: bool, out: &mut String) {
    if after {
        out.push_str(&layout.item);
    }
    if let Some(indent) = &layout.indent {
        out.push('\n');
        out.push_str(&indent.repeat(depth));
    }
}

fn close_line(layout: &Layout, depth: usize, out: &mut String) {
    if let Some(indent) = &layout.indent {
        out.push('\n');
        out.push_str(&indent.repeat(depth));
    }
}

/// A string in quotes, escaped as CPython's encoder does.
fn quote(text: &str, ensure_ascii: bool, out: &mut String) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 || (ensure_ascii && (c as u32) > 0x7e) => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    out.push_str(&format!("\\u{unit:04x}"));
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
}
