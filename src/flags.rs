//! Command-line flags that carry a VALUE, read so that a bad one is refused.
//!
//! ⚠ **The defect this exists to stop, measured 2026-09-13.** Every value flag in
//! this repo was read as
//!
//! ```ignore
//! args.iter().position(|a| a == "--half-life")
//!     .and_then(|i| args.get(i + 1))
//!     .and_then(|n| n.parse().ok())
//!     .unwrap_or(HALF_LIFE_DAYS)
//! ```
//!
//! so `--half-life bogus` produced output BYTE-IDENTICAL to passing no flag at
//! all, while `--half-life 3` differed — the flag was accepted, its value thrown
//! away, and a default-parameterised number printed as though it were the one
//! asked for. These tools produce the figures quoted in task bodies
//! (`memory-rank`, `memory-tiers`), so a silently ignored parameter is a wrong
//! measurement nobody can see, not merely a wrong run.
//!
//! ⚠ **Absent and unusable are DIFFERENT, and that is the whole design.** No flag
//! means "the default is what I want" and must stay silent. A flag with a missing
//! or unparseable value means the caller asked for something the tool cannot do,
//! and the only safe answer is to refuse — falling back is how the caller comes to
//! believe a number that was never computed.
//!
//! Related in kind to `dev-lint#768`, where `totality-checks` DROPPED any `--`
//! word it did not recognise and exited 0. Same family: the accepted set and the
//! honoured set were two different lists.

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
