use serde::{Deserialize, Serialize};

// ---- Layout ---------------------------------------------------------------

/// Which overlay shape to use. `Classic` is the original corner pet that
/// free-drags and clamps to the work area. `Notch` is a top-edge-attached
/// pill (flat top, rounded bottom) like a MacBook notch. `Island` is a
/// floating pill below the top edge, like Dynamic Island. Switchable is
/// just a config toggle between the two — the enum already covers it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum IslandLayout {
    #[default]
    Classic,
    Notch,
    Island,
    Bar,
}

/// Bar screen position (Top or Bottom).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BarPosition {
    #[default]
    Top,
    Bottom,
}

/// Waybar-style status bar configuration.
///
/// Layout contract (all sizes logical px): `edge_to_edge: true` (default)
/// spans the full monitor width; `false` insets the bar by `margin` on the
/// free sides for a floating look (the window stays full-width, the margin
/// stays transparent and click-through). `corner_radius` rounds the bar
/// blob (0 keeps the cheap flat fill). `modules_*` filter which modules
/// render in each zone — order within a zone is fixed, unknown names are
/// ignored, and an emptied zone collapses.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BarConfig {
    pub position: BarPosition,
    pub height: u32,
    pub edge_to_edge: bool,
    pub margin: u32,
    pub corner_radius: u32,
    pub reserve_space: bool,
    pub replace_taskbar: bool,
    /// Follow the cursor across monitors (re-anchor, re-size, re-reserve on
    /// every crossing). `false` pins the bar to the primary monitor.
    /// Defaults true so existing setups keep their behavior.
    pub follow_active_monitor: bool,
    pub modules_left: Vec<String>,
    pub modules_center: Vec<String>,
    pub modules_right: Vec<String>,
}

impl Default for BarConfig {
    fn default() -> Self {
        Self {
            position: BarPosition::Top,
            height: 36,
            edge_to_edge: true,
            margin: 0,
            corner_radius: 0,
            reserve_space: true,
            replace_taskbar: false,
            follow_active_monitor: true,
            modules_left: vec!["workspaces".to_string(), "window".to_string()],
            modules_center: vec!["termielle".to_string()],
            modules_right: vec![
                "cpu".to_string(),
                "memory".to_string(),
                "volume".to_string(),
                "battery".to_string(),
                // Between the status icons and the clock, which is where macOS
                // puts Control Center: a menu-bar item that is always live,
                // not something you have to open a card to reach.
                "control_center".to_string(),
                "clock".to_string(),
            ],
        }
    }
}

// ---- Glass ----------------------------------------------------------------

/// Custom layered glass material — baked into the DIB, not DWM acrylic.
///
/// All values are 0-255 bytes so the renderer can copy them directly into
/// PBGRA. The defaults give the liquid-dark theme: near-black tint at 0.70
/// with a soft border, top highlight, and drop shadow.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GlassConfig {
    /// Tint color as BGRA bytes. Default `[30,22,18,190]` is a cool neutral
    /// near-black at roughly 75% opacity.
    pub tint: [u8; 4],
    /// Backdrop blur radius in pixels for the frosted-glass effect: the
    /// live wallpaper behind the pill is captured (layered windows excluded)
    /// and box-blurred underneath the tint. 0 disables it (flat tint).
    pub blur_radius: u32,
    /// Border stroke alpha (0-255). 38 ~ 0.15.
    pub border_alpha: u8,
    /// Top-edge inner highlight alpha (0-255). 70 ~ 0.28.
    pub highlight_alpha: u8,
    /// Drop shadow alpha (0-255). 60 ~ 0.24, drawn 2px offset.
    pub shadow_alpha: u8,
    /// When the layout is a bezel-attached notch, render the body as opaque
    /// true black so it reads as display hardware, like the real MacBook
    /// notch and the iOS island (which must fuse with the camera housing).
    pub notch_black: bool,
}

impl Default for GlassConfig {
    fn default() -> Self {
        Self {
            tint: [30, 22, 18, 190],
            blur_radius: 12,
            border_alpha: 24,
            highlight_alpha: 42,
            shadow_alpha: 48,
            notch_black: true,
        }
    }
}

impl GlassConfig {
    pub fn clamp(&mut self) {
        // tint bytes already 0-255, no clamp needed.
        self.blur_radius = self.blur_radius.min(30);
        // border/highlight/shadow already u8.
    }
}

// ---- IslandConfig ---------------------------------------------------------

const MIN_COLLAPSED_W: u32 = 80;
const MAX_COLLAPSED_W: u32 = 1200;
const DEFAULT_COLLAPSED_W: u32 = 140;

const MIN_EXPANDED_W: u32 = 120;
const MAX_EXPANDED_W: u32 = 1600;
const DEFAULT_EXPANDED_W: u32 = 320;

const MIN_HEIGHT: u32 = 28;
const MAX_HEIGHT: u32 = 200;
const DEFAULT_HEIGHT: u32 = 36;

const MIN_RADIUS: u32 = 0;
const MAX_RADIUS: u32 = 120;
const DEFAULT_RADIUS: u32 = 18;

const MIN_Y_OFFSET: i32 = 0;
const MAX_Y_OFFSET: i32 = 500;
const DEFAULT_Y_OFFSET: i32 = 8;

const MIN_ANIM_MS: u32 = 100;
const MAX_ANIM_MS: u32 = 800;
const DEFAULT_ANIM_MS: u32 = 350;
const DEFAULT_COLLAPSE_MS: u32 = 300;
const DEFAULT_ALERT_MS: u32 = 220;

const MIN_MINIMAL_W: u32 = 48;
const MAX_MINIMAL_W: u32 = 800;
const DEFAULT_MINIMAL_W: u32 = 72;

const MIN_SPRING_BOUNCE: f32 = 0.0;
const MAX_SPRING_BOUNCE: f32 = 0.5;
const DEFAULT_SPRING_BOUNCE: f32 = 0.18;

/// Top-center notch / island geometry + material.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct IslandConfig {
    pub layout: IslandLayout,
    /// Compact width in logical pixels — the resting pill while an agent
    /// session or media is live (iOS "compact" presentation).
    pub collapsed_width: u32,
    /// Expanded width in logical pixels — the dashboard while hovered or
    /// pinned (iOS "expanded" presentation).
    pub expanded_width: u32,
    /// Minimal width in logical pixels — the small resting dot when nothing
    /// is live (iOS "minimal" presentation). Clamped to >= height + 8.
    pub minimal_width: u32,
    /// Height in logical pixels (both states).
    pub height: u32,
    /// Corner radius in logical pixels.
    pub corner_radius: u32,
    /// Y offset from top edge when `layout == Island`. Ignored for Notch.
    pub y_offset: i32,
    /// Morph perceptual duration in ms. Converted to spring
    /// stiffness/damping with `spring_bounce` (Apple's spring model), so
    /// morphs overshoot slightly and settle naturally like iOS.
    pub animation_ms: u32,
    /// Collapse duration in ms: retiring surfaces settle critically damped
    /// (no overshoot), slightly quicker than they opened.
    pub collapse_ms: u32,
    /// Alert-banner drop-in duration in ms: notifications arrive fast with
    /// the configured bounce.
    pub alert_ms: u32,
    /// Spring bounce 0.0-0.5: 0 = critically damped (no overshoot), higher
    /// = springier. Applies to expanding morphs and alert drop-ins.
    pub spring_bounce: f32,
    /// Theme preset name — e.g. "liquid-dark", "light", "midnight".
    /// When present the loader looks up `themes/<name>.json` but `glass`
    /// still overrides per-field.
    pub theme: String,
    /// Glass material baked into the frame.
    pub glass: GlassConfig,
    /// Whether the island should honor `scale` from AppConfig.
    pub scale_with_dpi: bool,
    /// Show running-task app icons in the expanded pill.
    pub show_tasks: bool,
    /// Maximum task icons shown when expanded (0-6).
    pub max_thumbnails: u32,
    /// Animate the termielle face inside the notch (advance its GIF frames).
    /// When false the face is a still of the state's first frame.
    pub face_animated: bool,
    /// Visual widgets rendered inside the pill (no text anywhere).
    /// Known names: `face` (animated character), `tasks` (running-task app
    /// icons), `agents` (one dot per live agent session), `music` (indicator
    /// strip while media plays), `ring` (progress ring around the face).
    /// Unknown names are dropped on load.
    pub widgets: Vec<String>,
    /// Which 0-100% metric the `ring` widget draws (`battery`, `cpu`,
    /// `mem`). Anything else falls back to `battery` on load.
    pub ring_metric: String,
    /// When true, hovering the collapsed island expands it (idle only);
    /// leaving collapses it again unless it was manually toggled.
    pub expand_on_hover: bool,
    /// When true, the island completely hides when idle and unhovered,
    /// popping down into the pill format on top-edge hover (macOS / iOS behavior).
    /// When false, it rests as the minimal dot or compact pill.
    pub auto_hide: bool,
    /// Forward Windows app toast notifications to the island as transient
    /// alert banners (app name plus the first text lines). Local-only:
    /// nothing leaves the machine. Needs notification-listener access
    /// Forward Windows app toast notifications to the island as transient
    /// alert banners (app name plus the first text lines). Local-only:
    /// nothing leaves the machine. Needs notification-listener access
    /// (Settings > Privacy > Notifications); without it the watcher exits
    /// silently and the island is unaffected.
    pub forward_toasts: bool,
    /// Waybar-style status bar configuration when `layout == IslandLayout::Bar`.
    pub bar: BarConfig,
}

impl Default for IslandConfig {
    fn default() -> Self {
        Self {
            layout: IslandLayout::Bar,
            collapsed_width: DEFAULT_COLLAPSED_W,
            expanded_width: DEFAULT_EXPANDED_W,
            animation_ms: DEFAULT_ANIM_MS,
            collapse_ms: DEFAULT_COLLAPSE_MS,
            alert_ms: DEFAULT_ALERT_MS,
            minimal_width: DEFAULT_MINIMAL_W,
            height: DEFAULT_HEIGHT,
            corner_radius: DEFAULT_RADIUS,
            y_offset: DEFAULT_Y_OFFSET,
            spring_bounce: DEFAULT_SPRING_BOUNCE,
            theme: "liquid-dark".to_string(),
            glass: GlassConfig::default(),
            scale_with_dpi: true,
            show_tasks: false,
            max_thumbnails: 4,
            face_animated: true,
            ring_metric: "battery".to_string(),
            widgets: Self::default_widgets(),
            expand_on_hover: true,
            auto_hide: false,
            forward_toasts: true,
            bar: BarConfig::default(),
        }
    }
}

impl IslandConfig {
    /// The stock Waybar module set: optional face, agent session dots, and
    /// media activity. The bar keeps these compact; details open only on click.
    pub fn default_widgets() -> Vec<String> {
        ["face", "agents", "music"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    /// Whether the named widget is enabled.
    pub fn has_widget(&self, name: &str) -> bool {
        self.widgets.iter().any(|w| w == name)
    }

    pub fn clamp(&mut self) {
        self.collapsed_width = self.collapsed_width.clamp(MIN_COLLAPSED_W, MAX_COLLAPSED_W);
        self.expanded_width = self.expanded_width.clamp(MIN_EXPANDED_W, MAX_EXPANDED_W);
        // Expanded must be >= collapsed.
        if self.expanded_width < self.collapsed_width {
            self.expanded_width = self.collapsed_width;
        }
        self.height = self.height.clamp(MIN_HEIGHT, MAX_HEIGHT);
        self.corner_radius = self
            .corner_radius
            .clamp(MIN_RADIUS, MAX_RADIUS)
            .min(self.height / 2);
        self.y_offset = self.y_offset.clamp(MIN_Y_OFFSET, MAX_Y_OFFSET);
        self.animation_ms = self.animation_ms.clamp(MIN_ANIM_MS, MAX_ANIM_MS);
        self.collapse_ms = self.collapse_ms.clamp(MIN_ANIM_MS, MAX_ANIM_MS);
        self.alert_ms = self.alert_ms.clamp(MIN_ANIM_MS, MAX_ANIM_MS);
        self.spring_bounce = self
            .spring_bounce
            .clamp(MIN_SPRING_BOUNCE, MAX_SPRING_BOUNCE);
        // A pill narrower than the height is unreadable; keep it elliptical.
        self.minimal_width = self
            .minimal_width
            .clamp(MIN_MINIMAL_W.max(self.height + 8), MAX_MINIMAL_W);
        if self.theme.is_empty() {
            self.theme = "liquid-dark".to_string();
        }
        // Truncate theme to sane length.
        if self.theme.len() > 64 {
            let mut boundary = 64;
            while !self.theme.is_char_boundary(boundary) {
                boundary -= 1;
            }
            self.theme.truncate(boundary);
        }
        self.max_thumbnails = self.max_thumbnails.min(6);
        if !matches!(self.ring_metric.as_str(), "battery" | "cpu" | "mem") {
            self.ring_metric = "battery".to_string();
        }
        self.bar.modules_center = self
            .bar
            .modules_center
            .iter()
            .map(|module| {
                if module == "island" {
                    "termielle"
                } else {
                    module.as_str()
                }
            })
            .filter(|module| *module == "termielle")
            .map(str::to_owned)
            .collect();
        self.widgets
            .retain(|w| matches!(w.as_str(), "face" | "tasks" | "agents" | "music"));
        self.widgets.truncate(16);
        self.glass.clamp();
        // Bar geometry must stay renderable: pill rows need ~18px, and an
        // unclamped height turns `(bar_h - 10)` into a u32 underflow (release
        // wrap → gigantic pill → GUI-thread hang). Margin never eats more
        // than half the bar, so the visual strip keeps positive height.
        self.bar.height = self.bar.height.clamp(24, 64);
        self.bar.margin = self.bar.margin.clamp(0, self.bar.height / 2);
        self.bar.corner_radius = self.bar.corner_radius.clamp(0, self.bar.height / 2);
    }

    /// Whether island/notch rendering is enabled.
    pub fn is_enabled(&self) -> bool {
        !matches!(self.layout, IslandLayout::Classic)
    }

    /// True when attached to top edge (flat top, rounded bottom).
    pub fn is_attached(&self) -> bool {
        matches!(self.layout, IslandLayout::Notch)
    }

    /// True when running as a Waybar-style status bar.
    pub fn is_bar(&self) -> bool {
        matches!(self.layout, IslandLayout::Bar)
    }

    /// Current geometry for a given expansion progress 0.0-1.0.
    pub fn geometry_for(&self, progress: f32) -> IslandGeometry {
        let p = progress.clamp(0.0, 1.0);
        let w = (self.collapsed_width as f32
            + (self.expanded_width as f32 - self.collapsed_width as f32) * p)
            .round() as u32;
        IslandGeometry {
            width: w,
            height: self.height,
            radius: self.corner_radius,
            attached: self.is_attached(),
            y_offset: self.y_offset,
        }
    }
}

/// Resolved pixel geometry for one frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IslandGeometry {
    pub width: u32,
    pub height: u32,
    pub radius: u32,
    pub attached: bool,
    pub y_offset: i32,
}

// ---- helpers --------------------------------------------------------------

/// Apple's spring model (WWDC23 "Animate with springs"): converts a
/// perceptual duration + bounce into physical stiffness/damping/mass so the
/// pill morphs with the same organic overshoot as the iOS Dynamic Island.
///
/// - `stiffness = (2π / duration)²`
/// - `damping = (1 - bounce) · 4π / duration` (bounce >= 0)
#[derive(Clone, Copy, Debug)]
pub struct SpringParams {
    pub stiffness: f32,
    pub damping: f32,
}

/// Derives spring parameters; `duration_ms` is the perceptual duration.
pub fn spring_params(duration_ms: u32, bounce: f32) -> SpringParams {
    let duration = (duration_ms.max(1) as f32) / 1000.0;
    let bounce = bounce.clamp(0.0, 0.5);
    SpringParams {
        stiffness: (std::f32::consts::TAU / duration).powi(2),
        damping: (1.0 - bounce) * 4.0 * std::f32::consts::PI / duration,
    }
}

#[cfg(test)]
mod spring_tests {
    use super::*;

    #[test]
    fn spring_params_match_apple_formulas() {
        let s = spring_params(500, 0.0);
        // duration 0.5s -> stiffness = (2π/0.5)² ≈ 157.9; damping = 4π/0.5 ≈ 25.1
        assert!((s.stiffness - 157.91).abs() < 0.1, "{}", s.stiffness);
        assert!((s.damping - 25.13).abs() < 0.1, "{}", s.damping);
        // Bounce reduces damping.
        let b = spring_params(500, 0.25);
        assert!(b.damping < s.damping);
        assert_eq!(b.stiffness, s.stiffness);
    }
}

/// Anchor a top-center rect inside `work` (logical pixels).
pub fn island_anchored_position(
    width: i32,
    height: i32,
    attached: bool,
    y_offset: i32,
    work: (i32, i32, i32, i32),
) -> (i32, i32) {
    let (left, top, right, _bottom) = work;
    let work_w = right - left;
    let x = left + (work_w - width) / 2;
    let y = if attached || height <= 4 {
        top
    } else {
        top + y_offset
    };
    let _ = height;
    (x, y)
}

/// Anchor a full-width status bar inside `work`. Unit-agnostic: callers pass
/// device pixels (`present_with_bar` with physical monitor bounds) and get
/// device pixels back — never mix logical here.
pub fn bar_anchored_position(
    height: i32,
    position: BarPosition,
    work: (i32, i32, i32, i32),
) -> (i32, i32) {
    let (left, top, _right, bottom) = work;
    let x = left;
    let y = match position {
        BarPosition::Top => top,
        BarPosition::Bottom => bottom - height,
    };
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_limit_is_utf8_safe() {
        let mut config = IslandConfig {
            theme: "界".repeat(30),
            ..Default::default()
        };
        config.clamp();
        assert_eq!(config.theme, "界".repeat(21));
    }

    #[test]
    fn defaults_are_waybar_ready() {
        let c = IslandConfig::default();
        assert_eq!(c.layout, IslandLayout::Bar);
        assert!(c.expand_on_hover);
        assert!(c.face_animated);
        for w in ["face", "agents", "music"] {
            assert!(c.has_widget(w), "missing widget {w}");
        }
        assert!(!c.has_widget("ring"));
        assert!(!c.show_tasks);
        assert_eq!(c.glass.blur_radius, 12);
    }

    #[test]
    fn ring_metric_falls_back_to_battery() {
        let mut c = IslandConfig {
            ring_metric: "quota".into(),
            ..Default::default()
        };
        c.clamp();
        assert_eq!(c.ring_metric, "battery");
        let mut c = IslandConfig {
            ring_metric: "cpu".into(),
            ..Default::default()
        };
        c.clamp();
        assert_eq!(c.ring_metric, "cpu");
    }

    #[test]
    fn clamp_drops_unknown_widgets() {
        let mut c = IslandConfig {
            widgets: vec!["clock".into(), "party-mode".into(), "face".into()],
            ..Default::default()
        };
        c.clamp();
        // Text widgets are gone in the visual-only dashboard.
        assert_eq!(c.widgets, vec!["face".to_string()]);
    }

    #[test]
    fn defaults_are_sane() {
        let c = IslandConfig::default();
        assert_eq!(c.layout, IslandLayout::Bar);
        assert!(c.collapsed_width < c.expanded_width);
        assert_eq!(c.glass.tint[3], 190);
    }

    #[test]
    fn clamp_expanded_below_collapsed() {
        let mut c = IslandConfig {
            collapsed_width: 300,
            expanded_width: 100,
            ..Default::default()
        };
        c.clamp();
        assert!(c.expanded_width >= c.collapsed_width);
    }

    #[test]
    fn clamp_radius_to_half_height() {
        let mut c = IslandConfig {
            height: 30,
            corner_radius: 40,
            ..Default::default()
        };
        c.clamp();
        assert_eq!(c.corner_radius, 15);
    }

    #[test]
    fn geometry_lerp() {
        let c = IslandConfig {
            layout: IslandLayout::Island,
            collapsed_width: 100,
            expanded_width: 200,
            height: 36,
            corner_radius: 18,
            ..Default::default()
        };
        assert_eq!(c.geometry_for(0.0).width, 100);
        assert_eq!(c.geometry_for(1.0).width, 200);
        assert_eq!(c.geometry_for(0.5).width, 150);
    }

    #[test]
    fn anchored_position_centers() {
        // work 0,0,1920,1080, island 140x36 floating y=8 -> x=890, y=8
        let (x, y) = island_anchored_position(140, 36, false, 8, (0, 0, 1920, 1080));
        assert_eq!(x, 890);
        assert_eq!(y, 8);
        let (x2, y2) = island_anchored_position(140, 36, true, 8, (0, 0, 1920, 1080));
        assert_eq!(x2, 890);
        assert_eq!(y2, 0);
    }

    #[test]
    fn missing_new_flags_inherit_struct_defaults() {
        // Old config files predate `forward_toasts`: container-level
        // `#[serde(default)]` must fill them from `Default`, not `false`.
        let c: IslandConfig = serde_json::from_str(r#"{"layout":"island"}"#).unwrap();
        assert!(c.forward_toasts);
        assert!(!c.is_bar());
    }

    #[test]
    fn bar_layout_and_config_serde() {
        let json = r#"{
            "layout": "bar",
            "bar": {
                "position": "bottom",
                "height": 40,
                "replace_taskbar": true,
                "reserve_space": true
            }
        }"#;
        let c: IslandConfig = serde_json::from_str(json).unwrap();
        assert!(c.is_bar());
        assert!(c.is_enabled());
        assert_eq!(c.bar.position, BarPosition::Bottom);
        assert_eq!(c.bar.height, 40);
        assert!(c.bar.replace_taskbar);
        assert!(c.bar.reserve_space);
        // New flags default on for old configs (container #[serde(default)]).
        assert!(c.bar.follow_active_monitor);
    }

    #[test]
    fn bar_anchored_position_computes_correctly() {
        let work = (0, 0, 1920, 1080);
        let (top_x, top_y) = bar_anchored_position(36, BarPosition::Top, work);
        assert_eq!(top_x, 0);
        assert_eq!(top_y, 0);

        let (bot_x, bot_y) = bar_anchored_position(36, BarPosition::Bottom, work);
        assert_eq!(bot_x, 0);
        assert_eq!(bot_y, 1080 - 36);
    }
}
