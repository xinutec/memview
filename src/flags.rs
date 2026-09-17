//! Command-line flags that carry a VALUE, read so that a bad one is refused.
//!
//! ⚠ **`.parse().ok().unwrap_or(default)` makes `--half-life bogus` produce output
//! BYTE-IDENTICAL to passing no flag** — the flag is accepted and its value thrown
//! away. `memory-rank` and `memory-tiers` produce the figures quoted in task
//! bodies, so that is a wrong measurement rather than a wrong run.
//!
//! ⚠ **Absent and unusable are DIFFERENT.** No flag means "the default is what I
//! want" and stays silent; a missing or unparseable value means the caller asked
//! for something the tool cannot do, and falling back is how they come to believe
//! a number nobody computed.

use std::str::FromStr;

use anyhow::{Result, bail};

/// The value of `name`, parsed, or `default` when the flag is absent.
///
/// Refuses — never falls back — when the flag is present but its value is missing
/// or will not parse.
pub fn value_of<T>(args: &[String], name: &str, default: T) -> Result<T>
where
    T: FromStr,
    T::Err: std::fmt::Display,
{
    let Some(at) = args.iter().position(|a| a == name) else {
        return Ok(default);
    };
    let Some(raw) = args.get(at + 1) else {
        bail!("{name} needs a value, and none followed it");
    };
    // ⚠ A following FLAG is a missing value, not a value. `--breadth --lease-days
    // 30` would otherwise try to parse `--lease-days` and report it as a bad
    // number, which sends the reader to the wrong flag.
    if raw.starts_with("--") {
        bail!("{name} needs a value, but the next argument is the flag {raw}");
    }
    match raw.parse() {
        Ok(v) => Ok(v),
        Err(why) => bail!("{name}: {raw:?} is not usable — {why}"),
    }
}

/// Refuse any `--flag` this tool does not know, naming the ones it does.
///
/// `value_of` above fixes the HONOURED set; this fixes the ACCEPTED one.
/// Measured: `memory-rank --nonsense` exited 0 with output byte-identical to no
/// arguments, and so did every other binary here.
///
/// Naming the known flags is most of what `--help` would give, for a caller who
/// mistyped one. Deliberately not a `--help` — that is open on `dev-lint#761`.
///
/// `--` ends the flags (memview#1525); `args` includes `argv[0]`.
pub fn reject_unknown(args: &[String], known: &[&str]) -> Result<()> {
    for arg in args.iter().skip(1) {
        if arg == "--" {
            break;
        }
        // A lone `-` is an operand by long convention (stdin), not a flag.
        if !arg.starts_with("--") {
            continue;
        }
        // `--flag=value` is the same flag as `--flag`.
        let name = arg.split_once('=').map_or(arg.as_str(), |(n, _)| n);
        if known.contains(&name) {
            continue;
        }
        let takes = if known.is_empty() {
            "it takes no flags".to_string()
        } else {
            format!("it takes {}", known.join(", "))
        };
        bail!("unknown flag {name} — {takes}");
    }
    Ok(())
}
