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
/// Declared colors win. Undeclared machines take the least-used palette hue
/// (unused hues first, in Okabe-Ito order). Up to eight machines each form
/// their own group; past eight, machines are grouped by `domain` (a machine
/// without a domain is a group of its own) and every undeclared machine of a
/// group shares the group's hue. Machines that end up with the same hue get
/// successive lightness steps in definition order, so no two machines of one
/// hue look identical until a hue carries more than four machines.
pub fn machine_styles(model: &Model, theme: &Theme) -> Vec<MachineStyle> {
    machine_colors(model).into_iter().map(|(color, shade)| style_for(color, shade, theme)).collect()
}

/// The palette color and lightness step of every machine, indexed by
/// `MachineId::index()`. See [`machine_styles`].
pub fn machine_colors(model: &Model) -> Vec<(PaletteColor, u8)> {
    let group_by_domain = model.machine_count() > PaletteColor::ALL.len();

    // Groups in order of first appearance: a domain, or one machine alone.
    let mut group_of: Vec<usize> = Vec::with_capacity(model.machine_count());
    let mut group_keys: Vec<Option<&str>> = Vec::new();
    for (_, machine) in model.machines() {
        let key = if group_by_domain { machine.domain.as_deref() } else { None };
        let existing = key.and_then(|k| group_keys.iter().position(|g| *g == Some(k)));
        let group = existing.unwrap_or_else(|| {
            group_keys.push(key);
            group_keys.len() - 1
        });
        group_of.push(group);
    }

    // A group's hue is the first declared color among its machines; groups
    // without one take the least-used hue afterwards.
    let mut group_color: Vec<Option<PaletteColor>> = vec![None; group_keys.len()];
    let mut usage = [0u32; 8];
    let slot = |c: PaletteColor| PaletteColor::ALL.iter().position(|&p| p == c).unwrap_or(0);
    for ((_, machine), &group) in model.machines().zip(&group_of) {
        if let Some(color) = machine.color {
            if group_color[group].is_none() {
                group_color[group] = Some(color);
            }
            usage[slot(color)] += 1;
        }
    }
    for color in &mut group_color {
        if color.is_none() {
            let (best, _) = PaletteColor::ALL
                .iter()
                .enumerate()
                .min_by_key(|(i, _)| (usage[*i], *i))
                .unwrap_or((0, &PaletteColor::Blue));
            usage[best] += 1;
            *color = Some(PaletteColor::ALL[best]);
        }
    }

    let mut shades_used = [0u8; 8];
    model
        .machines()
        .zip(&group_of)
        .map(|((_, machine), &group)| {
            let color = machine.color.or(group_color[group]).unwrap_or(PaletteColor::Blue);
            let shade = &mut shades_used[slot(color)];
            let this = *shade;
            *shade = shade.saturating_add(1);
            (color, this)
        })
        .collect()
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

    fn machines_yaml(entries: &[(&str, Option<&str>, Option<&str>)]) -> String {
        let mut yaml = String::from("machines:\n");
        for (name, color, domain) in entries {
            let mut attrs = vec!["states: [x]".to_owned()];
            if let Some(c) = color {
                attrs.push(format!("color: {c}"));
            }
            if let Some(d) = domain {
                attrs.push(format!("domain: {d}"));
            }
            yaml.push_str(&format!("  {name}: {{ {} }}\n", attrs.join(", ")));
        }
        yaml
    }

    #[test]
    fn up_to_eight_machines_get_distinct_hues_and_ignore_domains() {
        let names = ["A", "B", "C", "D", "E", "F", "G", "H"];
        let entries: Vec<_> = names.iter().map(|n| (*n, None, Some("same"))).collect();
        let model = cascade_core::load_str(&machines_yaml(&entries)).expect("loads");
        let colors = machine_colors(&model);
        let mut hues: Vec<PaletteColor> = colors.iter().map(|(c, _)| *c).collect();
        assert!(colors.iter().all(|(_, shade)| *shade == 0));
        hues.sort();
        hues.dedup();
        assert_eq!(hues.len(), 8, "{colors:?}");
        // Automatic assignment follows Okabe-Ito order.
        assert_eq!(colors[0].0, PaletteColor::ALL[0]);
        assert_eq!(colors[1].0, PaletteColor::ALL[1]);
    }

    #[test]
    fn duplicate_declared_colors_differ_in_lightness() {
        let model = cascade_core::load_str(&machines_yaml(&[("A", Some("blue"), None), ("B", Some("blue"), None)]))
            .expect("loads");
        let theme = Theme::light();
        let styles = machine_styles(&model, &theme);
        assert_eq!(styles[0], style_for(PaletteColor::Blue, 0, &theme));
        assert_eq!(styles[1], style_for(PaletteColor::Blue, 1, &theme));
        assert_ne!(styles[0].hue, styles[1].hue);
    }

    #[test]
    fn past_eight_machines_share_domain_hues_at_different_lightness() {
        let mut entries: Vec<(String, Option<&str>, Option<&str>)> = Vec::new();
        for i in 0..4 {
            entries.push((format!("Sales{i}"), None, Some("sales")));
        }
        for i in 0..4 {
            entries.push((format!("Ship{i}"), None, Some("shipping")));
        }
        entries.push(("Billing".to_owned(), Some("purple"), Some("money")));
        entries.push(("Ledger".to_owned(), None, Some("money")));
        let refs: Vec<(&str, Option<&str>, Option<&str>)> =
            entries.iter().map(|(n, c, d)| (n.as_str(), *c, *d)).collect();
        let model = cascade_core::load_str(&machines_yaml(&refs)).expect("loads");
        let colors = machine_colors(&model);
        let theme = Theme::light();
        let styles = machine_styles(&model, &theme);

        let sales = colors[0].0;
        assert!(colors[..4].iter().all(|(c, _)| *c == sales));
        assert_eq!(colors[..4].iter().map(|(_, s)| *s).collect::<Vec<_>>(), [0, 1, 2, 3]);
        let shipping = colors[4].0;
        assert_ne!(sales, shipping);
        assert!(colors[4..8].iter().all(|(c, _)| *c == shipping));
        // The declared color names the whole domain's hue.
        assert_eq!(colors[8], (PaletteColor::Purple, 0));
        assert_eq!(colors[9], (PaletteColor::Purple, 1));
        assert!(![sales, shipping].contains(&PaletteColor::Purple));
        // Same hue, different lightness: every style is distinct.
        for i in 0..styles.len() {
            for j in (i + 1)..styles.len() {
                assert_ne!(styles[i].hue, styles[j].hue, "{i} vs {j}");
            }
        }
    }

    #[test]
    fn past_eight_machines_without_domains_reuse_hues_with_shades() {
        let names: Vec<String> = (0..10).map(|i| format!("M{i}")).collect();
        let entries: Vec<_> = names.iter().map(|n| (n.as_str(), None, None)).collect();
        let model = cascade_core::load_str(&machines_yaml(&entries)).expect("loads");
        let colors = machine_colors(&model);
        assert_eq!(colors[8], (colors[0].0, 1));
        assert_eq!(colors[9], (colors[1].0, 1));
    }
}
