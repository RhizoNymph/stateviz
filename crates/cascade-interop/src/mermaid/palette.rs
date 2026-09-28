//! Machine hues for the causal flowchart.
//!
//! Mirrors the app's assignment: a declared color wins; undeclared machines
//! take the first palette hue no machine uses yet, in Okabe-Ito order,
//! cycling once all eight are taken. (The app additionally shades hues by
//! domain past eight machines; Mermaid output does not.)

use cascade_core::{Model, PaletteColor};

/// Fill and text colors for one machine, as `#RRGGBB`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Hue {
    pub fill: &'static str,
    pub text: &'static str,
}

/// The Okabe-Ito hex value and a readable text color on it.
pub(super) const fn hue(color: PaletteColor) -> Hue {
    let (fill, text) = match color {
        PaletteColor::Orange => ("#E69F00", "#000000"),
        PaletteColor::SkyBlue => ("#56B4E9", "#000000"),
        PaletteColor::Green => ("#009E73", "#ffffff"),
        PaletteColor::Yellow => ("#F0E442", "#000000"),
        PaletteColor::Blue => ("#0072B2", "#ffffff"),
        PaletteColor::Vermillion => ("#D55E00", "#ffffff"),
        PaletteColor::Purple => ("#CC79A7", "#000000"),
        PaletteColor::Black => ("#000000", "#ffffff"),
    };
    Hue { fill, text }
}

/// One hue per machine, indexed by machine id.
pub(super) fn machine_hues(model: &Model) -> Vec<Hue> {
    let mut used: Vec<PaletteColor> = model.machines().filter_map(|(_, m)| m.color).collect();
    let mut cycle = PaletteColor::ALL.iter().copied().cycle();
    model
        .machines()
        .map(|(_, machine)| {
            let color = machine.color.unwrap_or_else(|| {
                let fresh = PaletteColor::ALL.iter().copied().find(|c| !used.contains(c));
                let pick = fresh.or_else(|| cycle.next()).unwrap_or(PaletteColor::Blue);
                used.push(pick);
                pick
            });
            hue(color)
        })
        .collect()
}
