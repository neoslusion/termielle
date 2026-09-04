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
            description: String::new(),
        }
    }
}

/// Built-in presets that ship with the binary, so the overlay works with no
/// theme files on disk.
pub fn builtin_themes() -> Vec<ThemeFile> {
    // `auto` resolves to the system light/dark setting before lookup.
    let names: Vec<&str> = vec!["liquid-dark", "midnight", "light", "transparent"];
    let mut out = Vec::new();
    for name in names {
        out.push(builtin(name));
    }
    out
}

fn builtin(name: &str) -> ThemeFile {
    match name {
        "light" => ThemeFile {
            name: "light".into(),
            display_name: "Light Glass".into(),
            tint: Some([245, 245, 245, 160]),
            blur_radius: Some(12),
            border_alpha: Some(40),
            highlight_alpha: Some(90),
            shadow_alpha: Some(40),
            corner_radius: Some(18),
            description: "Light frosted glass for dark wallpapers".into(),
        },
        "midnight" => ThemeFile {
            name: "midnight".into(),
            display_name: "Midnight".into(),
            tint: Some([12, 18, 32, 200]),
            blur_radius: Some(12),
            border_alpha: Some(28),
            highlight_alpha: Some(50),
            shadow_alpha: Some(80),
            corner_radius: Some(18),
            description: "Deep navy, stronger shadow".into(),
        },
        "transparent" => ThemeFile {
            name: "transparent".into(),
            display_name: "Transparent".into(),
            tint: Some([30, 30, 30, 90]),
            blur_radius: Some(12),
            border_alpha: Some(20),
            highlight_alpha: Some(30),
            shadow_alpha: Some(30),
            corner_radius: Some(18),
            description: "Barely-there glass".into(),
        },
        _ => ThemeFile {
            name: "liquid-dark".into(),
            display_name: "Liquid Dark".into(),
            tint: Some([26, 26, 26, 180]),
            blur_radius: Some(12),
            border_alpha: Some(38),
            highlight_alpha: Some(70),
            shadow_alpha: Some(60),
            corner_radius: Some(18),
            description: "Near-black liquid glass, frosted border + top highlight".into(),
        },
    }
}

/// Apply a theme to `config` in place. Unknown names keep current config.
/// The name `auto` maps to the system light/dark setting.
///
/// The glass material matches the Windows 11 taskbar: dark ≈ #202020 at 80%
/// opacity, light ≈ #F3F3F3 at 90%, and fully opaque when the user turns
/// transparency effects off.
pub fn apply_theme(config: &mut IslandConfig, name: &str) {
    let resolved = if name == "auto" {
        crate::system::auto_theme_name().to_string()
    } else {
        name.to_string()
    };
    let transparency = crate::system::transparency_enabled();
    let (tint, border_alpha, highlight_alpha, shadow_alpha): ([u8; 4], u8, u8, u8) =
        match resolved.as_str() {
            "light" => (
                [243, 243, 243, if transparency { 230 } else { 255 }],
                40,
                90,
                if transparency { 40 } else { 0 },
            ),
            "transparent" => ([30, 30, 30, 90], 20, 30, 30),
            "midnight" => ([12, 18, 32, 205], 28, 50, 80),
            _ => (
                [32, 32, 32, if transparency { 205 } else { 255 }],
                38,
                70,
                if transparency { 60 } else { 0 },
            ),
        };
    config.glass.tint = tint;
    config.glass.border_alpha = border_alpha;
    config.glass.highlight_alpha = highlight_alpha;
    config.glass.shadow_alpha = shadow_alpha;
    config.glass.clamp();
    config.clamp();
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
    let roots = theme_roots();
    if let Some(file) = load_theme_file(&config.theme.clone(), &roots) {
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
    use crate::system::apps_use_light_theme;

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
        let light = crate::system::apps_use_light_theme() == Some(true);
        let expected: [u8; 4] = if light {
            [243, 243, 243, 230]
        } else {
            [32, 32, 32, 205]
        };
        assert_eq!(c.glass.tint, expected);
    }

    #[test]
    fn unknown_theme_keeps_current() {
        let mut c = IslandConfig::default();
        // Unknown names resolve to the dark taskbar default.
        apply_theme(&mut c, "nope");
        assert_eq!(c.glass.tint, [32, 32, 32, 205]);
    }
}
