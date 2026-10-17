//! Durations for timers and the simulator's virtual clock.

use std::fmt;

use serde::{Deserialize, Serialize};

use super::error::ExprError;

/// A non-negative span of virtual time, in milliseconds.
///
/// Text form: `<integer><unit>` with unit `ms`, `s`, `m`, `h` or `d`.
/// [`Display`](fmt::Display) picks the largest unit that divides exactly
/// (`90s`, `2m`), and zero is `0s`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Duration {
    millis: u64,
}

const UNITS: [(&str, u64); 5] = [("d", 86_400_000), ("h", 3_600_000), ("m", 60_000), ("s", 1_000), ("ms", 1)];

impl Duration {
    pub const ZERO: Duration = Duration { millis: 0 };

    pub const fn from_millis(millis: u64) -> Self {
        Self { millis }
    }

    pub const fn from_secs(secs: u64) -> Self {
        Self { millis: secs.saturating_mul(1_000) }
    }

    pub const fn millis(self) -> u64 {
        self.millis
    }

    pub const fn is_zero(self) -> bool {
        self.millis == 0
    }

    pub const fn checked_add(self, other: Duration) -> Option<Duration> {
        match self.millis.checked_add(other.millis) {
            Some(millis) => Some(Duration { millis }),
            None => None,
        }
    }

    pub const fn saturating_add(self, other: Duration) -> Duration {
        Duration { millis: self.millis.saturating_add(other.millis) }
    }

    pub const fn saturating_sub(self, other: Duration) -> Duration {
        Duration { millis: self.millis.saturating_sub(other.millis) }
    }
}

impl fmt::Display for Duration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.millis == 0 {
            return f.write_str("0s");
        }
        let (unit, size) =
            UNITS.iter().copied().find(|&(_, size)| self.millis.is_multiple_of(size)).unwrap_or(("ms", 1));
        write!(f, "{}{unit}", self.millis / size)
    }
}

impl std::str::FromStr for Duration {
    type Err = ExprError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        parse_duration(text)
    }
}

/// Parse `<integer><unit>`, e.g. `30s`, `1500ms`, `2h`. Surrounding
/// whitespace is allowed; nothing else is.
pub fn parse_duration(text: &str) -> Result<Duration, ExprError> {
    let leading = text.chars().take_while(|c| c.is_whitespace()).count();
    let body = text.trim();
    let digits: String = body.chars().take_while(char::is_ascii_digit).collect();
    let unit = &body[digits.len()..];
    if digits.is_empty() {
        return Err(ExprError::new("a duration starts with a whole number, e.g. `30s`".to_owned(), leading + 1));
    }
    let unit_col = leading + digits.len() + 1;
    let Some(&(_, size)) = UNITS.iter().find(|(name, _)| *name == unit) else {
        let message = if unit.is_empty() {
            "missing unit; use ms, s, m, h or d".to_owned()
        } else {
            format!("unknown unit `{unit}`; use ms, s, m, h or d")
        };
        return Err(ExprError::new(message, unit_col));
    };
    let too_long = || ExprError::new("duration is too long", leading + 1);
    let count: u64 = digits.parse().map_err(|_| too_long())?;
    count.checked_mul(size).map(Duration::from_millis).ok_or_else(too_long)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(text: &str) -> Option<u64> {
        parse_duration(text).ok().map(Duration::millis)
    }

    #[test]
    fn parses_every_unit() {
        assert_eq!(ms("1500ms"), Some(1_500));
        assert_eq!(ms("30s"), Some(30_000));
        assert_eq!(ms("30m"), Some(1_800_000));
        assert_eq!(ms("2h"), Some(7_200_000));
        assert_eq!(ms("1d"), Some(86_400_000));
        assert_eq!(ms(" 5s "), Some(5_000));
        assert_eq!(ms("0s"), Some(0));
    }

    #[test]
    fn rejects_malformed_text_with_columns() {
        let err = |text: &str| parse_duration(text).err().map(|e| (e.message, e.column));
        assert_eq!(err("30"), Some(("missing unit; use ms, s, m, h or d".into(), 3)));
        assert_eq!(err("30 s"), Some(("unknown unit ` s`; use ms, s, m, h or d".into(), 3)));
        assert_eq!(err("30sec"), Some(("unknown unit `sec`; use ms, s, m, h or d".into(), 3)));
        assert_eq!(err("s"), Some(("a duration starts with a whole number, e.g. `30s`".into(), 1)));
        assert_eq!(err("-1s"), Some(("a duration starts with a whole number, e.g. `30s`".into(), 1)));
        assert_eq!(err("1.5s"), Some(("unknown unit `.5s`; use ms, s, m, h or d".into(), 2)));
        assert_eq!(err(""), Some(("a duration starts with a whole number, e.g. `30s`".into(), 1)));
        assert!(err("99999999999999999999d").is_some());
        assert!(err("999999999999999d").is_some(), "overflow");
    }

    #[test]
    fn display_picks_the_largest_exact_unit() {
        let show = |millis| Duration::from_millis(millis).to_string();
        assert_eq!(show(0), "0s");
        assert_eq!(show(1), "1ms");
        assert_eq!(show(1_500), "1500ms");
        assert_eq!(show(90_000), "90s");
        assert_eq!(show(120_000), "2m");
        assert_eq!(show(5_400_000), "90m");
        assert_eq!(show(7_200_000), "2h");
        assert_eq!(show(172_800_000), "2d");
    }

    #[test]
    fn display_round_trips() {
        for millis in [0, 1, 999, 1_000, 61_000, 3_600_000, 86_400_000, 90_061_001, u64::MAX / 2] {
            let d = Duration::from_millis(millis);
            assert_eq!(parse_duration(&d.to_string()), Ok(d), "{millis}");
        }
    }

    #[test]
    fn arithmetic() {
        let a = Duration::from_secs(30);
        assert_eq!(a.checked_add(a), Some(Duration::from_secs(60)));
        assert_eq!(Duration::from_millis(u64::MAX).checked_add(Duration::from_millis(1)), None);
        assert_eq!(a.saturating_sub(Duration::from_secs(40)), Duration::ZERO);
        assert!(Duration::ZERO.is_zero());
    }
}
