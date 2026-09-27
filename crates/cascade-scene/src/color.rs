//! Colors, themes and machine hue assignment.
//!
//! Hue means entity and nothing else. Machines take their hue from the
//! Okabe-Ito colorblind-safe palette; everything else is neutral. The only
//! exceptions the spec allows are analysis findings and diff mode (red
//! outlines, green outlines), which never fill a shape.

use serde::{Deserialize, Serialize};

use cascade_core::{MachineId, Model, PaletteColor};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const fn hex(v: u32) -> Self {
        Self::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
    }

    /// Linear blend toward `other`; `t = 0` is `self`, `t = 1` is `other`.
    pub fn mix(self, other: Rgba, t: f32) -> Rgba {
        let t = t.clamp(0.0, 1.0);
        let lerp = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
        Rgba { r: lerp(self.r, other.r), g: lerp(self.g, other.g), b: lerp(self.b, other.b), a: lerp(self.a, other.a) }
    }

    pub fn with_alpha(self, alpha: f32) -> Rgba {
        Rgba { a: (alpha.clamp(0.0, 1.0) * 255.0).round() as u8, ..self }
    }

    /// `#rrggbb`, ignoring alpha.
    pub fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    pub fn alpha_f32(self) -> f32 {
        f32::from(self.a) / 255.0
    }
}

/// The eight Okabe-Ito colors.
pub fn okabe_ito(color: PaletteColor) -> Rgba {
    match color {
        PaletteColor::Orange => Rgba::hex(0xE69F00),
        PaletteColor::SkyBlue => Rgba::hex(0x56B4E9),
        PaletteColor::Green => Rgba::hex(0x009E73),
        PaletteColor::Yellow => Rgba::hex(0xF0E442),
        PaletteColor::Blue => Rgba::hex(0x0072B2),
        PaletteColor::Vermillion => Rgba::hex(0xD55E00),
        PaletteColor::Purple => Rgba::hex(0xCC79A7),
        PaletteColor::Black => Rgba::hex(0x000000),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ThemeMode {
    Light,
    Dark,
}

/// Neutral colors and emphasis parameters for one theme.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Theme {
    pub mode: ThemeMode,
    pub background: Rgba,
    pub text: Rgba,
    pub text_muted: Rgba,
    /// Event tags and emit arrows.
    pub neutral: Rgba,
    /// Controller hexagon outlines.
    pub controller: Rgba,
    /// External source boxes.
    pub external: Rgba,
    /// Grid lines, lifelines, lane borders.
    pub rule: Rgba,
    /// Finding badges and cascade-cycle back edges.
    pub finding: Rgba,
    /// Diff mode: added elements' outline.
    pub added: Rgba,
    /// Diff mode: removed elements' ghost outline.
    pub removed: Rgba,
    /// Outline width of normal shapes.
    pub stroke_width: f32,
    /// Outline width of selected / focused shapes. Selection is shown by
    /// weight, never by color.
    pub selected_stroke_width: f32,
    /// Opacity of everything outside a cone or path query (spec: 15%).
    pub dim_opacity: f32,
    pub font_size: f32,
    pub small_font_size: f32,
}

impl Theme {
    pub fn light() -> Self {
        Self {
            mode: ThemeMode::Light,
            background: Rgba::hex(0xFFFFFF),
            text: Rgba::hex(0x1F2328),
            text_muted: Rgba::hex(0x656D76),
            neutral: Rgba::hex(0x8C959F),
            controller: Rgba::hex(0x32383F),
            external: Rgba::hex(0x57606A),
            rule: Rgba::hex(0xD0D7DE),
            finding: Rgba::hex(0xCF222E),
            added: Rgba::hex(0x1A7F37),
            removed: Rgba::hex(0xCF222E),
            stroke_width: 1.5,
            selected_stroke_width: 3.5,
            dim_opacity: 0.15,
            font_size: 13.0,
            small_font_size: 11.0,
        }
    }

    pub fn dark() -> Self {
        Self {
            mode: ThemeMode::Dark,
            background: Rgba::hex(0x0D1117),
            text: Rgba::hex(0xE6EDF3),
            text_muted: Rgba::hex(0x8D96A0),
            neutral: Rgba::hex(0x6E7681),
            controller: Rgba::hex(0xC9D1D9),
            external: Rgba::hex(0xA0A8B0),
            rule: Rgba::hex(0x30363D),
            finding: Rgba::hex(0xF85149),
            added: Rgba::hex(0x3FB950),
            removed: Rgba::hex(0xF85149),
            stroke_width: 1.5,
            selected_stroke_width: 3.5,
            dim_opacity: 0.15,
            font_size: 13.0,
            small_font_size: 11.0,
        }
    }
}

/// How one machine is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineStyle {
    /// Full-saturation hue: transition pills, transition arrows, fire arrows
    /// into this machine, legend chip.
    pub hue: Rgba,
    /// Pale tint for state fills and lane backgrounds.
    pub pale: Rgba,
    /// Text drawn on top of `hue`.
    pub on_hue: Rgba,
}

/// Assign every machine a style, indexed by `MachineId::index()`.
///
/// Declared colors win. Undeclared machines take unused palette hues in
/// Okabe-Ito order. Past eight machines, machines are grouped by `domain`
/// and share their domain's hue at different lightness.
///
/// Stub-quality: `feat/view-scenes` owns the full domain grouping.
pub fn machine_styles(model: &Model, theme: &Theme) -> Vec<MachineStyle> {
    let mut used: Vec<PaletteColor> = model.machines().filter_map(|(_, m)| m.color).collect();
    let mut next_free = PaletteColor::ALL.iter().copied().cycle();
    let mut assigned: Vec<PaletteColor> = Vec::with_capacity(model.machine_count());
    for (_, machine) in model.machines() {
        let color = machine.color.unwrap_or_else(|| {
            let fresh = PaletteColor::ALL.iter().copied().find(|c| !used.contains(c));
            let pick = fresh.or_else(|| next_free.next()).unwrap_or(PaletteColor::Blue);
            used.push(pick);
            pick
        });
        assigned.push(color);
    }
    assigned.into_iter().map(|color| style_for(color, 0, theme)).collect()
}

/// A palette hue shifted by `shade` lightness steps (0 = the hue itself).
pub fn style_for(color: PaletteColor, shade: u8, theme: &Theme) -> MachineStyle {
    let mut hue = okabe_ito(color);
    if color == PaletteColor::Black && theme.mode == ThemeMode::Dark {
        hue = Rgba::hex(0xD0D7DE);
    }
    if shade > 0 {
        let toward = match theme.mode {
            ThemeMode::Light => Rgba::hex(0x000000),
            ThemeMode::Dark => Rgba::hex(0xFFFFFF),
        };
        hue = hue.mix(toward, 0.22 * f32::from(shade.min(3)));
    }
    let pale = match theme.mode {
        ThemeMode::Light => hue.mix(theme.background, 0.85),
        ThemeMode::Dark => hue.mix(theme.background, 0.78),
    };
    let luminance = 0.2126 * f32::from(hue.r) + 0.7152 * f32::from(hue.g) + 0.0722 * f32::from(hue.b);
    let on_hue = if luminance > 140.0 { Rgba::hex(0x000000) } else { Rgba::hex(0xFFFFFF) };
    MachineStyle { hue, pale, on_hue }
}

/// Convenience: the style for one machine from a styles table.
pub fn style_of(styles: &[MachineStyle], machine: MachineId) -> MachineStyle {
    styles.get(machine.index()).copied().unwrap_or(MachineStyle {
        hue: Rgba::hex(0x808080),
        pale: Rgba::hex(0xEEEEEE),
        on_hue: Rgba::hex(0xFFFFFF),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip_and_mix() {
        let c = Rgba::hex(0x0072B2);
        assert_eq!(c.to_hex(), "#0072b2");
        assert_eq!(c.mix(Rgba::hex(0xFFFFFF), 0.0), c);
        assert_eq!(c.mix(Rgba::hex(0xFFFFFF), 1.0), Rgba::hex(0xFFFFFF));
    }

    #[test]
    fn declared_colors_win_and_others_avoid_them() {
        let model = cascade_core::load_str(
            "machines:\n  A: { color: blue, states: [x] }\n  B: { states: [x] }\n  C: { states: [x] }\n",
        )
        .expect("loads");
        let styles = machine_styles(&model, &Theme::light());
        assert_eq!(styles[0].hue, okabe_ito(PaletteColor::Blue));
        assert_ne!(styles[1].hue, styles[0].hue);
        assert_ne!(styles[2].hue, styles[1].hue);
        assert_ne!(styles[2].hue, styles[0].hue);
    }
}
