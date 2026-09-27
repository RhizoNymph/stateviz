//! The machine color names a definition may use.
//!
//! Hue means entity and nothing else, so the only colors a definition can
//! name are the eight Okabe-Ito colorblind-safe hues. Mapping these to
//! concrete RGB values (and deriving lightness variants past eight machines)
//! is presentation and lives in `cascade-scene`.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PaletteColor {
    Orange,
    SkyBlue,
    Green,
    Yellow,
    Blue,
    Vermillion,
    Purple,
    Black,
}

impl PaletteColor {
    /// All colors in Okabe-Ito order, which is also the automatic assignment
    /// order for machines without a declared color.
    pub const ALL: [PaletteColor; 8] = [
        PaletteColor::Blue,
        PaletteColor::Orange,
        PaletteColor::Green,
        PaletteColor::Vermillion,
        PaletteColor::SkyBlue,
        PaletteColor::Purple,
        PaletteColor::Yellow,
        PaletteColor::Black,
    ];

    /// The canonical name, as written back to YAML.
    pub const fn name(self) -> &'static str {
        match self {
            PaletteColor::Orange => "orange",
            PaletteColor::SkyBlue => "sky-blue",
            PaletteColor::Green => "green",
            PaletteColor::Yellow => "yellow",
            PaletteColor::Blue => "blue",
            PaletteColor::Vermillion => "vermillion",
            PaletteColor::Purple => "purple",
            PaletteColor::Black => "black",
        }
    }
}

impl fmt::Display for PaletteColor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// The name is not one of the palette colors or their aliases.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("unknown color `{0}`; expected one of: orange, sky-blue, green, yellow, blue, vermillion, purple, black")]
pub struct UnknownColor(pub String);

impl FromStr for PaletteColor {
    type Err = UnknownColor;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let normalized: String =
            s.trim().chars().filter(|c| !matches!(c, '-' | '_' | ' ')).flat_map(char::to_lowercase).collect();
        match normalized.as_str() {
            "orange" => Ok(PaletteColor::Orange),
            "skyblue" | "lightblue" | "cyan" => Ok(PaletteColor::SkyBlue),
            "green" | "bluishgreen" | "teal" => Ok(PaletteColor::Green),
            "yellow" => Ok(PaletteColor::Yellow),
            "blue" => Ok(PaletteColor::Blue),
            "vermillion" | "vermilion" | "red" => Ok(PaletteColor::Vermillion),
            "purple" | "reddishpurple" | "pink" | "magenta" => Ok(PaletteColor::Purple),
            "black" | "gray" | "grey" => Ok(PaletteColor::Black),
            _ => Err(UnknownColor(s.to_owned())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_canonical_names_and_aliases() {
        for color in PaletteColor::ALL {
            assert_eq!(color.name().parse::<PaletteColor>(), Ok(color));
        }
        assert_eq!("Sky_Blue".parse(), Ok(PaletteColor::SkyBlue));
        assert_eq!("red".parse(), Ok(PaletteColor::Vermillion));
        assert_eq!("grey".parse(), Ok(PaletteColor::Black));
    }

    #[test]
    fn rejects_unknown_names() {
        assert_eq!("chartreuse".parse::<PaletteColor>(), Err(UnknownColor("chartreuse".to_owned())));
    }
}
