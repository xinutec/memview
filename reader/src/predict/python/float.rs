//! Python's floats as CPython prints and rounds them. Its arithmetic is the
//! machine's IEEE doubles, as Rust's is; what is CPython's own is the text.
//! Held to what CPython printed by `tests/suite/floats.rs`.

/// `repr(x)`, which `str(x)`, an f-string field and `json.dumps` share: the
/// shortest digits that read back as `x`, in exponent form below `1e-4` and
/// from `1e16`, with at least two exponent digits and a `.0` on a whole number.
pub(super) fn repr(x: f64) -> String {
    if x.is_nan() {
        return "nan".to_string();
    }
    if x.is_infinite() {
        return if x > 0.0 { "inf" } else { "-inf" }.to_string();
    }
    if x == 0.0 {
        return if x.is_sign_negative() { "-0.0" } else { "0.0" }.to_string();
    }
    // Rust's `{:e}` is the shortest digits that round-trip, as CPython's are.
    let scientific = format!("{x:e}");
    let Some((mantissa, exponent)) = scientific.split_once('e') else {
        return scientific;
    };
    let Ok(exponent) = exponent.parse::<i32>() else {
        return scientific;
    };
    let (sign, mantissa) = match mantissa.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", mantissa),
    };
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    if !(-4..16).contains(&exponent) {
        let (first, rest) = digits.split_at(1);
        let fraction = if rest.is_empty() {
            String::new()
        } else {
            format!(".{rest}")
        };
        let mark = if exponent < 0 { '-' } else { '+' };
        return format!(
            "{sign}{first}{fraction}e{mark}{:02}",
            exponent.unsigned_abs()
        );
    }
    if exponent < 0 {
        let zeros = "0".repeat(exponent.unsigned_abs() as usize - 1);
        return format!("{sign}0.{zeros}{digits}");
    }
    let point = exponent as usize + 1;
    if digits.len() <= point {
        let zeros = "0".repeat(point - digits.len());
        format!("{sign}{digits}{zeros}.0")
    } else {
        format!("{sign}{}.{}", &digits[..point], &digits[point..])
    }
}

/// `f"{x:.{places}f}"`: the exact value rounded half to even at `places`.
pub(super) fn fixed(x: f64, places: usize) -> String {
    if x.is_nan() {
        return "nan".to_string();
    }
    if x.is_infinite() {
        return if x > 0.0 { "inf" } else { "-inf" }.to_string();
    }
    format!("{x:.places$}")
}

/// `round(x, places)`: the nearest double to `x` rounded at `places`, which is
/// how CPython does it, through the correctly rounded decimal text.
pub(super) fn round(x: f64, places: i64) -> Option<f64> {
    if !x.is_finite() {
        return Some(x);
    }
    let places = usize::try_from(places)
        .ok()
        .filter(|places| *places <= 30)?;
    fixed(x, places).parse().ok()
}

/// A float literal as Python reads it: digits with a point or an exponent,
/// underscores between digits allowed. `None` for anything else.
pub(super) fn literal(text: &str) -> Option<f64> {
    let plain: String = text.chars().filter(|c| *c != '_').collect();
    let decimal = plain
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-'));
    let shaped = plain.contains(['.', 'e', 'E'])
        && plain.starts_with(|c: char| c.is_ascii_digit() || c == '.');
    (decimal && shaped).then(|| plain.parse().ok()).flatten()
}
