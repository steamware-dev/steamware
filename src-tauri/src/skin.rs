//! The app's accent, carried into Steam.
//!
//! The in-Steam plugin used to be painted from a fixed pink→purple pair. That is the brand, but it meant a
//! user who set the app to lime, teal or rose still met a magenta panel inside Steam — two programs wearing
//! different colours. This module turns whatever accent is in Settings into the palette the injected style
//! scripts use: the gradient starts on the user's exact accent and runs into a darker, hue-shifted stop, so
//! the panel always looks deliberate and always belongs to that theme.
//!
//! Nothing here touches the network, the bundle or the plugin's files: the palette is pushed into the page
//! as a small object (`window.__SteamwareSkin`) before the skin scripts run, and a signature in it
//! (`window.__SteamwareSkinId`) is what tells the injector a page is wearing an older accent and needs
//! repainting. Changing the accent in Settings therefore repaints open store pages within a second, with no
//! reload and no reinstall of the plugin.

use serde::Serialize;

/// Where the gradient's second stop sits relative to the accent, in degrees.
///
/// This is the one number that defines the look. Pink `#d63b96` is ~327°, and the original purple stop was
/// `#6b2bd9` at ~262° — so for the brand accent this rotation reproduces the pair the plugin shipped with
/// almost exactly, while any other accent gets a coherent partner rather than a random one.
const HUE_SHIFT: f64 = -58.0;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Palette {
    /// Signature of this palette. The injector compares it against the page's copy to decide on repaint.
    pub id: String,
    pub accent: String,
    pub surface: String,
    pub raised: String,
    pub line: String,
    pub text: String,
    pub text_dim: String,
    pub hover: String,
    pub gradient: String,
    pub gradient_hot: String,
    pub glow: String,
    pub glow_soft: String,
    /// Lightened accent, for icons and other small marks that need to read on the panel.
    pub icon: String,
}

fn parse_hex(value: &str) -> Option<(u8, u8, u8)> {
    let text = value.trim().trim_start_matches('#');
    if text.len() != 6 || !text.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let r = u8::from_str_radix(&text[0..2], 16).ok()?;
    let g = u8::from_str_radix(&text[2..4], 16).ok()?;
    let b = u8::from_str_radix(&text[4..6], 16).ok()?;
    Some((r, g, b))
}

/// `(hue 0..360, saturation 0..1, lightness 0..1)`
fn to_hsl((r, g, b): (u8, u8, u8)) -> (f64, f64, f64) {
    let r = r as f64 / 255.0;
    let g = g as f64 / 255.0;
    let b = b as f64 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let delta = max - min;
    if delta.abs() < f64::EPSILON {
        return (0.0, 0.0, l);
    }
    let s = if l > 0.5 { delta / (2.0 - max - min) } else { delta / (max + min) };
    let h = if (max - r).abs() < f64::EPSILON {
        ((g - b) / delta + if g < b { 6.0 } else { 0.0 }) / 6.0
    } else if (max - g).abs() < f64::EPSILON {
        ((b - r) / delta + 2.0) / 6.0
    } else {
        ((r - g) / delta + 4.0) / 6.0
    };
    (h * 360.0, s, l)
}

fn channel(p: f64, q: f64, mut t: f64) -> f64 {
    if t < 0.0 {
        t += 1.0;
    }
    if t > 1.0 {
        t -= 1.0;
    }
    if t < 1.0 / 6.0 {
        return p + (q - p) * 6.0 * t;
    }
    if t < 0.5 {
        return q;
    }
    if t < 2.0 / 3.0 {
        return p + (q - p) * (2.0 / 3.0 - t) * 6.0;
    }
    p
}

fn from_hsl(hue: f64, saturation: f64, lightness: f64) -> (u8, u8, u8) {
    let h = ((hue % 360.0) + 360.0) % 360.0 / 360.0;
    let s = saturation.clamp(0.0, 1.0);
    let l = lightness.clamp(0.0, 1.0);
    if s.abs() < f64::EPSILON {
        let value = (l * 255.0).round() as u8;
        return (value, value, value);
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let to_byte = |value: f64| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    (
        to_byte(channel(p, q, h + 1.0 / 3.0)),
        to_byte(channel(p, q, h)),
        to_byte(channel(p, q, h - 1.0 / 3.0)),
    )
}

fn hex((r, g, b): (u8, u8, u8)) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn rgba((r, g, b): (u8, u8, u8), alpha: f64) -> String {
    format!("rgba({r}, {g}, {b}, {alpha})")
}

/// Build the palette for `accent` (any `#rrggbb`) in `mode` (`dark` or `light`).
///
/// An unreadable accent falls back to the app's own default violet rather than to the plugin's magenta: the
/// panel should never be the one thing on screen that ignored the user's choice.
pub fn palette(accent: &str, mode: &str) -> Palette {
    let rgb = parse_hex(accent).unwrap_or((0xa7, 0x8b, 0xfa));
    let (hue, saturation, lightness) = to_hsl(rgb);

    // Keep the accent recognisable: only clamp when it would be too dark or too washed out to carry a
    // gradient on black (or on a light card, for that matter).
    let start_hsl = (hue, saturation.max(0.40), lightness.clamp(0.40, 0.78));
    let start = from_hsl(start_hsl.0, start_hsl.1, start_hsl.2);

    let end_s = (start_hsl.1 * 1.05).clamp(0.0, 0.80);
    let end_l = (start_hsl.2 * 0.96).clamp(0.34, 0.52);
    let end = from_hsl(start_hsl.0 + HUE_SHIFT, end_s, end_l);

    let hot = from_hsl(start_hsl.0 + 8.0, start_hsl.1, (start_hsl.2 * 1.12).clamp(0.0, 0.80));
    let hot_end = from_hsl(start_hsl.0 + HUE_SHIFT, end_s, (end_l + 0.09).clamp(0.0, 0.62));
    let icon = from_hsl(start_hsl.0, start_hsl.1.min(0.62), (start_hsl.2 + 0.24).clamp(0.0, 0.86));

    let light = mode == "light";
    let (surface, raised, line, text, text_dim, hover) = if light {
        ("#f7f8fc", "#eceef6", "rgba(16, 19, 28, 0.12)", "#151821", "#5a6172", "rgba(16, 19, 28, 0.06)")
    } else {
        ("#141519", "#1b1d23", "rgba(255, 255, 255, 0.09)", "#f4f5f8", "#aeb3c2", "rgba(255, 255, 255, 0.055)")
    };

    Palette {
        id: format!("{mode}:{}:{}:{surface}", hex(start), hex(end)),
        accent: hex(rgb),
        surface: surface.to_string(),
        raised: raised.to_string(),
        line: line.to_string(),
        text: text.to_string(),
        text_dim: text_dim.to_string(),
        hover: hover.to_string(),
        gradient: format!("linear-gradient(135deg, {} 0%, {} 100%)", hex(start), hex(end)),
        gradient_hot: format!("linear-gradient(135deg, {} 0%, {} 100%)", hex(hot), hex(hot_end)),
        glow: rgba(end, 0.42),
        glow_soft: rgba(start, 0.50),
        icon: hex(icon),
    }
}

/// The script that hands the palette to the overlay scripts. Evaluated before them in the same call, so a
/// skin script never runs against a previous page's colours.
pub fn script(palette: &Palette) -> String {
    let body = serde_json::to_string(palette).unwrap_or_else(|_| "{}".to_string());
    format!(
        "window.__SteamwareSkinId={};window.__SteamwareSkin={};",
        serde_json::to_string(&palette.id).unwrap_or_else(|_| "\"\"".to_string()),
        body
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brand_pink_reproduces_the_shipped_pair() {
        let palette = palette("#d63b96", "dark");
        // The original plugin chrome was #d63b96 → #6b2bd9.
        assert!(palette.gradient.starts_with("linear-gradient(135deg, #d63b96 0%, #"), "{}", palette.gradient);
        assert!(palette.gradient.ends_with(" 100%)"));
        let end = palette.gradient.split("#").nth(2).unwrap().trim_end_matches(" 100%)");
        let (_, _, lightness) = to_hsl(parse_hex(end).unwrap());
        assert!((0.42..=0.55).contains(&lightness), "end stop must stay a deep purple: {end}");
    }

    #[test]
    fn every_accent_yields_a_two_stop_gradient() {
        for accent in ["#a78bfa", "#b3d775", "#69d3c5", "#dd8894", "#000000", "#ffffff", "nonsense"] {
            let palette = palette(accent, "dark");
            assert!(palette.gradient.contains("linear-gradient(135deg"), "{accent}: {}", palette.gradient);
            assert_eq!(palette.gradient.matches('#').count(), 2, "{accent}: {}", palette.gradient);
            assert!(palette.icon.starts_with('#'));
            assert!(palette.glow.starts_with("rgba("));
        }
    }

    #[test]
    fn a_mid_accent_is_used_verbatim() {
        // No clamps should touch these: they are the colours the gallery offers.
        for accent in ["#a78bfa", "#d63b96", "#69d3c5"] {
            assert!(palette(accent, "dark").gradient.contains(accent), "{accent}");
        }
    }

    #[test]
    fn signature_changes_with_accent_and_mode() {
        assert_eq!(palette("#a78bfa", "dark"), palette("#a78bfa", "dark"));
        assert_ne!(palette("#a78bfa", "dark").id, palette("#b3d775", "dark").id);
        assert_ne!(palette("#a78bfa", "dark").id, palette("#a78bfa", "light").id);
        assert_eq!(palette("#A78BFA", "dark").id, palette("#a78bfa", "dark").id);
    }

    #[test]
    fn light_mode_flips_the_surface_but_keeps_the_accent() {
        let light = palette("#b3d775", "light");
        assert_eq!(light.surface, "#f7f8fc");
        assert_eq!(light.text, "#151821");
        assert!(light.gradient.contains("#b3d775"));
        assert_eq!(palette("#b3d775", "dark").gradient, light.gradient);
    }

    #[test]
    fn script_sets_a_parseable_object() {
        let palette = palette("#d63b96", "dark");
        let script = script(&palette);
        assert!(script.starts_with("window.__SteamwareSkinId=\""));
        assert!(script.contains("window.__SteamwareSkin={"));
        assert!(script.contains("\"gradient\":\"linear-gradient"));
    }
}
