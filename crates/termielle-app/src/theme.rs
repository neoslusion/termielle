//! Theme presets for the island/notch glass material.
//!
//! A theme is a named `GlassConfig` plus layout hints. Built-ins live in
//! `themes/*.json` next to the binary and in `~/.island/themes/` (user shadows
//! install). JSON is flat: `{ "tint":[b,g,r,a], "blur_radius":0, ... }`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use termielle_core::{GlassConfig, IslandConfig};

/// A theme file on disk.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeFile {
    pub name: String,
    pub display_name: String,
    pub tint: Option<[u8; 4]>,
    pub blur_radius: Option<u32>,
    pub border_alpha: Option<u8>,
    pub highlight_alpha: Option<u8>,
    pub shadow_alpha: Option<u8>,
    pub corner_radius: Option<u32>,
    /// Tint override when the layout is an attached notch (hardware-black
    /// reads differently from floating glass, so one tint rarely suits
    /// both). Wins over `tint`, loses to explicit user glass.
    pub notch_tint: Option<[u8; 4]>,
    /// Tint override when the layout is a floating island.
    pub island_tint: Option<[u8; 4]>,
    pub description: String,
}

impl Default for ThemeFile {
    fn default() -> Self {
        Self {
            name: "liquid-dark".into(),
            display_name: "Liquid Dark".into(),
            tint: None,
            blur_radius: None,
            border_alpha: None,
            highlight_alpha: None,
            shadow_alpha: None,
            corner_radius: None,
            notch_tint: None,
            island_tint: None,
            description: String::new(),
        }
    }
}

/// Apply a theme to `config` in place. Unknown names keep current config.
/// The name `auto` maps to the system light/dark setting.
///
/// The glass material matches the Windows 11 taskbar: dark ≈ #202020 at 80%
/// opacity, light ≈ #F3F3F3 at 90%, and fully opaque when the user turns
/// transparency effects off.
/// In `auto` mode with accent-on-taskbar enabled, the tint additionally
/// leans toward the system accent color; explicit themes stay exact.
pub fn apply_theme(config: &mut IslandConfig, name: &str) {
    let resolved = if name == "auto" {
        crate::system::auto_theme_name().to_string()
    } else {
        name.to_string()
    };
    let transparency = crate::system::transparency_enabled();
    let (mut tint, blur, border_alpha, highlight_alpha, shadow_alpha): ([u8; 4], u32, u8, u8, u8) =
        match resolved.as_str() {
            "light" => (
                [243, 243, 243, if transparency { 230 } else { 255 }],
                24,
                40,
                90,
                if transparency { 40 } else { 0 },
            ),
            "transparent" => ([30, 30, 30, 90], 16, 20, 30, 30),
            "midnight" => ([12, 18, 32, 205], 28, 28, 50, 80),
            _ => (
                [32, 32, 32, if transparency { 205 } else { 255 }],
                24,
                38,
                70,
                if transparency { 60 } else { 0 },
            ),
        };
    // System-following dark mode matches the Windows 11 taskbar acrylic:
    // Windows 11 taskbar acrylic naturally infuses a subtle undertone (~16%)
    // of the active accent / colorization color (e.g. Windows blue),
    // giving that signature luminous slate-blue acrylic look.
    // When "Show accent color on Start and taskbar" is enabled, it uses
    // a richer 40% accent blend.
    if name == "auto" {
        config.glass.notch_black = false;
        if resolved != "light" {
            let base_tint = [30, 30, 30, if transparency { 180 } else { 255 }];
            let accent = crate::system::accent_color_bgra();
            let mix = if crate::system::taskbar_shows_accent() {
                ACCENT_MIX_VIVID
            } else if transparency {
                ACCENT_MIX_SUBTLE
            } else {
                0.0
            };
            tint = blend_toward_accent(base_tint, accent, mix);
        } else if crate::system::taskbar_shows_accent() {
            tint = blend_toward_accent(tint, crate::system::accent_color_bgra(), ACCENT_MIX_VIVID);
        }
    }
    config.glass.tint = tint;
    config.glass.blur_radius = blur;
    config.glass.border_alpha = border_alpha;
    config.glass.highlight_alpha = highlight_alpha;
    config.glass.shadow_alpha = shadow_alpha;
    config.glass.clamp();
    config.clamp();
}

/// Vivid accent mix when "Show accent color on Start and taskbar" is enabled (40%).
pub const ACCENT_MIX_VIVID: f32 = 0.40;
/// Subtle accent infusion for Windows 11 acrylic (~16%), giving the dark acrylic
/// its signature cool blue / accent luminous undertone.
pub const ACCENT_MIX_SUBTLE: f32 = 0.16;

/// Linear blend of a BGRA tint toward the accent color. Alpha is preserved:
/// translucency stays the theme's decision, only the hue follows.
pub fn blend_toward_accent(base: [u8; 4], accent: [u8; 4], mix: f32) -> [u8; 4] {
    let mut out = base;
    for ch in 0..3 {
        out[ch] = (f32::from(base[ch]) * (1.0 - mix) + f32::from(accent[ch]) * mix)
            .round()
            .clamp(0.0, 255.0) as u8;
    }
    out
}

/// Try to load a theme file from `roots` (user first, install second).
pub fn load_theme_file(name: &str, roots: &[PathBuf]) -> Option<ThemeFile> {
    for root in roots {
        let path = root.join(format!("{name}.json"));
        if let Ok(bytes) = std::fs::read(&path) {
            if let Ok(theme) = serde_json::from_slice::<ThemeFile>(&bytes) {
                return Some(theme);
            }
        }
    }
    None
}

/// Theme search roots: `~/.island/themes`, then exe dir `themes`.
pub fn theme_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        roots.push(PathBuf::from(home.clone()).join(".island").join("themes"));
        // Back-compat: also check .termielle
        roots.push(PathBuf::from(home).join(".termielle").join("themes"));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            roots.push(dir.join("themes"));
        }
    }
    // For cargo run
    roots.push(PathBuf::from("themes"));
    roots
}

/// Resolve and apply: builtin -> file override -> explicit user glass.
///
/// The theme preset supplies the material, but any `glass` field the user set
/// explicitly in `config.json` (i.e. differing from [`GlassConfig::default`])
/// wins — otherwise every non-`custom` theme would silently re-darken a
/// user-chosen tint and the pill would vanish on dark wallpapers.
pub fn resolve_theme(config: &mut IslandConfig) {
    let roots = theme_roots();
    resolve_theme_with_roots(config, &roots);
}

/// [`resolve_theme`] with explicit search roots, for hermetic tests.
fn resolve_theme_with_roots(config: &mut IslandConfig, roots: &[PathBuf]) {
    // Snapshot explicit user overrides vs the glass default.
    let default = GlassConfig::default();
    let user = config.glass.clone();
    let overrides = [
        user.tint != default.tint,
        user.blur_radius != default.blur_radius,
        user.border_alpha != default.border_alpha,
        user.highlight_alpha != default.highlight_alpha,
        user.shadow_alpha != default.shadow_alpha,
    ];

    // First apply builtin preset for theme name so missing fields get defaults
    apply_theme(config, &config.theme.clone());
    // Then overlay file if present
    if let Some(file) = load_theme_file(&config.theme.clone(), roots) {
        if let Some(tint) = file.tint {
            config.glass.tint = tint;
        }
        if let Some(v) = file.blur_radius {
            config.glass.blur_radius = v;
        }
        if let Some(v) = file.border_alpha {
            config.glass.border_alpha = v;
        }
        if let Some(v) = file.highlight_alpha {
            config.glass.highlight_alpha = v;
        }
        if let Some(v) = file.shadow_alpha {
            config.glass.shadow_alpha = v;
        }
        if let Some(v) = file.corner_radius {
            config.corner_radius = v;
        }
        // Per-layout material: attached notches and floating islands catch
        // light differently. Applies before explicit user glass below.
        let layout_tint = if config.is_attached() {
            file.notch_tint
        } else {
            file.island_tint
        };
        if let Some(tint) = layout_tint {
            config.glass.tint = tint;
        }
    }

    // Explicit user overrides win over both presets.
    if overrides[0] {
        config.glass.tint = user.tint;
    }
    if overrides[1] {
        config.glass.blur_radius = user.blur_radius;
    }
    if overrides[2] {
        config.glass.border_alpha = user.border_alpha;
    }
    if overrides[3] {
        config.glass.highlight_alpha = user.highlight_alpha;
    }
    if overrides[4] {
        config.glass.shadow_alpha = user.shadow_alpha;
    }

    config.glass.clamp();
    config.clamp();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_apply_changes_tint() {
        let mut c = IslandConfig {
            theme: "light".into(),
            ..Default::default()
        };
        apply_theme(&mut c, "light");
        // Windows 11 taskbar light material: #F3F3F3.
        assert_eq!(c.glass.tint, [243, 243, 243, 230]);
    }

    #[test]
    fn auto_theme_resolves_to_the_taskbar_material() {
        let mut c = IslandConfig {
            theme: "auto".into(),
            ..Default::default()
        };
        apply_theme(&mut c, "auto");
        let light = crate::system::system_uses_light_theme()
            .or_else(crate::system::apps_use_light_theme)
            == Some(true);
        // Must mirror apply_theme: taskbar-matched dark veil, then the
        // accent lean when the taskbar shows it.
        let expected: [u8; 4] = if light {
            let base = [243, 243, 243, 230];
            if crate::system::taskbar_shows_accent() {
                blend_toward_accent(base, crate::system::accent_color_bgra(), ACCENT_MIX_VIVID)
            } else {
                base
            }
        } else {
            let base = [30, 30, 30, if crate::system::transparency_enabled() { 180 } else { 255 }];
            let mix = if crate::system::taskbar_shows_accent() {
                ACCENT_MIX_VIVID
            } else if crate::system::transparency_enabled() {
                ACCENT_MIX_SUBTLE
            } else {
                0.0
            };
            blend_toward_accent(base, crate::system::accent_color_bgra(), mix)
        };
        assert_eq!(c.glass.tint, expected);
        assert!(!c.glass.notch_black);
    }

    #[test]
    fn blend_leans_toward_accent_and_keeps_alpha() {
        // 40% of the way from near-black to Windows blue, alpha untouched.
        assert_eq!(
            blend_toward_accent([32, 32, 32, 205], [215, 120, 0, 255], 0.40),
            [105, 67, 19, 205]
        );
        // 16% subtle acrylic blend
        assert_eq!(
            blend_toward_accent([30, 30, 30, 180], [212, 120, 0, 255], 0.16),
            [59, 44, 25, 180]
        );
    }

    #[test]
    fn explicit_themes_ignore_the_taskbar_accent() {
        // Midnight stays navy even on an accent-painted taskbar: only `auto`
        // follows the system.
        let mut c = IslandConfig {
            theme: "midnight".into(),
            ..Default::default()
        };
        apply_theme(&mut c, "midnight");
        assert_eq!(c.glass.tint[..3], [12, 18, 32]);
    }

    #[test]
    fn unknown_theme_keeps_current() {
        let mut c = IslandConfig::default();
        // Unknown names resolve to the dark taskbar default.
        apply_theme(&mut c, "nope");
        assert_eq!(c.glass.tint, [32, 32, 32, 205]);
    }

    #[test]
    fn file_tint_applies_per_layout_before_user_glass() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("split.json"),
            r#"{"name":"split","tint":[10,10,10,200],"notch_tint":[0,0,0,255],"island_tint":[80,90,110,180]}"#,
        )
        .unwrap();
        let roots = vec![dir.path().to_path_buf()];
        // Notch takes the notch tint.
        let mut notch = IslandConfig {
            layout: termielle_core::IslandLayout::Notch,
            theme: "split".into(),
            ..Default::default()
        };
        resolve_theme_with_roots(&mut notch, &roots);
        assert_eq!(notch.glass.tint, [0, 0, 0, 255]);
        // Island takes the island tint.
        let mut island = IslandConfig {
            layout: termielle_core::IslandLayout::Island,
            theme: "split".into(),
            ..Default::default()
        };
        resolve_theme_with_roots(&mut island, &roots);
        assert_eq!(island.glass.tint, [80, 90, 110, 180]);
        // Explicit user glass still wins over both.
        let mut custom = IslandConfig {
            layout: termielle_core::IslandLayout::Island,
            theme: "split".into(),
            glass: termielle_core::GlassConfig {
                tint: [1, 2, 3, 4],
                ..Default::default()
            },
            ..Default::default()
        };
        resolve_theme_with_roots(&mut custom, &roots);
        assert_eq!(custom.glass.tint, [1, 2, 3, 4]);
    }
}
