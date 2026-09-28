//! Choosing the monospace font for canvas labels.
//!
//! Scene builders size text with `MonoMeasure` (a fixed advance per
//! character). The painter forces that advance when shaping, so any
//! installed monospace family works; this only picks a pleasant one.

/// Preferred families, in order.
const PREFERRED: &[&str] = &[
    "JetBrains Mono",
    "DejaVu Sans Mono",
    "Liberation Mono",
    "Noto Sans Mono",
    "Ubuntu Mono",
    "Source Code Pro",
    "Fira Mono",
    "IBM Plex Mono",
    "SF Mono",
    "Menlo",
    "Monaco",
    "Cascadia Mono",
    "Consolas",
    "Courier New",
];

/// Used when nothing better is installed; resolved by the platform.
pub const FALLBACK: &str = "monospace";

/// The best monospace family among `available` font names.
pub fn pick_mono(available: &[String]) -> String {
    PREFERRED
        .iter()
        .find(|p| available.iter().any(|a| a == *p))
        .map(|p| (*p).to_owned())
        .or_else(|| available.iter().find(|a| a.contains("Mono")).cloned())
        .unwrap_or_else(|| FALLBACK.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(n: &[&str]) -> Vec<String> {
        n.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn prefers_the_list_order() {
        assert_eq!(pick_mono(&names(&["Arial", "Liberation Mono", "DejaVu Sans Mono"])), "DejaVu Sans Mono");
    }

    #[test]
    fn falls_back_to_any_mono_then_the_generic_name() {
        assert_eq!(pick_mono(&names(&["Arial", "Go Mono"])), "Go Mono");
        assert_eq!(pick_mono(&names(&["Arial"])), FALLBACK);
    }
}
