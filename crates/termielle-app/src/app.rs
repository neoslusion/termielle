//! The controller: folds pipe events and timer ticks into window actions.
//!
//! [`Controller`] owns the reducer, the active animation, and the single
//! deadline the window's timer is armed for. It never touches Win32 itself;
//! [`main`](crate::main) maps its actions onto the window, the pipe, and the
//! log.

use crate::animation::notch::BRIDGE_K_MAX;
use crate::animation::spring::{BLOB_GAP_PX, Spring1, Spring2D};
use crate::animation::{
    AnimationError, AnimationSource, FrameBuffer, GifAnimation, fallback_frame,
};
use crate::tasks::{MediaInfo, WorkerUpdate};
use crate::window::scaled_size;
use std::collections::VecDeque;
use termielle_core::{
    AssetCatalog, EventMessage, IslandConfig, SessionReducer, VisualState, spring_params,
};

/// Edge length of the cached termielle face. Larger than any display size
/// (faces show at <= 32 logical px, so 96 stays crisp through render scales
/// up to 3x), so the downsample always resolves real detail instead of blur.
const FACE_SIZE: u32 = 96;

/// Square edge length of procedural fallback frames.
pub const FALLBACK_FRAME_SIZE: u32 = 360;

/// How old an event may be while still raising an alert banner. Live pipe
/// delivery is instant, so anything older is a journal replay or a delayed
/// ghost — and a notification about a days-old prompt is not news. The
/// reducer still folds the event into state; only the banner is gated.
const ALERT_FRESHNESS_MS: u64 = 60_000;

/// What the GUI thread must do after one controller step.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ControllerActions {
    /// Set when the visible state changed and its animation was reloaded.
    pub visible_state: Option<VisualState>,
    /// True when the frame changed and the window should repaint.
    pub present_frame: bool,
    /// Nearest reducer or animation deadline, for the single Win32 timer.
    pub next_deadline_ms: Option<u64>,
    /// Numeric code of an asset that failed to decode this step, if any.
    pub error_code: Option<i32>,
}

pub const HIT_MEDIA_PLAY_PAUSE: isize = -1;
pub const HIT_MEDIA_PREV: isize = -2;
pub const HIT_MEDIA_NEXT: isize = -3;
pub const HIT_ALERT_DISMISS: isize = -4;

/// What a click on the island should do.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClickOutcome {
    /// Toggle play/pause on the system media session.
    MediaToggle,
    /// Skip to previous track.
    MediaPrev,
    /// Skip to next track.
    MediaNext,
    /// Dismiss notification banner.
    AlertDismiss,
    /// Activate a specific window (HWND).
    ActivateWindow(isize),
    /// Switch to a virtual desktop workspace by index.
    WorkspaceSwitch(u32),
    /// Toggle system audio mute.
    VolumeToggle,
    /// The dashboard opened.
    Expanded,
    /// The dashboard closed.
    Collapsed,
    /// Click landed outside any interactive element.
    None,
}

/// Procedural-motion repaint cadence in ms (~20 fps): thinking bounce,
/// worker orbit, input pulse, celebration/shake one-shots, and the media
/// equalizer all ride this clock so they move even with no other deadline
/// pending. Cheap (cached glass, one present) and idle-silent.
const MOTION_TICK_MS: u64 = 50;

/// Cache key for the frosted-glass layer: geometry, material, blob layout,
/// and the authoring scale — a DPI or zoom change misses and rebuilds
/// instead of presenting stale-resolution glass.
type GlassCacheKey = (u32, u32, u32, bool, bool, u32, i32, u32, u32);

/// How many notification banners queue behind the showing one; arrivals
/// beyond that drop the longest-waiting unseen banner, never the showing one.
const MAX_QUEUED_ALERTS: usize = 3;

/// Transient alert banner displayed in the Dynamic Island on notifications.

#[derive(Clone, Debug)]
pub struct AlertBanner {
    pub title: String,
    pub subtitle: String,
    pub accent: [u8; 4],
    pub expires_at_ms: u64,
    pub duration_ms: u64,
}

/// Cached system metrics for Waybar mode, refreshed on a 1-second cadence
/// instead of blocking the render loop on every frame.
#[derive(Clone, Debug)]
pub(crate) struct BarMetricsCache {
    pub last_query_ms: u64,
    pub workspaces: crate::bar::workspaces::WorkspaceSnapshot,
    pub window_title: String,
    pub time_str: String,
    pub battery: (Option<u8>, bool),
    pub volume: crate::bar::volume::VolumeSnapshot,
    pub memory_pct: u8,
    pub cpu_pct: u8,
}
impl BarMetricsCache {
    /// Empty snapshot for fully-hidden bars: renders nothing and performs
    /// no system queries. `last_query_ms` is zero so re-showing a module
    /// refetches immediately instead of serving stale emptiness for 900ms.
    fn empty() -> Self {
        Self {
            last_query_ms: 0,
            workspaces: crate::bar::workspaces::WorkspaceSnapshot {
                total: 0,
                active: 0,
            },
            window_title: String::new(),
            time_str: String::new(),
            battery: (None, false),
            volume: crate::bar::volume::VolumeSnapshot {
                level: 0,
                muted: true,
            },
            memory_pct: 0,
            cpu_pct: 0,
        }
    }
}
/// One bar-module hit target: (id, x, y, w, h) in frame coordinates.
/// Collected per zone and installed as [`Controller::icon_hits`] by the
/// center painter, which owns the island interaction state.
type BarHit = (isize, i32, i32, u32, u32);

/// Expanded-card geometry for the bar center painter: where the drop-down
/// card lives plus whether this frame is expanded at all.
struct BarCard {
    island_x: i32,
    island_y: i32,
    island_w: u32,
    exp_h: u32,
    expanded: bool,
}
/// Shared inputs for one expanded-card section paint pass: frame geometry
/// plus the precomputed clock/accent values, so six section painters do
/// not each re-query the registry or recompute the pose clock.
struct CardPaintCtx {
    width: u32,
    height: u32,
    cy: i32,
    accent: [u8; 4],
    now: u64,
    eq_now: u64,
    pad: i32,
}

/// Ties the session reducer, the animation pipeline, and one deadline together.
///
/// All times are Unix epoch milliseconds, the same base the emitter stamps
/// events with. All animation objects stay on the thread that constructed the
/// controller, because [`GifAnimation`] is deliberately not `Send`.
pub struct Controller {
    reducer: SessionReducer,
    assets: AssetCatalog,
    reduced_motion: bool,
    state: VisualState,
    animation: AnimationSource,
    /// Frame just decoded, cloned out of the reused decoder canvas.
    current: FrameBuffer,
    /// When the next GIF frame is due; `None` for stills and reduced motion.
    frame_deadline: Option<u64>,
    /// Next procedural-motion repaint; `None` when nothing moves.
    motion_deadline: Option<u64>,
    /// Fixed interval between animation frames in milliseconds; when set, it
    /// overrides each frame's GIF delay so the animation plays at a constant
    /// frame rate (e.g. 60 fps) instead of the file's own timing.
    frame_interval_ms: Option<u64>,
    /// Smoothed cost of presenting one frame, in milliseconds.
    present_cost_ms: u64,
    island: IslandConfig,
    /// Active 2D spring (island mode); `None` when settled or classic.
    spring: Option<Spring2D>,
    /// Timestamp of the last spring integration (for real elapsed dt).
    spring_last_ms: u64,
    /// Current animated corner radius; the source of truth the spring
    /// integrates toward [`Self::target_radius`].
    radius: f32,
    /// Active blob-separation spring; `None` when the island is merged.
    separation: Option<Spring1>,
    /// True while the pointer is held down on the pill: the island swells
    /// slightly, like the Dynamic Island under the fingertip.
    pressed: bool,
    manually_expanded: bool,
    /// Set while the cursor hovers the island (when `expand_on_hover`).
    hover_expanded: bool,
    /// Pre-decoded termielle face frames (downscaled to [`FACE_SIZE`]) plus
    /// per-frame delays in ms. Empty means the static procedural fallback.
    face_frames: Vec<FrameBuffer>,
    face_delays: Vec<u32>,
    face_idx: usize,
    /// Streaming GIF decoder kept while the face loop is still filling (see
    /// `refresh_face`). `None` once the loop is complete or the face is
    /// static.
    face_decoder: Option<GifAnimation>,
    /// Next face-animation tick; `None` for stills, reduced motion, classic
    /// mode, or `face_animated: false`.
    face_deadline: Option<u64>,
    /// The face frame currently composited into the notch.
    face_frame: FrameBuffer,
    /// Live running-task icons for the expanded pill. Filled by the
    /// background worker via [`Controller::set_task_update`]; the GUI thread
    /// never captures, so hovering and morphing never stall.
    /// Hit rects for the task icons of the last rendered frame:
    /// (hwnd, x, y, w, h) in frame coordinates. Used for click activation.
    icon_hits: Vec<(isize, i32, i32, u32, u32)>,
    /// Cursor position in frame coordinates (from the hover poll), so the
    /// icon under the cursor draws its accent border.
    hover_point: Option<(i32, i32)>,
    /// Cached frosted-glass layer keyed by (w, h, radius, attached). The
    /// single-pass paint is ~1ms; caching makes face ticks ~free.
    /// Cached frosted-glass layer keyed by (w, h, radius, attached, black,
    /// blob count, media-blob x, media-blob w). The single-pass paint is
    /// ~1ms; caching makes face ticks ~free.
    glass_cache: Option<(GlassCacheKey, FrameBuffer)>,
    /// Last now-playing media state from the background worker.
    media: Option<MediaInfo>,
    /// Open window task icons from the background worker.
    tasks: Vec<crate::tasks::TaskIcon>,
    /// Queued notification alert banners; the front one shows.
    alerts: VecDeque<AlertBanner>,
    /// Last controller-clock time, for renders without their own timestamp.
    clock_ms: u64,
    /// When the current visual state began, for one-shot state effects.
    state_since_ms: u64,
    /// Monitor DPI scale (physical px per logical px), refreshed from the
    /// window on every present. 1.0 until the first present.
    dpi_scale: f32,
    /// User zoom from `AppConfig.scale`, applied on top of DPI.
    user_scale: f32,
    /// Monitor logical width for Waybar layout.
    bar_width: u32,
    /// Next bar periodic refresh deadline; None when not in bar mode.
    bar_deadline: Option<u64>,
    /// Cached bar metrics to prevent heavy COM/Registry/CPU queries on every render.
    bar_metrics_cache: Option<BarMetricsCache>,
}

impl Controller {
    /// Builds a controller showing the Idle state, ready to present.
    /// `frame_interval_ms` overrides the GIF's per-frame delays when set.
    pub fn new(
        ready_hold_ms: u64,
        busy_stall_ms: u64,
        assets: AssetCatalog,
        reduced_motion: bool,
        frame_interval_ms: Option<u64>,
    ) -> Self {
        Self::new_with_island(
            ready_hold_ms,
            busy_stall_ms,
            assets,
            reduced_motion,
            frame_interval_ms,
            IslandConfig::default(),
        )
    }

    /// Builds a controller with explicit island config.
    pub fn new_with_island(
        ready_hold_ms: u64,
        busy_stall_ms: u64,
        assets: AssetCatalog,
        reduced_motion: bool,
        frame_interval_ms: Option<u64>,
        island: IslandConfig,
    ) -> Self {
        let state = VisualState::Idle;
        let face_frame = fallback_frame(state, FACE_SIZE, 1.0);
        // Overwritten below for both island and classic modes.
        let still = fallback_frame(state, FALLBACK_FRAME_SIZE, 1.0);
        let (current, animation, frame_deadline) =
            (still.clone(), AnimationSource::Still(still), None);
        let radius = island.height as f32 / 2.0;
        let mut controller = Self {
            reducer: SessionReducer::new(ready_hold_ms, busy_stall_ms),
            assets,
            reduced_motion,
            state,
            animation,
            current,
            frame_deadline,
            motion_deadline: None,
            frame_interval_ms,
            present_cost_ms: 0,
            island,
            spring: None,
            spring_last_ms: 0,
            radius,
            separation: None,
            pressed: false,
            manually_expanded: false,
            hover_expanded: false,
            face_frames: Vec::new(),
            face_delays: Vec::new(),
            face_idx: 0,
            face_decoder: None,
            face_deadline: None,
            face_frame,
            icon_hits: Vec::new(),
            hover_point: None,
            media: None,
            tasks: Vec::new(),
            glass_cache: None,
            clock_ms: 0,
            state_since_ms: 0,
            alerts: VecDeque::new(),
            dpi_scale: 1.0,
            user_scale: 1.0,
            bar_width: 1920,
            bar_deadline: None,
            bar_metrics_cache: None,
        };
        if controller.island.is_enabled() {
            controller.refresh_face(state);
            let (w, h) = controller.target_size(state);
            controller.current = controller.render_island(state, w, h, 0);
            controller.arm_face_deadline(0);
        } else {
            let _ = controller.load_animation_classic(VisualState::Idle, 0);
        }
        controller
    }

    /// Reloads the termielle face for `state`: decodes the first animation
    /// frame synchronously (fast startup — the pipe must be listening within
    /// milliseconds) and keeps the streaming decoder so the remaining loop
    /// frames fill in one per face tick (see `advance_face`). Falls back to
    /// the procedural glyph when the asset is missing or undecodable.
    fn refresh_face(&mut self, state: VisualState) {
        self.face_frames.clear();
        self.face_delays.clear();
        self.face_idx = 0;
        self.face_deadline = None;
        self.face_decoder = None;
        self.face_frame = fallback_frame(state, FACE_SIZE, 1.0);
        let Some(path) = self.assets.resolve(state) else {
            return;
        };
        let Ok(mut gif) = GifAnimation::open(&path) else {
            return;
        };
        let Ok(first) = gif.next_frame() else {
            return;
        };
        if first.loop_index > 0 {
            return;
        }
        self.face_delays.push(first.delay_ms.clamp(20, 1000));
        self.face_frames.push(downscale_frame(first, FACE_SIZE));
        self.face_frame = self.face_frames[0].clone();
        // Animated faces keep streaming in the background of face ticks;
        // stills drop the decoder immediately.
        if self.island.face_animated && !self.reduced_motion {
            self.face_decoder = Some(gif);
        }
    }

    /// Arms the next face-animation tick when the face can advance: island
    /// mode, animated faces, motion allowed, and either frames still filling
    /// or 2+ frames decoded.
    fn arm_face_deadline(&mut self, now_ms: u64) {
        self.face_deadline = None;
        if !self.island.is_enabled() || self.reduced_motion || !self.island.face_animated {
            return;
        }
        if self.face_decoder.is_none() && self.face_frames.len() < 2 {
            return;
        }
        let delay = self
            .face_delays
            .get(self.face_idx)
            .copied()
            .unwrap_or(40)
            .max(20);
        self.face_deadline = Some(now_ms.saturating_add(delay as u64));
    }

    /// Advances the face animation one step and re-renders the current pill:
    /// decodes one more loop frame while filling, then cycles the cache.
    fn advance_face(&mut self, now_ms: u64) {
        if self.presentation() == crate::animation::notch::Presentation::Hidden {
            // Invisible top-edge sensor: keep the tick cadence (and the
            // background loop fill above), skip the pixels nobody sees.
            self.arm_face_deadline(now_ms);
            return;
        }
        if let Some(gif) = self.face_decoder.as_mut() {
            let done = match gif.next_frame() {
                Ok(frame) if frame.loop_index == 0 && self.face_frames.len() < 120 => {
                    self.face_delays.push(frame.delay_ms.clamp(20, 1000));
                    self.face_frames.push(downscale_frame(frame, FACE_SIZE));
                    false
                }
                _ => true,
            };
            if done {
                self.face_decoder = None;
            }
        }
        if self.face_frames.len() < 2 {
            // Single-frame face: nothing to cycle; re-arm only while filling.
            if self.face_decoder.is_none() {
                self.face_deadline = None;
            } else {
                let delay = self
                    .face_delays
                    .get(self.face_idx)
                    .copied()
                    .unwrap_or(40)
                    .max(20);
                self.face_deadline = Some(now_ms.saturating_add(delay as u64));
            }
            return;
        }
        self.face_idx = (self.face_idx + 1) % self.face_frames.len();
        self.face_frame = self.face_frames[self.face_idx].clone();
        let delay = self
            .face_delays
            .get(self.face_idx)
            .copied()
            .unwrap_or(40)
            .max(20);
        self.face_deadline = Some(now_ms.saturating_add(delay as u64));
        let (w, h) = self.current_logical_size();
        self.current = self.render_island(self.state, w, h, now_ms);
        self.animation = AnimationSource::Still(self.current.clone());
    }

    /// Renders one island frame for `state`. The frosted-glass layer is
    /// cached per geometry; content elements (face, task icons, session
    /// dots) are **evenly distributed** across the pill width instead of
    /// clumping left, and icon hit-rects are recorded for click activation.
    /// Renders one island frame for `state`: the blob silhouette material,
    /// the presentation content composited over it so content rides the
    /// morph, then the accent strip. Icon hit-rects are recorded for click
    /// activation.
    fn render_island(
        &mut self,
        state: VisualState,
        width: u32,
        height: u32,
        now_ms: u64,
    ) -> FrameBuffer {
        if self.island.is_bar() {
            return self.render_bar(state, width, height, now_ms);
        }

        use crate::animation::notch::Presentation;

        let presentation = self.presentation();
        if presentation == Presentation::Hidden || height <= 4 {
            self.icon_hits.clear();
            return self.blank_frame(width, height);
        }
        if self.spring.is_none() {
            self.radius = self.target_radius();
        }
        let island = self.island.clone();
        let attached = island.is_attached();
        let radius = (self.radius.round() as u32).max(1).min(height / 2);
        let blobs = self.blob_rects(width, height, radius, attached);
        let black = island.glass.notch_black && attached;
        let mut frame = self.glass_layer_blobs(width, height, radius, attached, &blobs, black);

        // Content rides the morph: it is drawn on its own canvas and
        // composited with an enter alpha (plus a slight rise for the
        // expanded card), so it fades and settles with the spring instead
        // of popping when the container lands.
        let mut content = self.blank_frame(width, height);
        self.render_content(
            &mut content,
            state,
            &island,
            presentation,
            width,
            height,
            &blobs,
            now_ms,
        );
        let age_ms = now_ms.saturating_sub(self.state_since_ms);
        let (alpha, dx, dy) =
            Self::content_motion(self.spring.as_ref(), presentation, state, age_ms);
        crate::animation::notch::blend_frame_over(&mut frame, &content, dx, dy, alpha);

        // Accent strip: agent state color; accent color while media plays.
        // Part of the silhouette, so it never fades with the content.
        let (state_color, _) = crate::animation::notch::accent_colors(state);
        let strip = if state != VisualState::Idle {
            state_color
        } else if self.media_playing() && island.has_widget("music") {
            crate::system::accent_color_bgra()
        } else {
            return frame;
        };
        // Awaiting input breathes: the strip pulses slowly so "waiting"
        // never reads as dead.
        let strip = if state == VisualState::NeedsInput {
            let pulse =
                (0.5 + 0.5 * (now_ms as f32 / 450.0 * std::f32::consts::TAU).sin()).clamp(0.0, 1.0);
            let mut lit = strip;
            lit[3] = (140.0 + 115.0 * pulse).round() as u8;
            lit
        } else {
            strip
        };
        crate::animation::notch::draw_accent_strip(&mut frame, attached, radius, strip);
        frame
    }

    /// Bar strip row geometry from raw config values (all logical px):
    /// `(margin, vis_off, vis_h, pill_off, pill_h, radius)` — horizontal
    /// margin, visual-strip offset within the bar strip, visual height,
    /// module-pill offset/height centered in the visual strip, and the
    /// clamped blob radius. Pure so tests can pin it without a Controller.
    /// `render_bar` adds `bar_y`; the collapsed hover sensor uses strip-local
    /// coords directly (collapsed frames put the strip at y=0).
    fn bar_row_geometry(
        bar_h: u32,
        top: bool,
        edge_to_edge: bool,
        margin: u32,
        corner_radius: u32,
    ) -> (u32, i32, u32, i32, u32, u32) {
        let margin = if edge_to_edge {
            0
        } else {
            margin.min(bar_h / 2)
        };
        let vis_h = bar_h.saturating_sub(margin).max(12);
        let pill_h = (vis_h.saturating_sub(10)).max(12).min(vis_h);
        let vis_off = if top { margin as i32 } else { 0 };
        let pill_off = vis_off + ((vis_h.saturating_sub(pill_h)) / 2) as i32;
        let radius = corner_radius.min(vis_h / 2);
        (margin, vis_off, vis_h, pill_off, pill_h, radius)
    }

    /// Row geometry for the live config: (margin, vis_y, vis_h, pill_y,
    /// pill_h, bar_radius) in collapsed-frame coords. `render_bar` shifts by
    /// `bar_y` for expanded-bottom frames.
    fn bar_row(&self) -> (u32, i32, u32, i32, u32, u32) {
        let bar = &self.island.bar;
        let (margin, vis_off, vis_h, pill_off, pill_h, radius) = Self::bar_row_geometry(
            bar.height,
            bar.position == termielle_core::BarPosition::Top,
            bar.edge_to_edge,
            bar.margin,
            bar.corner_radius,
        );
        (margin, vis_off, vis_h, pill_off, pill_h, radius)
    }

    /// Collapsed center-pill rect (x, y, w, h), logical px. Single source for
    /// `render_bar`, its hit target, and the `set_hover` sensor.
    fn bar_pill_rect(&self, width: u32) -> (i32, i32, u32, u32) {
        let (_, _, _, pill_off, pill_h, _) = self.bar_row();
        let pill_w = 180u32.min(width / 3).max(120);
        let pill_cx = ((width.saturating_sub(pill_w)) / 2) as i32;
        (pill_cx, pill_off, pill_w, pill_h)
    }

    /// Whether a bar module name is listed in its zone (`left`, `center`,
    /// `right`). Unknown names are ignored; order within a zone is fixed by
    /// the renderer. An emptied zone collapses (neighbors do not reflow).
    fn bar_module(&self, zone: &str, name: &str) -> bool {
        let list = match zone {
            "left" => &self.island.bar.modules_left,
            "center" => &self.island.bar.modules_center,
            _ => &self.island.bar.modules_right,
        };
        list.iter().any(|m| m == name)
    }

    /// Expanded island-card rect (x, y, w, h), logical px. Width matches the
    /// `render_bar` card blob (including `.max(300)`); height derives from
    /// the same `target_size` the renderer uses, so the sensor tracks the
    /// real 154..210 card instead of a hardcoded 210.
    fn bar_card_rect(&self) -> (i32, i32, i32, i32) {
        let bar_h = self.island.bar.height as i32;
        let is_top = self.island.bar.position == termielle_core::BarPosition::Top;
        let island_w = self
            .island
            .expanded_width
            .min(self.bar_width.saturating_sub(40))
            .max(300) as i32;
        let island_x = ((self.bar_width as i32 - island_w) / 2).max(0);
        let exp_h = (self.target_size(self.state).1 as i32)
            .saturating_sub(bar_h)
            .max(0);
        let y = if is_top { bar_h } else { 0 };
        (island_x, y, island_w, exp_h)
    }

    /// Renders a Waybar-style full-width acrylic status bar with an embedded Dynamic Island.
    fn render_bar(
        &mut self,
        state: VisualState,
        width: u32,
        height: u32,
        now_ms: u64,
    ) -> FrameBuffer {
        self.icon_hits.clear();

        let bar_h = self.island.bar.height;
        let is_top = self.island.bar.position == termielle_core::BarPosition::Top;
        let bar_y = if is_top {
            0
        } else {
            (height.saturating_sub(bar_h)) as i32
        };
        let expanded = height > bar_h;
        let exp_h = height.saturating_sub(bar_h);

        // 1. Compute glass blobs: the horizontal bar + optional downward (or upward) expanded island card
        let island_w = self
            .island
            .expanded_width
            .min(width.saturating_sub(40))
            .max(300);
        let island_x = ((width.saturating_sub(island_w)) / 2) as i32;
        let island_y = if is_top { bar_h as i32 } else { 0 };

        // Visual strip after margin (floating look when edge_to_edge is
        // false). The window stays full-width; the margin stays transparent
        // and click-through. Row metrics come from bar_row_geometry so the
        // renderer, hit targets, and hover sensor share one source.
        let (margin, vis_off, vis_h, pill_off, pill_h, bar_r) = self.bar_row();
        let bar_x = margin as i32;
        let bar_w = width.saturating_sub(margin * 2);
        let blob_y = bar_y + vis_off;
        let pill_y = bar_y + pill_off;

        let mut blobs = vec![crate::animation::notch::BlobRect {
            x: bar_x,
            y: blob_y,
            w: bar_w,
            h: vis_h,
            r: bar_r,
            attached: true,
        }];
        // Grafted while expanded: overlap the card into the bar by its corner
        // radius so the smooth-min union merges both blobs into one
        // silhouette. The card grows out of the bar (island morphology)
        // instead of floating over it as a separate object with a seam;
        // content still composes at island_x/island_y below.
        let grafted = expanded && exp_h > 4;
        if grafted {
            const GRAFT: i32 = 20;
            let (cy, ch) = if is_top {
                (island_y - GRAFT, exp_h + GRAFT as u32)
            } else {
                (island_y, exp_h + GRAFT as u32)
            };
            blobs.push(crate::animation::notch::BlobRect {
                x: island_x,
                y: cy,
                w: island_w,
                h: ch,
                r: 20,
                attached: true,
            });
        }

        // Draw acrylic glass layer for the composite bar silhouette (never solid black)
        let mut frame = self.glass_layer_blobs(width, height, 0, true, &blobs, false);

        // Accent strip on the bar edge
        let (state_color, _) = crate::animation::notch::accent_colors(state);
        let accent = crate::system::accent_color_bgra();
        let strip = if state != VisualState::Idle {
            state_color
        } else if self.media_playing() && self.island.has_widget("music") {
            accent
        } else {
            [accent[0], accent[1], accent[2], 80]
        };
        let strip_y = if is_top {
            blob_y + vis_h as i32 - 2
        } else {
            blob_y
        };
        // Accent strip on the bar edge, split around the grafted card while
        // expanded: a full-width line would stab through the card's buried
        // corners and read as a seam between two objects.
        let strip_segs = if grafted {
            let card_l = island_x.max(bar_x);
            let card_r = (island_x + island_w as i32).min(bar_x + bar_w as i32);
            [
                (bar_x, (card_l - bar_x).max(0) as u32),
                (card_r, (bar_x + bar_w as i32 - card_r).max(0) as u32),
            ]
        } else {
            [(bar_x, bar_w), (0, 0)]
        };
        for (seg_x, seg_w) in strip_segs {
            crate::animation::notch::draw_rounded_rect(
                &mut frame,
                seg_x,
                strip_y,
                seg_w,
                2,
                0,
                strip,
                [0, 0, 0, 0],
            );
        }
        // Module visibility from bar.modules_{left,center,right}. Order is
        // fixed; an emptied zone collapses (neighbors do not reflow). Zones
        // re-check their own flags when painting.
        let show_any = ["workspaces", "window"]
            .iter()
            .any(|m| self.bar_module("left", m))
            || ["clock", "battery", "volume", "memory", "cpu"]
                .iter()
                .any(|m| self.bar_module("right", m))
            || self.bar_module("center", "island");

        // Cached metrics: only query Windows Registry/COM/System once per ~second
        let metrics = if !show_any {
            // Nothing visible: skip every Registry/COM/system query.
            BarMetricsCache::empty()
        } else if let Some(cache) = &self.bar_metrics_cache {
            if now_ms.saturating_sub(cache.last_query_ms) < 900 {
                cache.clone()
            } else {
                let win_title = crate::media::foreground_title();
                let fresh = BarMetricsCache {
                    last_query_ms: now_ms,
                    workspaces: crate::bar::workspaces::query_workspaces(),
                    window_title: win_title,
                    time_str: crate::system::current_time_text(),
                    battery: crate::system::battery_status(),
                    volume: crate::bar::volume::query_volume(),
                    memory_pct: crate::system::memory_info().0,
                    cpu_pct: crate::system::cpu_percent(),
                };
                self.bar_metrics_cache = Some(fresh.clone());
                fresh
            }
        } else {
            let win_title = crate::media::foreground_title();
            let fresh = BarMetricsCache {
                last_query_ms: now_ms,
                workspaces: crate::bar::workspaces::query_workspaces(),
                window_title: win_title,
                time_str: crate::system::current_time_text(),
                battery: crate::system::battery_status(),
                volume: crate::bar::volume::query_volume(),
                memory_pct: crate::system::memory_info().0,
                cpu_pct: crate::system::cpu_percent(),
            };
            self.bar_metrics_cache = Some(fresh.clone());
            fresh
        };

        let card = BarCard {
            island_x,
            island_y,
            island_w,
            exp_h,
            expanded,
        };
        let mut bar_hits = self.paint_bar_left(&mut frame, &metrics, accent, bar_x, pill_y, pill_h);
        bar_hits.extend(self.paint_bar_right(&mut frame, &metrics, width, bar_x, pill_y, pill_h));
        self.paint_bar_center(&mut frame, state, now_ms, width, card, bar_hits);

        frame
    }
    /// Bar zone 2 (left): workspace switcher plus the active-window title
    /// pill. Pure painter: draws into `frame` and returns its hit targets
    /// for the orchestrator to install.
    fn paint_bar_left(
        &self,
        frame: &mut FrameBuffer,
        metrics: &BarMetricsCache,
        accent: [u8; 4],
        bar_x: i32,
        pill_y: i32,
        pill_h: u32,
    ) -> Vec<BarHit> {
        let mut hits = Vec::new();
        // 2. Modules Left: Workspaces + Window Title
        let mut cur_x = bar_x + 12;

        if self.bar_module("left", "workspaces") {
            // Workspaces
            let ws = metrics.workspaces;
            let ws_w = 28u32;
            for i in 1..=ws.total.min(10) {
                let active = i == ws.active;
                let (bg, border, text_col) = if active {
                    (
                        [accent[0], accent[1], accent[2], 190],
                        [255, 255, 255, 140],
                        [255, 255, 255, 255],
                    )
                } else {
                    (
                        [255, 255, 255, 24],
                        [255, 255, 255, 36],
                        [190, 190, 190, 220],
                    )
                };
                crate::animation::notch::draw_rounded_rect(
                    frame,
                    cur_x,
                    pill_y,
                    ws_w,
                    pill_h,
                    pill_h / 2,
                    bg,
                    border,
                );
                let num_str = format!("{}", i);
                let text_x = cur_x + ((ws_w - 8) / 2) as i32;
                let text_y = pill_y + ((pill_h - 12) / 2) as i32;
                crate::animation::notch::draw_text(
                    frame, &num_str, text_x, text_y, 16, 11, true, text_col,
                );

                // Register hit target for workspace switching
                hits.push((
                    crate::bar::HIT_BAR_WORKSPACE_BASE - i as isize,
                    cur_x,
                    pill_y,
                    ws_w,
                    pill_h,
                ));

                cur_x += ws_w as i32 + 6;
            }
        }

        // Window Title
        if self.bar_module("left", "window") && !metrics.window_title.is_empty() {
            cur_x += 6;
            let app_name = crate::media::app_name_from_title(&metrics.window_title);
            let display_text = if app_name.len() < metrics.window_title.len() {
                format!("{} — {}", app_name, metrics.window_title)
            } else {
                metrics.window_title.clone()
            };
            let max_chars = 36;
            let truncated: String = display_text.chars().take(max_chars).collect();
            let title_w = (truncated.chars().count() as u32 * 8 + 24).clamp(60, 260);

            crate::animation::notch::draw_rounded_rect(
                frame,
                cur_x,
                pill_y,
                title_w,
                pill_h,
                pill_h / 2,
                [255, 255, 255, 18],
                [255, 255, 255, 30],
            );
            crate::animation::notch::draw_text(
                frame,
                &truncated,
                cur_x + 10,
                pill_y + ((pill_h - 12) / 2) as i32,
                title_w - 16,
                11,
                false,
                [230, 230, 230, 230],
            );
        }

        hits
    }

    /// Bar zone 3 (right): clock, battery, volume, RAM, and CPU pills.
    /// Pure painter: draws into `frame` and returns its hit targets
    /// for the orchestrator to install.
    fn paint_bar_right(
        &self,
        frame: &mut FrameBuffer,
        metrics: &BarMetricsCache,
        width: u32,
        bar_x: i32,
        pill_y: i32,
        pill_h: u32,
    ) -> Vec<BarHit> {
        let mut hits = Vec::new();
        // 3. Modules Right: Clock, Battery, Volume, RAM, CPU
        let mut cur_right = (width as i32) - bar_x - 12;

        if self.bar_module("right", "clock") {
            // Clock
            let clock_w = 84u32;
            cur_right -= clock_w as i32;
            crate::animation::notch::draw_rounded_rect(
                frame,
                cur_right,
                pill_y,
                clock_w,
                pill_h,
                pill_h / 2,
                [255, 255, 255, 24],
                [255, 255, 255, 36],
            );
            crate::animation::notch::draw_text(
                frame,
                &metrics.time_str,
                cur_right + 8,
                pill_y + ((pill_h - 12) / 2) as i32,
                clock_w - 12,
                11,
                true,
                [240, 240, 240, 255],
            );
        }

        if self.bar_module("right", "battery") {
            // Battery
            let (bat_opt, is_charging) = metrics.battery;
            if let Some(bat_pct) = bat_opt {
                cur_right -= 8;
                let bat_w = 78u32;
                cur_right -= bat_w as i32;
                let bat_text = if is_charging {
                    format!("⚡ {}%", bat_pct)
                } else {
                    format!("BAT {}%", bat_pct)
                };
                crate::animation::notch::draw_rounded_rect(
                    frame,
                    cur_right,
                    pill_y,
                    bat_w,
                    pill_h,
                    pill_h / 2,
                    [255, 255, 255, 20],
                    [255, 255, 255, 32],
                );
                crate::animation::notch::draw_text(
                    frame,
                    &bat_text,
                    cur_right + 8,
                    pill_y + ((pill_h - 12) / 2) as i32,
                    bat_w - 12,
                    11,
                    false,
                    [220, 220, 220, 230],
                );
            }
        }

        if self.bar_module("right", "volume") {
            // Volume
            cur_right -= 8;
            let vol = metrics.volume;
            let vol_w = 78u32;
            cur_right -= vol_w as i32;
            let vol_text = if vol.muted {
                "MUTED".to_string()
            } else {
                format!("VOL {}%", vol.level)
            };
            let vol_bg = if vol.muted {
                [160, 40, 40, 60]
            } else {
                [255, 255, 255, 20]
            };
            crate::animation::notch::draw_rounded_rect(
                frame,
                cur_right,
                pill_y,
                vol_w,
                pill_h,
                pill_h / 2,
                vol_bg,
                [255, 255, 255, 32],
            );
            crate::animation::notch::draw_text(
                frame,
                &vol_text,
                cur_right + 8,
                pill_y + ((pill_h - 12) / 2) as i32,
                vol_w - 12,
                11,
                false,
                [220, 220, 220, 230],
            );
            hits.push((
                crate::bar::HIT_BAR_VOLUME_TOGGLE,
                cur_right,
                pill_y,
                vol_w,
                pill_h,
            ));
        }

        if self.bar_module("right", "memory") {
            // RAM
            cur_right -= 8;
            let mem_pct = metrics.memory_pct;
            let mem_w = 78u32;
            cur_right -= mem_w as i32;
            let mem_text = format!("RAM {}%", mem_pct);
            crate::animation::notch::draw_rounded_rect(
                frame,
                cur_right,
                pill_y,
                mem_w,
                pill_h,
                pill_h / 2,
                [255, 255, 255, 20],
                [255, 255, 255, 32],
            );
            crate::animation::notch::draw_text(
                frame,
                &mem_text,
                cur_right + 8,
                pill_y + ((pill_h - 12) / 2) as i32,
                mem_w - 12,
                11,
                false,
                [220, 220, 220, 230],
            );
        }

        if self.bar_module("right", "cpu") {
            // CPU
            cur_right -= 8;
            let cpu_pct = metrics.cpu_pct;
            let cpu_w = 78u32;
            cur_right -= cpu_w as i32;
            let cpu_text = format!("CPU {}%", cpu_pct);
            crate::animation::notch::draw_rounded_rect(
                frame,
                cur_right,
                pill_y,
                cpu_w,
                pill_h,
                pill_h / 2,
                [255, 255, 255, 20],
                [255, 255, 255, 32],
            );
            crate::animation::notch::draw_text(
                frame,
                &cpu_text,
                cur_right + 8,
                pill_y + ((pill_h - 12) / 2) as i32,
                cpu_w - 12,
                11,
                false,
                [220, 220, 220, 230],
            );
        }

        hits
    }

    /// Bar zone 4 (center): the resting island pill, or the expanded card
    /// composited below the bar. Owns [`Controller::icon_hits`]: it installs
    /// the passed-in module hits alongside the island's own.
    fn paint_bar_center(
        &mut self,
        frame: &mut FrameBuffer,
        state: VisualState,
        now_ms: u64,
        width: u32,
        card: BarCard,
        mut hits: Vec<BarHit>,
    ) {
        // 4. Center Module: Dynamic Island
        if !card.expanded {
            if self.bar_module("center", "island") {
                // Resting Dynamic Island pill flush in the center of the bar.
                // Shared with the hover sensor via bar_pill_rect: same rect here,
                // in the hit target below, and in set_hover.
                let (pill_cx, pill_y, pill_w, pill_h) = self.bar_pill_rect(width);

                crate::animation::notch::draw_rounded_rect(
                    frame,
                    pill_cx,
                    pill_y,
                    pill_w,
                    pill_h,
                    pill_h / 2,
                    [10, 10, 10, 190],
                    [255, 255, 255, 45],
                );

                // Mini face on the left of the pill
                let face_sz = (pill_h - 4).min(22);
                let face_x = pill_cx + 4;
                let face_y = pill_y + ((pill_h - face_sz) / 2) as i32;
                crate::animation::notch::blit_rounded(
                    frame,
                    &self.face_frame,
                    face_x,
                    face_y,
                    face_sz,
                    face_sz,
                    face_sz / 2,
                );

                // Status label or media wave inside the pill
                let label_x = face_x + face_sz as i32 + 6;
                let label_max_w = pill_w.saturating_sub((label_x - pill_cx) as u32 + 6);
                if self.media_playing() && self.island.has_widget("music") {
                    if let Some(media) = &self.media {
                        let track = format!("{} — {}", media.title, media.artist);
                        crate::animation::notch::draw_text(
                            frame,
                            &track,
                            label_x,
                            pill_y + ((pill_h - 12) / 2) as i32,
                            label_max_w,
                            11,
                            false,
                            [220, 220, 220, 240],
                        );
                    } else {
                        crate::animation::notch::draw_text(
                            frame,
                            "Now Playing",
                            label_x,
                            pill_y + ((pill_h - 12) / 2) as i32,
                            label_max_w,
                            11,
                            false,
                            [220, 220, 220, 240],
                        );
                    }
                } else if state != VisualState::Idle {
                    let (sc, _) = crate::animation::notch::accent_colors(state);
                    let dot_r = 3u32;
                    crate::animation::notch::draw_disc(
                        frame,
                        label_x + 4,
                        pill_y + (pill_h / 2) as i32,
                        dot_r,
                        sc,
                    );
                    let state_name = format!("{:?}", state);
                    crate::animation::notch::draw_text(
                        frame,
                        &state_name,
                        label_x + 12,
                        pill_y + ((pill_h - 12) / 2) as i32,
                        label_max_w.saturating_sub(14),
                        11,
                        false,
                        [220, 220, 220, 240],
                    );
                } else {
                    crate::animation::notch::draw_text(
                        frame,
                        "Termielle",
                        label_x,
                        pill_y + ((pill_h - 12) / 2) as i32,
                        label_max_w,
                        11,
                        true,
                        [220, 220, 220, 220],
                    );
                }

                // Register hit target for clicking the Dynamic Island pill
                hits.push((
                    crate::bar::HIT_BAR_ISLAND_PILL,
                    pill_cx,
                    pill_y,
                    pill_w,
                    pill_h,
                ));
            }
            self.icon_hits = hits;
        } else {
            // Expanded Dynamic Island card dropping organically below the bar!
            // Island content only renders when the center module is listed;
            // the glass card and module hits below stay unconditional.
            let mut content = self.blank_frame(card.island_w, card.exp_h);
            if self.bar_module("center", "island") {
                let sub_blobs = [crate::animation::notch::BlobRect {
                    x: 0,
                    y: 0,
                    w: card.island_w,
                    h: card.exp_h,
                    r: 18,
                    attached: true,
                }];

                let island_cfg = self.island.clone();
                self.render_content(
                    &mut content,
                    state,
                    &island_cfg,
                    crate::animation::notch::Presentation::Expanded,
                    card.island_w,
                    card.exp_h,
                    &sub_blobs,
                    now_ms,
                );
            }

            // Offset hit targets recorded in sub-frame by (island_x, island_y)
            for hit in &mut self.icon_hits {
                hit.1 += card.island_x;
                hit.2 += card.island_y;
            }
            self.icon_hits.extend(hits);

            if self.bar_module("center", "island") {
                let age_ms = now_ms.saturating_sub(self.state_since_ms);
                let (alpha, dx, dy) = Self::content_motion(
                    self.spring.as_ref(),
                    crate::animation::notch::Presentation::Expanded,
                    state,
                    age_ms,
                );
                crate::animation::notch::blend_frame_over(
                    frame,
                    &content,
                    card.island_x + dx,
                    card.island_y + dy,
                    alpha,
                );
            }
        }
    }
    /// Content motion during a morph: a slight opacity dip while the
    /// container is at its fastest (mid-spring) and a small rise into
    /// place when the target is the expanded card. Settled frames get
    /// full opacity.
    fn content_motion(
        spring: Option<&Spring2D>,
        presentation: crate::animation::notch::Presentation,
        state: VisualState,
        state_age_ms: u64,
    ) -> (u8, i32, i32) {
        use crate::animation::notch::Presentation;
        let dx = Self::shake_dx(state, state_age_ms);
        let Some(spring) = spring else {
            return (255, dx, 0);
        };
        let p = spring.progress();
        let dip = (1.0 - 0.45 * (p * std::f32::consts::PI).sin()).clamp(0.0, 1.0);
        let alpha = (255.0 * dip).round() as u8;
        let dy = match presentation {
            Presentation::Expanded => ((1.0 - p) * 6.0).round() as i32,
            _ => 0,
        };
        (alpha, dx, dy)
    }

    /// Damped horizontal shake for failed turns: ±4 px decaying over 300 ms,
    /// then exactly zero so settled frames stay pixel-stable.
    fn shake_dx(state: VisualState, age_ms: u64) -> i32 {
        if state != VisualState::Failed || age_ms >= 300 {
            return 0;
        }
        let age = age_ms as f32;
        (4.0 * (-age / 90.0).exp() * (age * 0.22).sin()).round() as i32
    }

    /// Vertical bounce for thinking dots: staggered sine, ±2 px. Zero for
    /// every other state so settled frames stay pixel-stable.
    fn think_bob(state: VisualState, index: usize, now_ms: u64) -> i32 {
        if state != VisualState::Thinking {
            return 0;
        }
        (2.0 * ((now_ms as f32 / 240.0) + index as f32 * 2.1).sin()).round() as i32
    }

    /// Worker orbit position: dots circle the face while tools run.
    /// Positions only; the caller gates on Working and paints.
    fn orbit_dot(cx: i32, cy: i32, radius: i32, index: u32, now_ms: u64) -> (i32, i32) {
        let a = now_ms as f32 / 600.0 * std::f32::consts::TAU + index as f32 * 2.094;
        (
            cx + (radius as f32 * a.cos()).round() as i32,
            cy + (radius as f32 * a.sin()).round() as i32,
        )
    }

    /// Celebration sparkle after a turn completes: one of 8 dots flying out
    /// from (`cx`, `cy`) over 600 ms with fading alpha, then `None` forever.
    /// Deterministic in age: no particle state to keep.
    fn sparkle_dot(cx: i32, cy: i32, index: u32, age_ms: u64) -> Option<(i32, i32, u8)> {
        if age_ms >= 600 {
            return None;
        }
        let t = age_ms as f32 / 600.0;
        let a = index as f32 * std::f32::consts::TAU / 8.0;
        let r = 6.0 + 20.0 * t;
        Some((
            cx + (r * a.cos()).round() as i32,
            cy + (r * a.sin()).round() as i32,
            ((1.0 - t) * 255.0).round() as u8,
        ))
    }

    /// Draws the presentation content (alert card, face, session dots,
    /// media, dashboard) onto `frame`. In the split compact presentation
    /// the trailing media content is positioned inside the detached media
    /// blob instead of inline.
    #[allow(clippy::too_many_arguments)]
    fn render_content(
        &mut self,
        frame: &mut FrameBuffer,
        state: VisualState,
        island: &IslandConfig,
        presentation: crate::animation::notch::Presentation,
        width: u32,
        height: u32,
        blobs: &[crate::animation::notch::BlobRect],
        now_ms: u64,
    ) {
        use crate::animation::notch::Presentation;

        // Render according to the active iOS/macOS presentation class.
        // Each arm is a focused painter below; shared setup (glass, frame
        // size) stays with the callers.
        self.icon_hits.clear();
        if self.paint_alert_banner(frame, island, width, height, now_ms) {
            return;
        }
        match presentation {
            Presentation::Hidden => {}
            Presentation::Minimal => {
                self.paint_minimal_content(frame, island, width, height, now_ms)
            }
            Presentation::Compact => {
                self.paint_compact_content(frame, state, island, width, height, blobs, now_ms);
            }
            Presentation::Expanded => {
                self.paint_expanded_content(frame, state, island, width, height, now_ms);
            }
        }
    }
    /// Content painter 0: the active notification alert banner. Returns
    /// true when a banner showed (the caller returns early); the cloned
    /// alert unties the `alerts` borrow from the paint calls.
    #[allow(clippy::too_many_arguments)]
    fn paint_alert_banner(
        &mut self,
        frame: &mut FrameBuffer,
        island: &IslandConfig,
        width: u32,
        height: u32,
        now_ms: u64,
    ) -> bool {
        let cy = (height / 2) as i32;
        let now = now_ms;
        // 0. Active Notification Alert Banner
        if let Some(alert) = self.alerts.front() {
            let pad = 18i32;

            if height >= 85 {
                // Tall card layout: drops vertically downward
                if island.has_widget("face") {
                    let face_size = 22i32;
                    let fx = pad;
                    let fy = 14;
                    crate::animation::notch::draw_disc(
                        frame,
                        fx + face_size / 2,
                        fy + face_size / 2,
                        (face_size / 2 + 2) as u32,
                        [alert.accent[0], alert.accent[1], alert.accent[2], 65],
                    );
                    crate::animation::notch::blit_rounded(
                        frame,
                        &self.face_frame,
                        fx,
                        fy,
                        face_size as u32,
                        face_size as u32,
                        6,
                    );
                }

                let tag_x = if island.has_widget("face") {
                    pad + 30
                } else {
                    pad
                };
                crate::animation::notch::draw_text(
                    frame,
                    "System Notification",
                    tag_x,
                    15,
                    width.saturating_sub((tag_x as u32) + 40),
                    10,
                    true,
                    [alert.accent[0], alert.accent[1], alert.accent[2], 255],
                );

                // Trailing beacon badge with soft glowing halo
                crate::animation::notch::draw_disc(
                    frame,
                    width as i32 - 24,
                    22,
                    7,
                    [alert.accent[0], alert.accent[1], alert.accent[2], 50],
                );
                crate::animation::notch::draw_disc(frame, width as i32 - 24, 22, 4, alert.accent);

                // Hairline glass separator
                crate::animation::notch::fill_rect_pub(
                    frame,
                    pad,
                    38,
                    width.saturating_sub((pad as u32) * 2),
                    1,
                    [255, 255, 255, 22],
                );

                // Main headline and detail text
                let text_w = width.saturating_sub((pad as u32) * 2);
                crate::animation::notch::draw_text(
                    frame,
                    &alert.title,
                    pad,
                    48,
                    text_w,
                    15,
                    true,
                    self.ink(),
                );
                crate::animation::notch::draw_text(
                    frame,
                    &alert.subtitle,
                    pad,
                    76,
                    text_w,
                    12,
                    false,
                    self.ink_dim(),
                );

                // Timeout hairline: the banner's remaining life, so the
                // auto-dismiss reads as intentional rather than a flicker.
                let frac = if alert.duration_ms == 0 {
                    0.0
                } else {
                    alert.expires_at_ms.saturating_sub(now) as f32 / alert.duration_ms as f32
                }
                .clamp(0.0, 1.0);
                let hair_w =
                    ((width.saturating_sub((pad as u32) * 2)) as f32 * frac).round() as u32;
                if hair_w > 0 {
                    crate::animation::notch::fill_rect_pub(
                        frame,
                        pad,
                        height as i32 - 4,
                        hair_w,
                        2,
                        [alert.accent[0], alert.accent[1], alert.accent[2], 200],
                    );
                }

                // Bottom accent pill bar
                if height >= 105 {
                    let bar_w = 44u32;
                    let bar_x = (width as i32 - bar_w as i32) / 2;
                    crate::animation::notch::fill_rect_pub(
                        frame,
                        bar_x,
                        height as i32 - 10,
                        bar_w,
                        3,
                        alert.accent,
                    );
                }
            } else {
                let face_size = (height.saturating_sub(18)).min(36) as i32;
                let fx = pad;
                let fy = cy - face_size / 2;

                if island.has_widget("face") {
                    crate::animation::notch::draw_disc(
                        frame,
                        fx + face_size / 2,
                        cy,
                        (face_size / 2 + 3) as u32,
                        [alert.accent[0], alert.accent[1], alert.accent[2], 65],
                    );
                    crate::animation::notch::blit_rounded(
                        frame,
                        &self.face_frame,
                        fx,
                        fy,
                        face_size as u32,
                        face_size as u32,
                        8,
                    );
                }

                let text_x = if island.has_widget("face") {
                    fx + face_size + 14
                } else {
                    pad
                };
                let text_w = width.saturating_sub((text_x as u32) + 36);

                crate::animation::notch::draw_text(
                    frame,
                    &alert.title,
                    text_x,
                    cy - 7,
                    text_w,
                    12,
                    true,
                    [255, 255, 255, 245],
                );

                // Trailing beacon badge
                crate::animation::notch::draw_disc(frame, width as i32 - 20, cy, 5, alert.accent);
            }

            return true;
        }
        false
    }

    /// Content painter: the minimal dot/face presentation.
    fn paint_minimal_content(
        &mut self,
        frame: &mut FrameBuffer,
        island: &IslandConfig,
        width: u32,
        height: u32,
        now_ms: u64,
    ) {
        let cy = (height / 2) as i32;
        let accent = crate::system::accent_color_bgra();
        let now = now_ms;
        // A paused equalizer holds its pose instead of performing playback.
        let eq_now = if self.media_playing() { now } else { 0 };
        if self.media_playing() && island.has_widget("music") {
            let art_size = (height.saturating_sub(12)).min(22) as i32;
            let art_x = (width as i32 / 2) - 22;
            if let Some(thumb) = self.media.as_ref().and_then(|m| m.thumbnail.as_ref()) {
                crate::animation::notch::blit_rounded(
                    frame,
                    &crate::animation::FrameBuffer {
                        width: thumb.width,
                        height: thumb.height,
                        pixels_pbgra: thumb.pixels_pbgra.clone(),
                        delay_ms: 0,
                        loop_index: 0,
                        scale: 1.0,
                    },
                    art_x,
                    cy - art_size / 2,
                    art_size as u32,
                    art_size as u32,
                    6,
                );
            } else {
                crate::animation::notch::draw_disc(
                    frame,
                    art_x + art_size / 2,
                    cy,
                    (art_size / 2) as u32,
                    [accent[0], accent[1], accent[2], 255],
                );
            }
            // Mini equalizer bars
            let eq_x = (width as i32 / 2) + 6;
            for (i, &phase) in [0u64, 180, 360].iter().enumerate() {
                let t =
                    ((eq_now.saturating_add(phase) % 800) as f32 / 800.0) * std::f32::consts::TAU;
                let h = (3.0 + 5.0 * (t + i as f32).sin().abs()).round() as i32;
                crate::animation::notch::fill_rect_pub(
                    frame,
                    eq_x + i as i32 * 4,
                    cy + 4 - h,
                    2,
                    h as u32,
                    accent,
                );
            }
            self.icon_hits.push((0, art_x - 2, cy - 10, 44, 20));
        } else if island.has_widget("face") {
            let face_size = (height.saturating_sub(8)).min(32) as i32;
            crate::animation::notch::blit_scaled(
                frame,
                &self.face_frame,
                (width as i32 - face_size) / 2,
                cy - face_size / 2,
                face_size as u32,
                face_size as u32,
            );
        }
    }

    /// Content painter: the compact live pill (face, status, media live
    /// activity, session dots), including the two-blob split layout.
    #[allow(clippy::too_many_arguments)]
    fn paint_compact_content(
        &mut self,
        frame: &mut FrameBuffer,
        state: VisualState,
        island: &IslandConfig,
        width: u32,
        height: u32,
        blobs: &[crate::animation::notch::BlobRect],
        now_ms: u64,
    ) {
        let cy = (height / 2) as i32;
        let accent = crate::system::accent_color_bgra();
        let now = now_ms;
        // A paused equalizer holds its pose instead of performing playback.
        let eq_now = if self.media_playing() { now } else { 0 };
        if blobs.len() > 1 {
            // The Dynamic Island split: agent content leads in the
            // primary blob, media lives in the detached blob, and
            // the liquid bridge between them is drawn by the
            // material. Clicks on the media blob toggle playback.
            let (primary, media_blob) = (blobs[0], blobs[1]);
            if island.has_widget("face") {
                let face_size = (height.saturating_sub(8)).min(28) as i32;
                let face_x = primary.x + 12;
                if state != VisualState::Idle {
                    let (sc, _) = crate::animation::notch::accent_colors(state);
                    crate::animation::notch::draw_disc(
                        frame,
                        face_x + face_size / 2,
                        cy,
                        (face_size / 2 + 2) as u32,
                        [sc[0], sc[1], sc[2], 50],
                    );
                }
                crate::animation::notch::blit_scaled(
                    frame,
                    &self.face_frame,
                    face_x,
                    cy - face_size / 2,
                    face_size as u32,
                    face_size as u32,
                );
                if state == VisualState::Working {
                    // Worker orbit: three dots circle the face while
                    // tools run.
                    let (wc, _) = crate::animation::notch::accent_colors(state);
                    let ocx = face_x + face_size / 2;
                    for i in 0..3u32 {
                        let (odx, ody) = Self::orbit_dot(ocx, cy, face_size / 2 + 7, i, now);
                        crate::animation::notch::draw_disc(frame, odx, ody, 2, wc);
                    }
                }
            }
            if state != VisualState::Idle || self.reducer.session_count() > 0 {
                let (dot, _) = crate::animation::notch::accent_colors(state);
                let count = if island.has_widget("agents") {
                    self.reducer.session_count().clamp(1, 4)
                } else {
                    1
                };
                let mut dot_x = primary.x + primary.w as i32 - 16;
                for i in 0..count {
                    let dy = Self::think_bob(state, i, now);
                    crate::animation::notch::draw_disc(frame, dot_x, cy + dy, 3, dot);
                    dot_x -= 10;
                }
            }
            // Media blob: album art plus the equalizer bars.
            let art_size = (height.saturating_sub(12)).min(22) as i32;
            let art_x = media_blob.x + 10;
            if let Some(thumb) = self.media.as_ref().and_then(|m| m.thumbnail.as_ref()) {
                crate::animation::notch::blit_rounded(
                    frame,
                    &crate::animation::FrameBuffer {
                        width: thumb.width,
                        height: thumb.height,
                        pixels_pbgra: thumb.pixels_pbgra.clone(),
                        delay_ms: 0,
                        loop_index: 0,
                        scale: 1.0,
                    },
                    art_x,
                    cy - art_size / 2,
                    art_size as u32,
                    art_size as u32,
                    6,
                );
            } else {
                crate::animation::notch::draw_disc(
                    frame,
                    art_x + art_size / 2,
                    cy,
                    (art_size / 2) as u32,
                    accent,
                );
            }
            let eq_x = art_x + art_size + 6;
            for (i, &phase) in [0u64, 180, 360].iter().enumerate() {
                let t =
                    ((eq_now.saturating_add(phase) % 800) as f32 / 800.0) * std::f32::consts::TAU;
                let h = (3.0 + 8.0 * (t + i as f32).sin().abs()).round() as i32;
                crate::animation::notch::fill_rect_pub(
                    frame,
                    eq_x + i as i32 * 4,
                    cy + 4 - h,
                    2,
                    h as u32,
                    accent,
                );
            }
            self.icon_hits
                .push((HIT_MEDIA_PLAY_PAUSE, media_blob.x, 0, media_blob.w, height));
        } else {
            // Split Dynamic Island: Leading face, Trailing activity
            if island.has_widget("face") {
                let face_size = (height.saturating_sub(8)).min(28) as i32;
                let face_x = 12;
                if state != VisualState::Idle {
                    let (sc, _) = crate::animation::notch::accent_colors(state);
                    crate::animation::notch::draw_disc(
                        frame,
                        face_x + face_size / 2,
                        cy,
                        (face_size / 2 + 2) as u32,
                        [sc[0], sc[1], sc[2], 50],
                    );
                }
                crate::animation::notch::blit_scaled(
                    frame,
                    &self.face_frame,
                    face_x,
                    cy - face_size / 2,
                    face_size as u32,
                    face_size as u32,
                );
                if state == VisualState::Working {
                    let (wc, _) = crate::animation::notch::accent_colors(state);
                    let ocx = face_x + face_size / 2;
                    for i in 0..3u32 {
                        let (odx, ody) = Self::orbit_dot(ocx, cy, face_size / 2 + 7, i, now);
                        crate::animation::notch::draw_disc(frame, odx, ody, 2, wc);
                    }
                }
            }

            let mut right_cursor = width as i32 - 14;

            // Media equalizer, with the album art leading it
            if self.media_playing() && island.has_widget("music") {
                let bx = right_cursor - 14;
                let art_size = (height.saturating_sub(12)).min(22) as i32;
                let art_x = bx - art_size - 6;
                if let Some(thumb) = self.media.as_ref().and_then(|m| m.thumbnail.as_ref()) {
                    crate::animation::notch::blit_rounded(
                        frame,
                        &crate::animation::FrameBuffer {
                            width: thumb.width,
                            height: thumb.height,
                            pixels_pbgra: thumb.pixels_pbgra.clone(),
                            delay_ms: 0,
                            loop_index: 0,
                            scale: 1.0,
                        },
                        art_x,
                        cy - art_size / 2,
                        art_size as u32,
                        art_size as u32,
                        6,
                    );
                } else {
                    crate::animation::notch::draw_disc(
                        frame,
                        art_x + art_size / 2,
                        cy,
                        (art_size / 2) as u32,
                        [accent[0], accent[1], accent[2], 230],
                    );
                }
                let base = cy + 7;
                for (i, &phase) in [0u64, 180, 360].iter().enumerate() {
                    let t = ((eq_now.saturating_add(phase) % 800) as f32 / 800.0)
                        * std::f32::consts::TAU;
                    let h = (3.0 + 8.0 * (t + i as f32).sin().abs()).round() as i32;
                    crate::animation::notch::fill_rect_pub(
                        frame,
                        bx + i as i32 * 5,
                        base - h,
                        3,
                        h as u32,
                        accent,
                    );
                }
                self.icon_hits.push((0, art_x - 2, cy - 10, 44, 20));
                right_cursor -= 22;
            }

            // Agent status beacon and dots
            if state != VisualState::Idle || self.reducer.session_count() > 0 {
                let (dot, _) = crate::animation::notch::accent_colors(state);
                let count = if island.has_widget("agents") {
                    self.reducer.session_count().clamp(1, 4)
                } else {
                    1
                };
                for i in 0..count {
                    right_cursor -= 10;
                    let dy = Self::think_bob(state, i, now);
                    crate::animation::notch::draw_disc(
                        frame,
                        right_cursor + 4,
                        cy + dy,
                        3,
                        [dot[0], dot[1], dot[2], dot[3]],
                    );
                }
            } else if state == VisualState::Idle && !self.media_playing() {
                let text_x = if island.has_widget("face") { 46 } else { 16 };
                let text_w = width.saturating_sub((text_x as u32) + 18);
                crate::animation::notch::draw_text(
                    frame,
                    "Termielle",
                    text_x,
                    cy - 6,
                    text_w,
                    11,
                    true,
                    [220, 225, 235, 210],
                );
                crate::animation::notch::draw_disc(
                    frame,
                    width as i32 - 16,
                    cy,
                    3,
                    [accent[0], accent[1], accent[2], 180],
                );
            }
        }
    }
    /// Content painter: the expanded dashboard card (media body, agent
    /// live activity, task switcher, standby telemetry).
    #[allow(clippy::too_many_arguments)]
    fn paint_expanded_content(
        &mut self,
        frame: &mut FrameBuffer,
        state: VisualState,
        island: &IslandConfig,
        width: u32,
        height: u32,
        now_ms: u64,
    ) {
        let cy = (height / 2) as i32;
        let accent = crate::system::accent_color_bgra();
        let now = now_ms;
        // A paused equalizer holds its pose instead of performing playback.
        let eq_now = if self.media_playing() { now } else { 0 };
        let pad = 18i32;

        let ctx = CardPaintCtx {
            width,
            height,
            cy,
            accent,
            now,
            eq_now,
            pad,
        };

        if height >= 85 {
            // Authentic tall card layout dropping vertically downward.
            self.paint_card_header(frame, state, island, &ctx);
            if self.media_playing() && island.has_widget("music") {
                self.paint_media_card(frame, island, &ctx);
            } else if state != VisualState::Idle {
                self.paint_agent_activity(frame, state, island, &ctx);
            } else if island.has_widget("tasks") && island.show_tasks && !self.tasks.is_empty() {
                self.paint_task_switcher(frame, island, &ctx);
            } else {
                self.paint_standby_dashboard(frame, island, &ctx);
            }
        } else {
            // Compact / transitioning view.
            self.paint_short_card(frame, state, island, &ctx);
        }
    }
    /// Expanded card header row: face glyph plus the hairline divider.
    fn paint_card_header(
        &self,
        frame: &mut FrameBuffer,
        state: VisualState,
        island: &IslandConfig,
        ctx: &CardPaintCtx,
    ) {
        // Authentic tall card layout dropping vertically downward
        // 1. Top Header Row
        if island.has_widget("face") {
            let face_size = 22i32;
            let fx = ctx.pad;
            let fy = 14;
            let (sc, _) = crate::animation::notch::accent_colors(state);
            crate::animation::notch::draw_disc(
                frame,
                fx + face_size / 2,
                fy + face_size / 2,
                (face_size / 2 + 2) as u32,
                [sc[0], sc[1], sc[2], 65],
            );
            crate::animation::notch::blit_rounded(
                frame,
                &self.face_frame,
                fx,
                fy,
                face_size as u32,
                face_size as u32,
                6,
            );
        }

        // Hairline glass divider
        crate::animation::notch::fill_rect_pub(
            frame,
            ctx.pad,
            40,
            ctx.width.saturating_sub((ctx.pad as u32) * 2),
            1,
            [255, 255, 255, 22],
        );
    }

    /// Expanded card section: prominent media body (artwork, track,
    /// progress, transport controls).
    fn paint_media_card(
        &mut self,
        frame: &mut FrameBuffer,
        island: &IslandConfig,
        ctx: &CardPaintCtx,
    ) {
        // Header title & mini equalizer in header row
        let tag_x = if island.has_widget("face") {
            ctx.pad + 30
        } else {
            ctx.pad
        };
        crate::animation::notch::draw_text(
            frame,
            "Now Playing",
            tag_x,
            16,
            ctx.width.saturating_sub((tag_x as u32) + 50),
            10,
            true,
            ctx.accent,
        );

        // 4-bar mini equalizer in top right
        let eq_x = ctx.width as i32 - ctx.pad - 20;
        for (i, &phase) in [0u64, 180, 360, 540].iter().enumerate() {
            let t =
                ((ctx.eq_now.saturating_add(phase) % 700) as f32 / 700.0) * std::f32::consts::TAU;
            let h = (3.0 + 8.0 * (t + i as f32).sin().abs()).round() as i32;
            crate::animation::notch::fill_rect_pub(
                frame,
                eq_x + i as i32 * 5,
                26 - h,
                3,
                h as u32,
                ctx.accent,
            );
        }

        // Prominent Media Body Card:
        // High-res 64x64 Album Artwork
        let art_size = 64i32;
        let art_x = ctx.pad;
        let art_y = 48i32;

        if let Some(thumb) = self.media.as_ref().and_then(|m| m.thumbnail.as_ref()) {
            crate::animation::notch::blit_rounded(
                frame,
                &crate::animation::FrameBuffer {
                    width: thumb.width,
                    height: thumb.height,
                    pixels_pbgra: thumb.pixels_pbgra.clone(),
                    delay_ms: 0,
                    loop_index: 0,
                    scale: 1.0,
                },
                art_x,
                art_y,
                art_size as u32,
                art_size as u32,
                12,
            );
        } else {
            crate::animation::notch::draw_disc(
                frame,
                art_x + art_size / 2,
                art_y + art_size / 2,
                (art_size / 2) as u32,
                [ctx.accent[0], ctx.accent[1], ctx.accent[2], 220],
            );
        }

        // Track title, artist, and app
        let text_x = art_x + art_size + 14;
        let text_w = ctx.width.saturating_sub((text_x as u32) + (ctx.pad as u32));
        let title = self
            .media
            .as_ref()
            .map(|m| m.title.as_str())
            .unwrap_or("Playing");
        let artist = self
            .media
            .as_ref()
            .map(|m| m.artist.as_str())
            .unwrap_or("Media");

        crate::animation::notch::draw_text(frame, title, text_x, 50, text_w, 14, true, self.ink());
        crate::animation::notch::draw_text(
            frame,
            artist,
            text_x,
            74,
            text_w,
            11,
            false,
            self.ink_dim(),
        );

        let app_name = self.media.as_ref().map(|m| m.app.as_str()).unwrap_or("");
        if !app_name.is_empty() {
            crate::animation::notch::draw_text(
                frame,
                app_name,
                text_x,
                94,
                text_w,
                10,
                false,
                [ctx.accent[0], ctx.accent[1], ctx.accent[2], 210],
            );
        }

        // Progress / timeline bar
        if ctx.height >= 145 {
            let track_y = 120i32;
            let track_w = ctx.width.saturating_sub((ctx.pad as u32) * 2);
            crate::animation::notch::fill_rect_pub(
                frame,
                ctx.pad,
                track_y,
                track_w,
                3,
                [255, 255, 255, 30],
            );
            crate::animation::notch::fill_rect_pub(
                frame,
                ctx.pad,
                track_y,
                (track_w as f32 * 0.45) as u32,
                3,
                ctx.accent,
            );
        }

        // Dedicated Media Controls Row: Prev (⏮), Play/Pause (⏯), Next (⏭)
        if ctx.height >= 165 {
            let ctrl_y = 148i32;
            let center_x = ctx.width as i32 / 2;

            // Previous Track Button
            let prev_x = center_x - 56;
            let prev_hover = self
                .hover_point
                .is_some_and(|(px, py)| (px - prev_x).pow(2) + (py - ctrl_y).pow(2) <= 16 * 16);
            let btn_bg = if prev_hover {
                [255, 255, 255, 45]
            } else {
                [255, 255, 255, 25]
            };
            crate::animation::notch::draw_button_circle(
                frame,
                prev_x,
                ctrl_y,
                14,
                btn_bg,
                [255, 255, 255, 60],
            );
            crate::animation::notch::draw_glyph_prev(frame, prev_x, ctrl_y, [255, 255, 255, 240]);
            self.icon_hits
                .push((HIT_MEDIA_PREV, prev_x - 16, ctrl_y - 16, 32, 32));

            // Play / Pause Button
            let play_hover = self
                .hover_point
                .is_some_and(|(px, py)| (px - center_x).pow(2) + (py - ctrl_y).pow(2) <= 19 * 19);
            let play_bg = if play_hover {
                [ctx.accent[0], ctx.accent[1], ctx.accent[2], 255]
            } else {
                [ctx.accent[0], ctx.accent[1], ctx.accent[2], 210]
            };
            crate::animation::notch::draw_button_circle(
                frame,
                center_x,
                ctrl_y,
                17,
                play_bg,
                [255, 255, 255, 100],
            );
            if self.media.as_ref().is_some_and(|m| m.playing) {
                crate::animation::notch::draw_glyph_pause(frame, center_x, ctrl_y, self.ink());
            } else {
                crate::animation::notch::draw_glyph_play(frame, center_x, ctrl_y, self.ink());
            }
            self.icon_hits
                .push((HIT_MEDIA_PLAY_PAUSE, center_x - 18, ctrl_y - 18, 36, 36));

            // Next Track Button
            let next_x = center_x + 56;
            let next_hover = self
                .hover_point
                .is_some_and(|(px, py)| (px - next_x).pow(2) + (py - ctrl_y).pow(2) <= 16 * 16);
            let btn_bg = if next_hover {
                [255, 255, 255, 45]
            } else {
                [255, 255, 255, 25]
            };
            crate::animation::notch::draw_button_circle(
                frame,
                next_x,
                ctrl_y,
                14,
                btn_bg,
                [255, 255, 255, 60],
            );
            crate::animation::notch::draw_glyph_next(frame, next_x, ctrl_y, [255, 255, 255, 240]);
            self.icon_hits
                .push((HIT_MEDIA_NEXT, next_x - 16, ctrl_y - 16, 32, 32));
        }
    }
    /// Expanded card section: agent live activity (sessions, status,
    /// beacon, sparkles, session badge).
    fn paint_agent_activity(
        &mut self,
        frame: &mut FrameBuffer,
        state: VisualState,
        island: &IslandConfig,
        ctx: &CardPaintCtx,
    ) {
        // Agent Live Activity (e.g. Claude, Codex, Agy, OpenCode is running)
        let primary = self.reducer.primary_session();
        let agent_source = primary
            .as_ref()
            .map(|(s, _, _)| s.as_str())
            .unwrap_or("agent");
        let session_id = primary.as_ref().map(|(_, id, _)| id.as_str()).unwrap_or("");
        let (sc, _) = crate::animation::notch::accent_colors(state);

        let tag_x = if island.has_widget("face") {
            ctx.pad + 30
        } else {
            ctx.pad
        };
        let header_title = format!("Live Activity • {}", agent_source);
        crate::animation::notch::draw_text(
            frame,
            &header_title,
            tag_x,
            16,
            ctx.width.saturating_sub((tag_x as u32) + 50),
            10,
            true,
            [sc[0], sc[1], sc[2], 255],
        );

        // Session count dots top right
        let sessions = self.reducer.session_count().clamp(1, 4);
        for i in 0..sessions {
            crate::animation::notch::draw_disc(
                frame,
                ctx.width as i32 - ctx.pad - (i as i32 * 10) - 4,
                24,
                3,
                [sc[0], sc[1], sc[2], sc[3]],
            );
        }

        // Main status body
        let (state_title, state_detail) = match state {
            VisualState::Idle => ("Termielle", "Ready for instructions"),
            VisualState::Thinking => (
                "Reasoning & Planning",
                "Analyzing context and constructing plan...",
            ),
            VisualState::Working => ("Executing Actions", "Running autonomous tools and edits..."),
            VisualState::NeedsInput => ("Action Required", "Waiting for confirmation or input"),
            VisualState::Ready => ("Turn Complete", "Task finished successfully!"),
            VisualState::Failed => ("Turn Failed", "Execution stopped with error"),
        };

        // Glowing state beacon disc
        let beacon_x = ctx.pad + 16;
        let beacon_y = 72;
        crate::animation::notch::draw_disc(
            frame,
            beacon_x,
            beacon_y,
            14,
            [sc[0], sc[1], sc[2], 45],
        );
        crate::animation::notch::draw_disc(
            frame,
            beacon_x,
            beacon_y,
            8,
            [sc[0], sc[1], sc[2], 255],
        );
        // Celebration sparkles after a turn completes; silent
        // once the 600 ms flight ends.
        if state == VisualState::Ready {
            let age = ctx.now.saturating_sub(self.state_since_ms);
            for i in 0..8u32 {
                if let Some((sx, sy, sa)) = Self::sparkle_dot(beacon_x, beacon_y, i, age) {
                    if sa > 0 {
                        crate::animation::notch::draw_disc(
                            frame,
                            sx,
                            sy,
                            1,
                            [sc[0], sc[1], sc[2], sa],
                        );
                    }
                }
            }
        }

        let text_x = beacon_x + 24;
        let text_w = ctx.width.saturating_sub((text_x as u32) + (ctx.pad as u32));
        crate::animation::notch::draw_text(
            frame,
            state_title,
            text_x,
            52,
            text_w,
            15,
            true,
            self.ink(),
        );
        crate::animation::notch::draw_text(
            frame,
            state_detail,
            text_x,
            76,
            text_w,
            11,
            false,
            self.ink_dim(),
        );

        // Session ID badge pill
        if !session_id.is_empty() && ctx.height >= 140 {
            let badge_text = format!("Session: {}", &session_id[..session_id.len().min(26)]);
            let bw = (badge_text.len() * 6 + 18) as u32;
            crate::animation::notch::fill_rect_pub(frame, text_x, 98, bw, 18, [255, 255, 255, 18]);
            crate::animation::notch::draw_text(
                frame,
                &badge_text,
                text_x + 6,
                101,
                bw - 10,
                10,
                false,
                self.ink_dim(),
            );
        }

        // Bottom ctx.accent pill bar
        if ctx.height >= 145 {
            let bar_w = 64u32;
            let bar_x = (ctx.width as i32 - bar_w as i32) / 2;
            crate::animation::notch::fill_rect_pub(
                frame,
                bar_x,
                ctx.height as i32 - 12,
                bar_w,
                3,
                [sc[0], sc[1], sc[2], 255],
            );
        }
    }
    /// Expanded card section: opt-in open-windows switcher tiles.
    fn paint_task_switcher(
        &mut self,
        frame: &mut FrameBuffer,
        island: &IslandConfig,
        ctx: &CardPaintCtx,
    ) {
        // Optional / Opt-in Open Windows Switcher (when tasks widget is explicitly enabled)
        let tag_x = if island.has_widget("face") {
            ctx.pad + 30
        } else {
            ctx.pad
        };
        crate::animation::notch::draw_text(
            frame,
            "Active Tasks & Windows",
            tag_x,
            16,
            ctx.width.saturating_sub((tag_x as u32) + 50),
            10,
            true,
            ctx.accent,
        );

        let tile_w = 44i32;
        let tile_h = 44i32;
        let gap = 12i32;
        let count = self.tasks.len().min(5) as i32;
        let total_w = count * tile_w + (count - 1) * gap;
        let start_x = ((ctx.width as i32 - total_w) / 2).max(ctx.pad);
        let tile_y = 56i32;

        for (i, task) in self.tasks.iter().take(5).enumerate() {
            let tx = start_x + i as i32 * (tile_w + gap);
            let is_hover = self.hover_point.is_some_and(|(px, py)| {
                px >= tx && px < tx + tile_w && py >= tile_y && py < tile_y + tile_h
            });
            let tile_bg = if is_hover {
                [255, 255, 255, 40]
            } else {
                [255, 255, 255, 18]
            };
            let tile_border = if is_hover {
                [ctx.accent[0], ctx.accent[1], ctx.accent[2], 220]
            } else {
                [255, 255, 255, 45]
            };

            // Tile glass background
            crate::animation::notch::fill_rect_pub(
                frame,
                tx,
                tile_y,
                tile_w as u32,
                tile_h as u32,
                tile_bg,
            );
            crate::animation::notch::fill_rect_pub(
                frame,
                tx,
                tile_y,
                tile_w as u32,
                1,
                tile_border,
            );
            crate::animation::notch::fill_rect_pub(
                frame,
                tx,
                tile_y + tile_h - 1,
                tile_w as u32,
                1,
                tile_border,
            );
            crate::animation::notch::fill_rect_pub(
                frame,
                tx,
                tile_y,
                1,
                tile_h as u32,
                tile_border,
            );
            crate::animation::notch::fill_rect_pub(
                frame,
                tx + tile_w - 1,
                tile_y,
                1,
                tile_h as u32,
                tile_border,
            );

            // Blit high-res window app icon inside tile
            crate::animation::notch::blit_rounded(
                frame,
                &crate::animation::FrameBuffer {
                    width: task.width,
                    height: task.height,
                    pixels_pbgra: task.pixels_pbgra.clone(),
                    delay_ms: 0,
                    loop_index: 0,
                    scale: 1.0,
                },
                tx + 4,
                tile_y + 4,
                (tile_w - 8) as u32,
                (tile_h - 8) as u32,
                6,
            );

            // Hit target for window activation
            self.icon_hits
                .push((task.hwnd, tx, tile_y, tile_w as u32, tile_h as u32));
        }

        // Window title tooltip / caption below tiles
        let hovered_task = self.tasks.iter().take(5).find(|t| {
            self.hover_point.is_some_and(|(px, py)| {
                self.icon_hits.iter().any(|&(h, hx, hy, hw, hh)| {
                    h == t.hwnd
                        && px >= hx
                        && px < hx + hw as i32
                        && py >= hy
                        && py < hy + hh as i32
                })
            })
        });
        let display_title = hovered_task
            .map(|t| t.title.as_str())
            .unwrap_or("Click an app to switch to it");
        crate::animation::notch::draw_text(
            frame,
            display_title,
            ctx.pad,
            116,
            ctx.width.saturating_sub((ctx.pad as u32) * 2),
            11,
            false,
            self.ink_dim(),
        );

        // Bottom ctx.accent pill bar
        if ctx.height >= 145 {
            let bar_w = 48u32;
            let bar_x = (ctx.width as i32 - bar_w as i32) / 2;
            crate::animation::notch::fill_rect_pub(
                frame,
                bar_x,
                ctx.height as i32 - 12,
                bar_w,
                3,
                [ctx.accent[0], ctx.accent[1], ctx.accent[2], 200],
            );
        }
    }
    /// Expanded card section: default standby and glanceables dashboard
    /// (time, headline, telemetry cards).
    fn paint_standby_dashboard(
        &mut self,
        frame: &mut FrameBuffer,
        island: &IslandConfig,
        ctx: &CardPaintCtx,
    ) {
        // Default Clean Standby & Glanceables Dashboard
        let tag_x = if island.has_widget("face") {
            ctx.pad + 30
        } else {
            ctx.pad
        };
        crate::animation::notch::draw_text(
            frame,
            "Standby • Ready",
            tag_x,
            16,
            ctx.width.saturating_sub((tag_x as u32) + 70),
            10,
            true,
            ctx.accent,
        );

        let stats = crate::system::collect();

        // Live time in top-right header
        crate::animation::notch::draw_text(
            frame,
            &stats.time,
            ctx.width as i32 - ctx.pad - 42,
            15,
            42,
            11,
            true,
            self.ink(),
        );

        // Trailing live beacon with halo next to time
        let beacon_x = ctx.width as i32 - ctx.pad - 52;
        crate::animation::notch::draw_disc(
            frame,
            beacon_x,
            22,
            5,
            [ctx.accent[0], ctx.accent[1], ctx.accent[2], 45],
        );
        crate::animation::notch::draw_disc(
            frame,
            beacon_x,
            22,
            2,
            [ctx.accent[0], ctx.accent[1], ctx.accent[2], 255],
        );

        // Headline & Subtitle
        let text_w = ctx.width.saturating_sub((ctx.pad as u32) * 2);
        crate::animation::notch::draw_text(
            frame,
            "Termielle is Ready",
            ctx.pad,
            42,
            text_w,
            13,
            true,
            self.ink(),
        );
        crate::animation::notch::draw_text(
            frame,
            "Standing by for agent instructions or media playback",
            ctx.pad,
            60,
            text_w,
            10,
            false,
            self.ink_dim(),
        );

        // System Telemetry Cards (CPU, RAM, Power/System)
        if ctx.height >= 125 {
            // 3 rich telemetry modules
            struct CardInfo {
                label: &'static str,
                dot_color: [u8; 4],
                value: String,
                pct: u8,
                fill_color: [u8; 4],
                caption: String,
            }

            // CPU card setup
            let cpu_heavy = stats.cpu_percent >= 75;
            let (cpu_dot, cpu_fill) = if cpu_heavy {
                ([30, 90, 245, 255], [30, 90, 245, 255])
            } else {
                (
                    [ctx.accent[0], ctx.accent[1], ctx.accent[2], 255],
                    [ctx.accent[0], ctx.accent[1], ctx.accent[2], 255],
                )
            };
            let cpu_caption = if stats.cpu_percent < 25 {
                "Calm".to_string()
            } else if stats.cpu_percent < 65 {
                "Active".to_string()
            } else {
                "High Load".to_string()
            };

            // RAM card setup
            let ram_dot = [225, 175, 20, 255]; // cyan/teal
            let ram_fill = [225, 175, 20, 255];
            let ram_caption = if stats.mem_total_gb > 0.0 {
                format!("{:.0}/{:.0} GB", stats.mem_used_gb, stats.mem_total_gb)
            } else {
                "System RAM".to_string()
            };

            // Power/Battery card setup
            let (bat_dot, bat_val, bat_fill, bat_pct, bat_caption) =
                if let Some(bat) = stats.battery {
                    let charging = stats.battery_charging;
                    let dot = if charging {
                        [60, 205, 80, 255]
                    } else if bat < 20 {
                        [20, 160, 245, 255]
                    } else {
                        [60, 205, 80, 255]
                    };
                    let val = if charging {
                        format!("{}% +", bat)
                    } else {
                        format!("{}%", bat)
                    };
                    let cap = if charging {
                        "Charging".to_string()
                    } else {
                        "On Battery".to_string()
                    };
                    (dot, val, dot, bat, cap)
                } else {
                    (
                        [60, 205, 80, 255],
                        "Online".to_string(),
                        [60, 205, 80, 255],
                        100,
                        "Desktop".to_string(),
                    )
                };

            let cards = [
                CardInfo {
                    label: "CPU",
                    dot_color: cpu_dot,
                    value: format!("{}%", stats.cpu_percent),
                    pct: stats.cpu_percent,
                    fill_color: cpu_fill,
                    caption: cpu_caption,
                },
                CardInfo {
                    label: "RAM",
                    dot_color: ram_dot,
                    value: format!("{}%", stats.mem_percent),
                    pct: stats.mem_percent,
                    fill_color: ram_fill,
                    caption: ram_caption,
                },
                CardInfo {
                    label: if stats.battery.is_some() {
                        "BATTERY"
                    } else {
                        "SYSTEM"
                    },
                    dot_color: bat_dot,
                    value: bat_val,
                    pct: bat_pct,
                    fill_color: bat_fill,
                    caption: bat_caption,
                },
            ];

            let card_count = cards.len() as i32;
            let gap = 8i32;
            let total_gap = (card_count - 1) * gap;
            let card_w = ((text_w as i32 - total_gap) / card_count).max(60);
            let card_y = 80i32;
            let card_h = 50u32;

            let card_bg = [
                (ctx.accent[0] as u32 * 20 / 255) as u8,
                (ctx.accent[1] as u32 * 20 / 255) as u8,
                (ctx.accent[2] as u32 * 20 / 255) as u8,
                26,
            ];
            let card_border = [255, 255, 255, 34];
            let track_color = [255, 255, 255, 20];

            for (i, card) in cards.iter().enumerate() {
                let cx = ctx.pad + i as i32 * (card_w + gap);

                // Smooth rounded frosted card
                crate::animation::notch::draw_rounded_rect(
                    frame,
                    cx,
                    card_y,
                    card_w as u32,
                    card_h,
                    7,
                    card_bg,
                    card_border,
                );

                // Top row: indicator dot + label + value
                crate::animation::notch::draw_disc(frame, cx + 9, card_y + 11, 3, card.dot_color);
                crate::animation::notch::draw_text(
                    frame,
                    card.label,
                    cx + 16,
                    card_y + 6,
                    (card_w - 48).max(20) as u32,
                    9,
                    true,
                    self.ink_dim(),
                );
                crate::animation::notch::draw_text(
                    frame,
                    &card.value,
                    cx + card_w - 38,
                    card_y + 6,
                    34,
                    9,
                    true,
                    self.ink(),
                );

                // Mini progress bar
                let bar_w = (card_w - 18).max(10) as u32;
                crate::animation::notch::draw_progress_bar(
                    frame,
                    cx + 9,
                    card_y + 23,
                    bar_w,
                    4,
                    card.pct,
                    track_color,
                    card.fill_color,
                );

                // Detail caption under progress bar
                crate::animation::notch::draw_text(
                    frame,
                    &card.caption,
                    cx + 9,
                    card_y + 32,
                    bar_w,
                    9,
                    false,
                    self.ink_dim(),
                );
            }
        }

        // Bottom ctx.accent pill bar
        if ctx.height >= 145 {
            let bar_w = 48u32;
            let bar_x = (ctx.width as i32 - bar_w as i32) / 2;
            crate::animation::notch::fill_rect_pub(
                frame,
                bar_x,
                ctx.height as i32 - 10,
                bar_w,
                3,
                [ctx.accent[0], ctx.accent[1], ctx.accent[2], 200],
            );
        }
    }
    /// Expanded card fallback: compact transitioning view for short cards.
    fn paint_short_card(
        &mut self,
        frame: &mut FrameBuffer,
        state: VisualState,
        island: &IslandConfig,
        ctx: &CardPaintCtx,
    ) {
        // Compact / transitioning view
        let face_size = (ctx.height.saturating_sub(20)).min(36) as i32;
        if island.has_widget("face") {
            let fx = ctx.pad;
            let fy = ctx.cy - face_size / 2;
            let (sc, _) = crate::animation::notch::accent_colors(state);
            crate::animation::notch::draw_disc(
                frame,
                fx + face_size / 2,
                ctx.cy,
                (face_size / 2 + 3) as u32,
                [sc[0], sc[1], sc[2], 55],
            );
            crate::animation::notch::blit_rounded(
                frame,
                &self.face_frame,
                fx,
                fy,
                face_size as u32,
                face_size as u32,
                8,
            );
        }

        if self.media_playing() && island.has_widget("music") {
            let art_size = 28i32;
            let media_start_x = ctx.pad + face_size + 14;
            let art_y = ctx.cy - art_size / 2;
            if let Some(thumb) = self.media.as_ref().and_then(|m| m.thumbnail.as_ref()) {
                crate::animation::notch::blit_rounded(
                    frame,
                    &crate::animation::FrameBuffer {
                        width: thumb.width,
                        height: thumb.height,
                        pixels_pbgra: thumb.pixels_pbgra.clone(),
                        delay_ms: 0,
                        loop_index: 0,
                        scale: 1.0,
                    },
                    media_start_x,
                    art_y,
                    art_size as u32,
                    art_size as u32,
                    6,
                );
            } else {
                crate::animation::notch::draw_disc(
                    frame,
                    media_start_x + art_size / 2,
                    ctx.cy,
                    (art_size / 2) as u32,
                    [ctx.accent[0], ctx.accent[1], ctx.accent[2], 230],
                );
            }
            let text_x = media_start_x + art_size + 10;
            let text_w = (ctx.width as i32 - ctx.pad - text_x).max(40) as u32;
            let title = self
                .media
                .as_ref()
                .map(|m| m.title.as_str())
                .unwrap_or("Playing");
            crate::animation::notch::draw_text(
                frame,
                title,
                text_x,
                ctx.cy - 7,
                text_w,
                11,
                true,
                [255, 255, 255, 245],
            );
        } else {
            let left_edge = ctx.pad + face_size + 14;
            let text_w = (ctx.width as i32 - ctx.pad - left_edge).max(40) as u32;
            let title = match state {
                VisualState::Idle => "Termielle",
                VisualState::Thinking => "Reasoning",
                VisualState::Working => "Working",
                VisualState::NeedsInput => "Needs Input",
                VisualState::Ready => "Turn Complete",
                VisualState::Failed => "Turn Failed",
            };
            crate::animation::notch::draw_text(
                frame,
                title,
                left_edge,
                ctx.cy - 7,
                text_w,
                12,
                true,
                [255, 255, 255, 240],
            );
        }
    }
    /// Theme/material changes clear it via [`Controller::set_island_config`].
    fn glass_layer_blobs(
        &mut self,
        w: u32,
        h: u32,
        r: u32,
        attached: bool,
        blobs: &[crate::animation::notch::BlobRect],
        black: bool,
    ) -> FrameBuffer {
        let blob_count = blobs.len() as u32;
        let right_x = blobs.get(1).map_or(0, |b| b.x);
        let right_w = blobs.get(1).map_or(0, |b| b.w);
        let key = (
            w,
            h,
            r,
            attached,
            black,
            blob_count,
            right_x,
            right_w,
            self.render_scale().to_bits(),
        );
        if let Some((cached_key, buf)) = &self.glass_cache {
            if *cached_key == key {
                return buf.clone();
            }
        }
        // The liquid bridge between blobs thins and snaps as the
        // separation extends.
        let t = self.separation_now().clamp(0.0, BLOB_GAP_PX) / BLOB_GAP_PX;
        let bridge_k = BRIDGE_K_MAX * (1.0 - t);
        let buf = crate::animation::notch::glass_layer_blobs(
            w,
            h,
            blobs,
            &self.island.glass,
            black,
            bridge_k,
            self.render_scale(),
        );
        self.glass_cache = Some((key, buf.clone()));
        buf
    }

    /// Whether the idle pill is currently expanded by user click.
    fn is_expanded_idle(&self) -> bool {
        self.manually_expanded
    }

    /// The iOS/macOS presentation the pill should rest in right now:
    /// - Alert active: Expanded notification card (124px)
    /// - Manually expanded by click: Expanded card (154px)
    /// - Live agent activity or media: Compact pill (the real island never
    ///   auto-expands — live activities ride in the compact pill, split in
    ///   two blobs when both are live; tapping is what opens the dashboard,
    ///   and needs-input/failure still auto-expand as alert banners)
    /// - Hovered: Compact pill
    /// - Idle & unhovered: Hidden (top-edge hover sensor) if auto_hide is on, else Minimal dot
    pub fn presentation(&self) -> crate::animation::notch::Presentation {
        use crate::animation::notch::Presentation;
        if !self.alerts.is_empty() {
            return Presentation::Expanded;
        }
        if self.manually_expanded {
            // Click to extend it vertical and horizontal
            return Presentation::Expanded;
        }
        let agent_live = !matches!(self.state, VisualState::Idle);
        if agent_live || self.media_playing() || self.hover_expanded {
            return Presentation::Compact;
        }
        if self.island.auto_hide {
            Presentation::Hidden
        } else {
            Presentation::Minimal
        }
    }

    /// Whether the pill currently shows the wide dashboard layout (icons,
    /// agent usage, media) rather than the compact face+text layout.
    fn dashboard_expanded(&self) -> bool {
        if !self.island.is_enabled() || self.spring.is_some() {
            return false;
        }
        matches!(
            self.presentation(),
            crate::animation::notch::Presentation::Expanded
        )
    }

    /// Whether the teal media strip shows (widget on + something playing).
    fn media_playing(&self) -> bool {
        self.island.has_widget("music") && self.media.as_ref().is_some_and(|m| m.playing)
    }

    /// Stores a background worker round (media state + backdrop).
    /// Returns true when the visible frame should repaint.
    pub fn set_task_update(&mut self, update: WorkerUpdate) -> bool {
        let media_changed = self
            .media
            .as_ref()
            .map(|m| (&m.title, &m.artist, m.playing))
            != update
                .media
                .as_ref()
                .map(|m| (&m.title, &m.artist, m.playing));
        let tasks_changed = self.tasks.len() != update.tasks.len()
            || self
                .tasks
                .iter()
                .zip(&update.tasks)
                .any(|(a, b)| a.hwnd != b.hwnd);
        self.media = update.media;
        self.tasks = update.tasks;
        if !self.island.is_enabled() {
            return false;
        }
        if self.spring.is_some() {
            // A morph is in flight; the next morph tick (≤16 ms away)
            // re-renders with the new content automatically. Content never
            // freezes until the morph ends.
            return false;
        }
        if !self.dashboard_expanded() && !self.media_playing() {
            return false;
        }
        if !media_changed && !tasks_changed && !self.is_expanded_idle() && !self.media_playing() {
            return false;
        }
        let (w, h) = self.current_logical_size();
        self.current = self.render_island(self.state, w, h, self.clock_ms);
        self.animation = AnimationSource::Still(self.current.clone());
        true
    }

    /// Queues a transient alert notification banner behind the showing one
    /// (capped at [`MAX_QUEUED_ALERTS`]); the front banner shows until it
    /// expires or is dismissed, then the next takes its place.
    pub fn trigger_alert(
        &mut self,
        title: impl Into<String>,
        subtitle: impl Into<String>,
        accent: [u8; 4],
        duration_ms: u64,
        now_ms: u64,
    ) -> bool {
        if !self.island.is_enabled() {
            return false;
        }
        self.alerts.push_back(AlertBanner {
            title: title.into(),
            subtitle: subtitle.into(),
            accent,
            expires_at_ms: now_ms.saturating_add(duration_ms),
            duration_ms,
        });
        while self.alerts.len() > MAX_QUEUED_ALERTS {
            // Drop the longest-waiting unseen banner, never the showing one.
            if self.alerts.len() > 1 {
                self.alerts.remove(1);
            } else {
                self.alerts.pop_front();
            }
        }
        self.morph_to_target(now_ms)
    }

    /// Update island config live (e.g. theme switch).
    pub fn set_island_config(&mut self, island: IslandConfig, now_ms: u64) {
        let was_enabled = self.island.is_enabled();
        let is_enabled = island.is_enabled();
        self.island = island;
        // Material/geometry may have changed: drop the cached glass layer.
        self.glass_cache = None;
        // A config change clears transient hover state so the pill can never
        // get stuck expanded after hover-expand was toggled off.
        self.hover_expanded = false;
        if was_enabled != is_enabled || self.spring.is_some() {
            self.spring = None;
            if is_enabled {
                let (w, h) = self.target_size(self.state);
                self.current = self.render_island(self.state, w, h, now_ms);
                self.animation = AnimationSource::Still(self.current.clone());
                self.frame_deadline = None;
                self.arm_face_deadline(now_ms);
            } else {
                let _ = self.load_animation_classic(self.state, now_ms);
            }
        } else if is_enabled {
            let (w, h) = self.target_size(self.state);
            self.current = self.render_island(self.state, w, h, now_ms);
            self.animation = AnimationSource::Still(self.current.clone());
            self.arm_face_deadline(now_ms);
        }
    }

    /// Target (width, height) for the current visual and presentation state.
    pub fn target_size(&self, _state: VisualState) -> (u32, u32) {
        if self.island.is_bar() {
            let w = self.bar_width;
            let base_h = self.island.bar.height;
            if !self.alerts.is_empty() {
                return (w, base_h + 124);
            }
            return match self.presentation() {
                crate::animation::notch::Presentation::Expanded => {
                    let tasks_active = self.island.has_widget("tasks")
                        && self.island.show_tasks
                        && !self.tasks.is_empty();
                    let exp_h = if self.media_playing() && tasks_active {
                        210u32
                    } else if self.media_playing() {
                        175u32
                    } else if tasks_active {
                        180u32
                    } else {
                        154u32
                    };
                    (w, base_h + exp_h)
                }
                _ => (w, base_h),
            };
        }
        if !self.alerts.is_empty() {
            let w = self.island.expanded_width.max(320);
            return (w, 124u32.max(self.island.height));
        }
        match self.presentation() {
            crate::animation::notch::Presentation::Expanded => {
                let exp_w = self.island.expanded_width;
                let tasks_active = self.island.has_widget("tasks")
                    && self.island.show_tasks
                    && !self.tasks.is_empty();
                let exp_h = if self.media_playing() && tasks_active {
                    210u32
                } else if self.media_playing() {
                    175u32
                } else if tasks_active {
                    180u32
                } else {
                    154u32
                };
                (exp_w, exp_h.max(self.island.height))
            }
            crate::animation::notch::Presentation::Compact => {
                // The compact pill hugs its live content, and the split
                // island's union is the primary blob, the gap, and the
                // media blob.
                let primary = self.compact_primary_width();
                let width = if self.split_active() {
                    primary + BLOB_GAP_PX as u32 + self.compact_media_width()
                } else if self.media_playing() && self.island.has_widget("music") {
                    primary + self.compact_media_width() - 8
                } else {
                    primary
                };
                (width.max(self.island.minimal_width), self.island.height)
            }
            crate::animation::notch::Presentation::Minimal => {
                let mut w = self.island.minimal_width;
                if self.media_playing() && self.island.has_widget("music") {
                    w = w.max(72);
                }
                (w, self.island.height)
            }
            crate::animation::notch::Presentation::Hidden => (self.island.collapsed_width, 2),
        }
    }

    /// The compact pill's primary (leading) content width: the termielle
    /// face plus the live agent session dots, or the idle label.
    fn compact_primary_width(&self) -> u32 {
        let agent_visible = self.state != VisualState::Idle || self.reducer.session_count() > 0;
        if !agent_visible {
            return self.island.collapsed_width;
        }
        let mut w = 12u32;
        if self.island.has_widget("face") {
            w += 28 + 10;
        }
        if self.island.has_widget("agents") {
            let dots = self.reducer.session_count().clamp(1, 4) as u32;
            w += dots * 10 + 6;
        }
        w.max(56)
    }

    /// The compact pill's media (trailing) content width: album art plus
    /// the equalizer bars.
    fn compact_media_width(&self) -> u32 {
        if !self.island.has_widget("music") {
            return 0;
        }
        22 + 20 + 12
    }

    /// Whether the island currently presents as two blobs — the agent
    /// session pill plus a detached media pill, the Dynamic Island split —
    /// which is the compact presentation with both activities live.
    fn split_active(&self) -> bool {
        self.island.is_enabled()
            && self.presentation() == crate::animation::notch::Presentation::Compact
            && self.media_playing()
            && self.island.has_widget("music")
            && (self.state != VisualState::Idle || self.reducer.session_count() > 0)
    }

    /// The current blob-separation distance in pixels.
    fn separation_now(&self) -> f32 {
        match self.separation {
            Some(spring) => spring.x,
            None if self.split_active() => BLOB_GAP_PX,
            None => 0.0,
        }
    }

    /// The blob silhouette for a frame: one pill when merged, or the
    /// primary blob narrowing while the media blob pulls out — the
    /// smooth-min union connects them with a liquid bridge that thins and
    /// snaps as the separation spring extends.
    fn blob_rects(
        &self,
        width: u32,
        height: u32,
        radius: u32,
        attached: bool,
    ) -> Vec<crate::animation::notch::BlobRect> {
        use crate::animation::notch::BlobRect;
        let base = BlobRect {
            x: 0,
            y: 0,
            w: width,
            h: height,
            r: radius,
            attached,
        };
        let split = self.split_active();
        let sep = self.separation_now().clamp(0.0, BLOB_GAP_PX);
        if !split || sep <= 0.25 || width <= BLOB_GAP_PX as u32 {
            return vec![base];
        }
        let t = (sep / BLOB_GAP_PX).clamp(0.0, 1.0);
        let lerp = |a: f32, b: f32| a + (b - a) * t;
        let primary = self.compact_primary_width().min(width.saturating_sub(1));
        let secondary = self
            .compact_media_width()
            .min(width.saturating_sub(primary + BLOB_GAP_PX as u32).max(1));
        let left_w = lerp(width as f32, primary as f32).round().max(1.0) as u32;
        let right_x = lerp(
            width.saturating_sub(secondary) as f32,
            (primary + BLOB_GAP_PX as u32) as f32,
        )
        .round() as i32;
        vec![
            BlobRect { w: left_w, ..base },
            BlobRect {
                x: right_x,
                w: secondary,
                ..base
            },
        ]
    }

    /// Re-aims the blob-separation spring at the current presentation
    /// without disturbing its velocity.
    fn retarget_separation(&mut self, params: termielle_core::SpringParams) {
        let sep_target = if self.split_active() {
            BLOB_GAP_PX
        } else {
            0.0
        };
        let previous = self.separation.take();
        let sep_from = previous.as_ref().map_or(0.0, |s| s.x);
        let sep_vel = previous.as_ref().map_or(0.0, |s| s.v);
        if (sep_target - sep_from).abs() > 0.5 || self.split_active() {
            let mut sep = Spring1::new(sep_from, sep_target, params);
            sep.v = sep_vel;
            self.separation = Some(sep);
        }
    }

    /// The settled corner radius for the current presentation. The pill
    /// keeps near-semicircular ends; the expanded card is rounder than a
    /// pill but still generous, like iOS.
    fn target_radius(&self) -> f32 {
        match self.presentation() {
            crate::animation::notch::Presentation::Expanded => {
                let max_r = self.target_size(self.state).1 as f32 / 2.0;
                (self.island.corner_radius as f32)
                    .clamp(24.0, 34.0)
                    .min(max_r)
            }
            _ => self.island.height as f32 / 2.0,
        }
    }

    pub fn target_width(&self, state: VisualState) -> u32 {
        self.target_size(state).0
    }

    /// The active island configuration.
    pub fn island_config(&self) -> &IslandConfig {
        &self.island
    }

    /// Returns (attached, y_offset) for anchoring the island window. The
    /// anchor never moves during a morph — iOS grows the island downward
    /// from a fixed top edge; moving the window mid-morph reads as a pop.
    /// The notch stays flush with the bezel in every presentation; the
    /// floating island keeps its offset.
    pub fn island_anchor(&self) -> Option<(bool, i32)> {
        if !self.is_island() {
            return None;
        }
        if self.island.is_bar() {
            return Some((true, 0));
        }
        let attached = self.island.is_attached();
        let y_offset = if attached {
            0
        } else {
            (self.island.y_offset as f32 * self.render_scale()).round() as i32
        };
        Some((attached, y_offset))
    }

    /// Refreshes the monitor DPI scale from the window. Called on every
    /// present, so dragging the pill across monitors with different DPIs
    /// tracks without any cache-invalidation path. Garbage in is ignored.
    pub fn set_dpi_scale(&mut self, scale: f32) {
        if scale.is_finite() && scale > 0.0 {
            self.dpi_scale = scale.clamp(0.5, 4.0);
        }
    }

    /// User zoom from `AppConfig.scale`, applied on top of monitor DPI.
    pub fn set_user_scale(&mut self, scale: f32) {
        if scale.is_finite() && scale > 0.0 {
            self.user_scale = scale.clamp(0.5, 2.0);
        }
    }

    /// Sets the logical monitor width for Waybar layout.
    pub fn set_bar_width(&mut self, width_logical: u32) {
        if width_logical > 0 && self.bar_width != width_logical {
            self.bar_width = width_logical;
            if self.island.is_bar() {
                let (_, h) = self.target_size(self.state);
                self.current = self.render_island(self.state, width_logical, h, self.clock_ms);
                self.animation = AnimationSource::Still(self.current.clone());
            }
        }
    }

    /// Physical pixels per logical pixel for the next present: monitor DPI
    /// (unless the island opts out via `scale_with_dpi`) times the user
    /// zoom. Frames are authored at this scale, so the window presents
    /// them 1:1 with no filtering.
    pub fn render_scale(&self) -> f32 {
        let dpi = if self.island.scale_with_dpi {
            self.dpi_scale
        } else {
            1.0
        };
        (dpi * self.user_scale).clamp(0.5, 4.0)
    }

    /// Allocates a transparent frame for a logical (`width`, `height`) at
    /// the current render scale: layout reads logical, the raster is device
    /// pixels, and paint operations scale their inputs by [`FrameBuffer::scale`].
    fn blank_frame(&self, width: u32, height: u32) -> FrameBuffer {
        let scale = self.render_scale();
        let (width, height) = scaled_size((width.max(1), height.max(1)), scale);
        FrameBuffer {
            width,
            height,
            pixels_pbgra: vec![0; (width * height * 4) as usize],
            delay_ms: 0,
            loop_index: 0,
            scale,
        }
    }

    /// Maps one physical client pixel back to frame (logical) space for
    /// hit-testing against `icon_hits` and `hover_point`.
    fn to_logical(&self, v: i32) -> i32 {
        (v as f32 / self.render_scale()).round() as i32
    }

    /// The current frame's size in logical units: the window and the spring
    /// both reason in logical pixels while [`FrameBuffer`] holds device pixels.
    /// Derived from the frame's own authoring scale, never the live render
    /// scale — right after a DPI change the two differ, and the live scale
    /// would unpick the wrong logical size.
    fn current_logical_size(&self) -> (u32, u32) {
        let authored = self.current.scale;
        let authored = if authored.is_finite() && authored > 0.0 {
            authored
        } else {
            self.render_scale()
        };
        (
            (self.current.width as f32 / authored).round().max(1.0) as u32,
            (self.current.height as f32 / authored).round().max(1.0) as u32,
        )
    }

    /// Re-renders the current frame when its pixels no longer match the
    /// render scale (the pill crossed monitors with different DPIs, or the
    /// zoom changed under a settled pill). Returns true when a repaint is
    /// needed. Self-healing: scale is part of the glass cache key, so a
    /// stale frame simply misses and rebuilds here.
    pub fn refresh_scale(&mut self, now_ms: u64) -> bool {
        let (logical_w, logical_h) = self.current_logical_size();
        let (want_w, want_h) = scaled_size((logical_w, logical_h), self.render_scale());
        if (want_w, want_h) == (self.current.width, self.current.height) {
            return false;
        }
        if self.island.is_enabled() {
            self.current = self.render_island(self.state, logical_w, logical_h, now_ms);
        } else {
            let _ = self.load_animation_classic(self.state, now_ms);
            return true;
        }
        self.animation = AnimationSource::Still(self.current.clone());
        true
    }

    /// Resamples a device-pixel asset frame (classic-mode GIF art, authored
    /// at 1.0) up to `scale`, preserving the old present-time upscale
    /// exactly: same bilinear math, same window size, one step earlier so
    /// the present path stays 1:1.
    fn to_render_size(scale: f32, frame: FrameBuffer) -> FrameBuffer {
        let (want_w, want_h) = scaled_size((frame.width, frame.height), scale);
        if (want_w, want_h) == (frame.width, frame.height) {
            return frame;
        }
        let mut out = crate::animation::resample_bilinear(&frame, want_w, want_h);
        out.scale = scale;
        out
    }

    /// Theme-aware ink for neutral copy; see
    /// [`crate::animation::notch::ink_pair`].
    fn ink(&self) -> [u8; 4] {
        crate::animation::notch::ink_pair(&self.island.glass).0
    }

    /// Dim companion ink for secondary copy.
    fn ink_dim(&self) -> [u8; 4] {
        crate::animation::notch::ink_pair(&self.island.glass).1
    }

    /// Routes a click at physical client (`x`, `y`): coordinates are mapped
    /// to frame space before hit-testing, then —
    /// - over interactive buttons (media control, window switcher, notification dismiss)
    /// - over the pill otherwise: toggles expansion and returns the outcome.
    pub fn handle_click(&mut self, x: i32, y: i32, now_ms: u64) -> ClickOutcome {
        let (x, y) = (self.to_logical(x), self.to_logical(y));
        let hit_id = self
            .icon_hits
            .iter()
            .find(|&&(_, hx, hy, hw, hh)| {
                x >= hx && x < hx + hw as i32 && y >= hy && y < hy + hh as i32
            })
            .map(|&(id, ..)| id);

        if let Some(id) = hit_id {
            match id {
                HIT_MEDIA_PLAY_PAUSE => return ClickOutcome::MediaToggle,
                HIT_MEDIA_PREV => return ClickOutcome::MediaPrev,
                HIT_MEDIA_NEXT => return ClickOutcome::MediaNext,
                HIT_ALERT_DISMISS => {
                    self.alerts.pop_front();
                    let _ = self.morph_to_target(now_ms);
                    return ClickOutcome::AlertDismiss;
                }
                crate::bar::HIT_BAR_VOLUME_TOGGLE => return ClickOutcome::VolumeToggle,
                crate::bar::HIT_BAR_ISLAND_PILL => {
                    if self.manually_expanded {
                        self.manually_expanded = false;
                        let _ = self.morph_to_target(now_ms);
                        return ClickOutcome::Collapsed;
                    } else if self.toggle_expand(now_ms) {
                        return ClickOutcome::Expanded;
                    }
                }
                id if id <= crate::bar::HIT_BAR_WORKSPACE_BASE
                    && id > crate::bar::HIT_BAR_WORKSPACE_BASE - 50 =>
                {
                    let ws_num = (crate::bar::HIT_BAR_WORKSPACE_BASE - id) as usize;
                    return ClickOutcome::WorkspaceSwitch(ws_num as u32);
                }
                hwnd if hwnd > 0 => return ClickOutcome::ActivateWindow(hwnd),
                _ => {}
            }
        }
        if self.manually_expanded {
            self.manually_expanded = false;
            let _ = self.morph_to_target(now_ms);
            return ClickOutcome::Collapsed;
        }
        if self.toggle_expand(now_ms) {
            return ClickOutcome::Expanded;
        }
        ClickOutcome::None
    }

    /// Stores the cursor position for icon hover highlighting. The poll
    /// reports physical client pixels; they are mapped to frame space before
    /// comparing against `icon_hits`. Triggers a repaint only when the
    /// highlighted icon changed.
    pub fn set_hover_point(&mut self, point: Option<(i32, i32)>) -> bool {
        let point = point.map(|(x, y)| (self.to_logical(x), self.to_logical(y)));
        if self.hover_point == point {
            return false;
        }
        self.hover_point = point;
        // Repaint only when the point moved across an icon hit-rect boundary.
        let on_icon = |pt: &Option<(i32, i32)>| {
            pt.is_some_and(|(px, py)| {
                self.icon_hits.iter().any(|(_, hx, hy, hw, hh)| {
                    px >= *hx && px < hx + *hw as i32 && py >= *hy && py < hy + *hh as i32
                })
            })
        };
        on_icon(&self.hover_point) != on_icon(&point)
    }

    pub fn toggle_expand(&mut self, now_ms: u64) -> bool {
        if !self.island.is_enabled() {
            return false;
        }
        self.manually_expanded = !self.manually_expanded;
        self.morph_to_target(now_ms)
    }

    /// Whether the island is currently manually expanded into the full card.
    pub fn is_manually_expanded(&self) -> bool {
        self.manually_expanded
    }

    /// Collapses the island if it was manually expanded (e.g. click outside or Escape).
    pub fn collapse_if_expanded(&mut self, now_ms: u64) -> bool {
        if !self.island.is_enabled() || !self.manually_expanded {
            return false;
        }
        self.manually_expanded = false;
        self.morph_to_target(now_ms)
    }

    /// Tracks the cursor entering (`inside = true`) or leaving the pill.
    /// Hover expands the idle pill when `expand_on_hover` is set; leaving
    /// collapses it again unless it was manually toggled open.
    pub fn set_hover(&mut self, inside: bool, now_ms: u64) -> bool {
        if !self.island.is_enabled() || !self.island.expand_on_hover {
            return false;
        }
        let inside = if self.island.is_bar() {
            if let Some((hx, hy)) = self.hover_point {
                let (ix, iy, iw, ih) = if self.is_expanded_idle() {
                    self.bar_card_rect()
                } else if !self.bar_module("center", "island") {
                    // Hidden center module: no pill to hover.
                    (0, 0, 0, 0)
                } else {
                    let (px, py, pw, ph) = self.bar_pill_rect(self.bar_width);
                    (px, py, pw as i32, ph as i32)
                };
                hx >= ix && hx < ix + iw && hy >= iy && hy < iy + ih
            } else {
                false
            }
        } else {
            inside
        };
        if self.state != VisualState::Idle || self.manually_expanded {
            // Still track the flag so a leave during an agent turn can't
            // collapse anything afterwards.
            self.hover_expanded = inside;
            return false;
        }
        if self.hover_expanded == inside {
            return false;
        }
        self.hover_expanded = inside;
        self.morph_to_target(now_ms)
    }

    /// Press feedback: the pointer went down (`true`) or up on the pill.
    /// The island swells ~3% under the pointer, like the Dynamic Island
    /// under the fingertip, and settles back on release.
    pub fn set_pressed(&mut self, pressed: bool, now_ms: u64) -> bool {
        if !self.island.is_enabled() || self.pressed == pressed {
            return false;
        }
        self.pressed = pressed;
        self.morph_to_target(now_ms)
    }

    /// Spring for one morph: growing surfaces open with the configured
    /// bounce, shrinking ones settle critically damped, and alert drop-ins
    /// get their own fast snap. Areas decide — no per-caller wiring.
    fn morph_params(
        &self,
        from_w: u32,
        from_h: u32,
        to_w: u32,
        to_h: u32,
    ) -> termielle_core::SpringParams {
        let growing = to_w as u64 * to_h as u64 >= from_w as u64 * from_h as u64;
        if !self.alerts.is_empty() && growing {
            spring_params(self.island.alert_ms, self.island.spring_bounce)
        } else if growing {
            spring_params(self.island.animation_ms, self.island.spring_bounce)
        } else {
            spring_params(self.island.collapse_ms, 0.0)
        }
    }

    /// Morphs (or snaps, under reduced motion) to the current target size
    /// using the iOS spring model. Interrupted morphs inherit the current
    /// velocity, exactly like the Dynamic Island. The corner radius and
    /// the blob separation ride the same motion.
    fn morph_to_target(&mut self, now_ms: u64) -> bool {
        let (mut target_w, mut target_h) = self.target_size(self.state);
        let mut target_r = self.target_radius();
        // Press swell: inflate the target a few percent while held (not in bar mode).
        if self.pressed && !self.island.is_bar() {
            target_w = ((target_w as f32 * 1.03).round() as u32).max(target_w + 2);
            target_h = ((target_h as f32 * 1.03).round() as u32).max(target_h + 2);
            target_r *= 1.03;
        }
        let (from_w, from_h) = self.current_logical_size();
        let params = self.morph_params(from_w, from_h, target_w, target_h);
        self.retarget_separation(params);

        if self.reduced_motion {
            self.spring = None;
            self.radius = target_r;
            self.current = self.render_island(self.state, target_w, target_h, now_ms);
            self.animation = AnimationSource::Still(self.current.clone());
            self.frame_deadline = None;
            return true;
        }
        let (from_w, from_h, from_r, vel_x, vel_y, vel_z) = match self.spring.take() {
            Some(s) => (s.x, s.y, s.z, s.vx, s.vy, s.vz),
            None => {
                let (logical_w, logical_h) = self.current_logical_size();
                (
                    logical_w as f32,
                    logical_h as f32,
                    self.radius,
                    0.0,
                    0.0,
                    0.0,
                )
            }
        };
        if (from_w - target_w as f32).abs() < 1.0
            && (from_h - target_h as f32).abs() < 1.0
            && (from_r - target_r).abs() < 0.5
            && vel_x.abs() < 1.0
            && vel_y.abs() < 1.0
            && vel_z.abs() < 1.0
        {
            self.spring = None;
            self.radius = target_r;
            self.current = self.render_island(self.state, target_w, target_h, now_ms);
            self.animation = AnimationSource::Still(self.current.clone());
            self.frame_deadline = None;
            return true;
        }
        let mut spring = Spring2D::new(
            from_w.round() as u32,
            from_h.round() as u32,
            from_r,
            target_w,
            target_h,
            target_r,
            params,
        );
        spring.vx = vel_x;
        spring.vy = vel_y;
        spring.vz = vel_z;
        self.spring = Some(spring);
        self.spring_last_ms = now_ms;
        let interval = self.frame_interval_ms.unwrap_or(16);
        let next = now_ms
            .saturating_add(interval)
            .saturating_sub(self.present_cost_ms.min(interval.saturating_sub(1)));
        self.frame_deadline = Some(next);
        true
    }

    /// Feeds the measured cost of the last present back into frame pacing.
    pub fn set_present_cost(&mut self, elapsed_ms: u64) {
        self.present_cost_ms = (self.present_cost_ms + elapsed_ms) / 2;
    }

    /// Folds one pipe event in and returns what the window must do.
    pub fn handle_event(&mut self, event: EventMessage, now_ms: u64) -> ControllerActions {
        self.clock_ms = now_ms;
        let kind = event.event;
        let session = event.session_id.clone();
        let fresh = now_ms.saturating_sub(event.timestamp_ms) <= ALERT_FRESHNESS_MS;
        self.reducer.advance(now_ms);
        self.reducer.apply(event);
        let mut actions = self.sync_state(now_ms);

        if self.island.is_enabled() {
            match kind {
                termielle_core::EventKind::NeedsInput if fresh => {
                    self.trigger_alert(
                        "Input Required",
                        format!("Session: {session}"),
                        [255, 180, 50, 255],
                        3500,
                        now_ms,
                    );
                    actions.present_frame = true;
                }
                termielle_core::EventKind::TurnFailed if fresh => {
                    self.trigger_alert(
                        "Task Failed",
                        format!("Session: {session}"),
                        [250, 70, 70, 255],
                        3500,
                        now_ms,
                    );
                    actions.present_frame = true;
                }
                _ => {}
            }
        }

        actions.next_deadline_ms = self.next_deadline_ms();
        actions
    }

    /// Fires every reducer and animation deadline due at `now_ms`.
    pub fn on_timer(&mut self, now_ms: u64) -> ControllerActions {
        self.clock_ms = now_ms;
        self.reducer.advance(now_ms);
        let mut actions = self.sync_state(now_ms);

        // NOTE: dashboard content arrives from the background worker via
        // `set_task_update`; nothing here may block on window capture or
        // media queries, otherwise hovering and morphing visibly stall.

        // Check if the showing transient notification alert expired; the
        // next queued banner (if any) takes its place via the re-morph.
        let expired = self
            .alerts
            .front()
            .is_some_and(|alert| now_ms >= alert.expires_at_ms);
        if expired {
            self.alerts.pop_front();
            self.morph_to_target(now_ms);
            actions.present_frame = true;
        }

        // Spring morph tick: integrate (width, height, radius) toward the
        // target with the iOS spring model, alongside the blob-separation
        // spring.
        if self.island.is_enabled() {
            if let Some(spring) = self.spring.as_mut() {
                let mut elapsed = now_ms.saturating_sub(self.spring_last_ms).min(200);
                self.spring_last_ms = now_ms;
                let omega = spring.stiffness.sqrt().max(1.0);
                let sub_ms = ((0.35 / omega) * 1000.0).ceil().max(2.0) as u64;
                let mut current_size = (spring.x.round() as u32, spring.y.round() as u32);
                let mut settled = false;
                while elapsed > 0 && !settled {
                    let dt = (elapsed.min(sub_ms) as f32 / 1000.0).max(0.0005);
                    elapsed = elapsed.saturating_sub(sub_ms);
                    if let Some(sep) = self.separation.as_mut() {
                        sep.step(dt);
                    }
                    let ((nw, nh), ns) = {
                        let (size, s) = spring.step(dt);
                        ((size.0, size.1), s)
                    };
                    current_size = (nw, nh);
                    settled = ns;
                }
                self.radius = spring.z;
                let target_w = spring.target_x.round() as u32;
                let target_h = spring.target_y.round() as u32;
                let separation_settled = self.separation.as_ref().is_none_or(|s| s.settled());
                let target_reached =
                    settled || (current_size == (target_w, target_h) && separation_settled);
                if target_reached {
                    self.spring = None;
                    if let Some(sep) = self.separation.as_mut() {
                        sep.x = sep.target;
                        sep.v = 0.0;
                    }
                    self.current = self.render_island(self.state, target_w, target_h, now_ms);
                    self.animation = AnimationSource::Still(self.current.clone());
                    self.frame_deadline = None;
                    self.arm_face_deadline(now_ms);
                } else {
                    let (w, h) = current_size;
                    self.current = self.render_island(self.state, w, h, now_ms);
                    self.animation = AnimationSource::Still(self.current.clone());
                    let interval = self.frame_interval_ms.unwrap_or(16);
                    let next = now_ms
                        .saturating_add(interval)
                        .saturating_sub(self.present_cost_ms.min(interval.saturating_sub(1)));
                    self.frame_deadline = Some(next);
                }
                actions.present_frame = true;
            } else if self.separation.is_some() {
                // The size settled but the blobs are still travelling.
                let mut sep = self.separation.take().unwrap_or_default();
                sep.step(1.0 / 60.0);
                let settled = sep.settled();
                if settled {
                    sep.x = sep.target;
                    sep.v = 0.0;
                }
                if sep.target > 0.0 || !settled {
                    self.separation = Some(sep);
                }
                if !settled || self.split_active() {
                    let (w, h) = self.target_size(self.state);
                    self.current = self.render_island(self.state, w, h, now_ms);
                    actions.present_frame = true;
                }
                if !settled {
                    let interval = self.frame_interval_ms.unwrap_or(16);
                    self.frame_deadline = Some(now_ms.saturating_add(interval));
                }
            }
        }

        // Face-animation tick: cheap (pre-decoded frames + cached icons).
        // It keeps ticking during morphs — on the real island the activity
        // content never freezes while the container is moving.
        if self.island.is_enabled() {
            if let Some(face_at) = self.face_deadline {
                if now_ms >= face_at {
                    self.advance_face(now_ms);
                    actions.present_frame = true;
                }
            }
        }

        // Procedural-motion tick: repaints the pill so bounce, orbit, pulse,
        // sparkles, shake, and the equalizer move even with no other
        // deadline pending. Skipped while a morph tick already repainted
        // this call — the poses ride the morph renders instead.
        if self.motion_deadline.is_some_and(|d| now_ms >= d)
            && self.spring.is_none()
            && self.separation.is_none()
        {
            self.motion_deadline = None;
            let (w, h) = self.current_logical_size();
            self.current = self.render_island(self.state, w, h, now_ms);
            self.animation = AnimationSource::Still(self.current.clone());
            actions.present_frame = true;
        }
        if self.motion_active(now_ms) {
            let due = now_ms.saturating_add(MOTION_TICK_MS);
            self.motion_deadline = Some(match self.motion_deadline {
                Some(d) => d.min(due),
                None => due,
            });
        } else {
            self.motion_deadline = None;
        }

        // Bar mode periodic update (1-second cadence for clock and system metrics)
        if self.island.is_bar() {
            if self.bar_deadline.is_some_and(|d| now_ms >= d) {
                self.bar_deadline = Some(now_ms.saturating_add(1000));
                let (w, h) = self.current_logical_size();
                self.current = self.render_island(self.state, w, h, now_ms);
                self.animation = AnimationSource::Still(self.current.clone());
                actions.present_frame = true;
            } else if self.bar_deadline.is_none() {
                self.bar_deadline = Some(now_ms.saturating_add(1000));
            }
        }

        if let Some(deadline) = self.frame_deadline {
            if self.spring.is_none() && now_ms >= deadline {
                actions.present_frame = true;
                if let Some(code) = self.advance_animation(now_ms) {
                    actions.error_code = Some(code);
                }
            }
        }

        actions.next_deadline_ms = self.next_deadline_ms();
        actions
    }

    /// Whether any procedural motion is live: thinking bounce, worker orbit,
    /// input pulse, celebration/shake one-shots, or a playing equalizer.
    /// Gated on island mode and full motion — reduced motion stills
    /// everything procedural (the face keeps its own deadline). The media
    /// equalizer only moves while actually playing; a paused pose is static.
    fn motion_active(&self, now_ms: u64) -> bool {
        if !self.island.is_enabled() || self.reduced_motion {
            return false;
        }
        if self.media_playing() {
            return true;
        }
        match self.state {
            VisualState::Thinking | VisualState::Working | VisualState::NeedsInput => true,
            VisualState::Ready => now_ms.saturating_sub(self.state_since_ms) < 600,
            VisualState::Failed => now_ms.saturating_sub(self.state_since_ms) < 300,
            VisualState::Idle => false,
        }
    }

    /// The earliest moment [`Controller::handle_event`] or [`Controller::on_timer`]
    /// can change anything, or `None` when no deadline is pending.
    pub fn next_deadline_ms(&self) -> Option<u64> {
        let earliest = match (self.reducer.next_deadline_ms(), self.frame_deadline) {
            (Some(reducer_at), Some(frame_at)) => Some(reducer_at.min(frame_at)),
            (reducer_at, frame_at) => reducer_at.or(frame_at),
        };
        let mut deadline = match (earliest, self.face_deadline) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        if let Some(alert) = self.alerts.front() {
            deadline = match deadline {
                Some(d) => Some(d.min(alert.expires_at_ms)),
                None => Some(alert.expires_at_ms),
            };
        }
        deadline = match (deadline, self.motion_deadline) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        if self.island.is_bar() {
            if let Some(bar_at) = self.bar_deadline {
                deadline = match deadline {
                    Some(d) => Some(d.min(bar_at)),
                    None => Some(bar_at),
                };
            }
        }
        deadline
    }

    /// The frame that should currently be on screen.
    pub fn current_frame(&self) -> &FrameBuffer {
        &self.current
    }

    /// The state currently being presented; drives the acknowledgement file.
    pub fn visible_state(&self) -> VisualState {
        self.state
    }

    /// Whether island mode is active.
    pub fn is_island(&self) -> bool {
        self.island.is_enabled()
    }

    /// Replaces the active frame with the still for the current state.
    pub fn fallback_to_still(&mut self) {
        if self.island.is_enabled() {
            let (w, h) = self.target_size(self.state);
            self.spring = None;
            self.current = self.render_island(self.state, w, h, self.clock_ms);
            self.animation = AnimationSource::Still(self.current.clone());
            self.frame_deadline = None;
            self.arm_face_deadline(0);
        } else {
            let still = fallback_frame(self.state, FALLBACK_FRAME_SIZE, self.render_scale());
            self.frame_deadline = None;
            self.current = still.clone();
            self.animation = AnimationSource::Still(still);
        }
    }

    /// Reloads the animation when the visible state moved.
    fn sync_state(&mut self, now_ms: u64) -> ControllerActions {
        let mut actions = ControllerActions::default();
        let state = self.reducer.visible_state();
        if state != self.state {
            self.state = state;
            self.state_since_ms = now_ms;
            actions.visible_state = Some(state);
            actions.present_frame = true;
            // A manual expansion is the user's explicit open: mid-turn flips
            // (thinking -> working) must not collapse it from under them.
            // Only the turn ending retires the card.
            if state == VisualState::Idle {
                self.manually_expanded = false;
            }
            if self.island.is_enabled() {
                self.refresh_face(state);
                let (target_w, target_h) = self.target_size(state);
                let (current_w, current_h) = self.current_logical_size();
                if (current_w == target_w && current_h == target_h) || self.reduced_motion {
                    self.current = self.render_island(state, target_w, target_h, now_ms);
                    self.animation = AnimationSource::Still(self.current.clone());
                    self.spring = None;
                    self.frame_deadline = None;
                    self.arm_face_deadline(now_ms);
                } else {
                    let params = self.morph_params(current_w, current_h, target_w, target_h);
                    self.retarget_separation(params);
                    self.spring = Some(Spring2D::new(
                        current_w,
                        current_h,
                        self.radius,
                        target_w,
                        target_h,
                        self.target_radius(),
                        params,
                    ));
                    self.spring_last_ms = now_ms;
                    let interval = self.frame_interval_ms.unwrap_or(16);
                    let next = now_ms
                        .saturating_add(interval)
                        .saturating_sub(self.present_cost_ms.min(interval.saturating_sub(1)));
                    self.frame_deadline = Some(next);
                }
            } else {
                actions.error_code = self.load_animation_classic(state, now_ms);
            }
        } else if self.island.is_enabled() {
            // Visual state didn't move, but active content (e.g. sessions or media) may
            // dynamically change the target pill size.
            let (target_w, target_h) = self.target_size(self.state);
            let (current_w, current_h) = self.current_logical_size();
            if (current_w, current_h) != (target_w, target_h) && self.spring.is_none() {
                if self.reduced_motion {
                    self.current = self.render_island(self.state, target_w, target_h, now_ms);
                    self.animation = AnimationSource::Still(self.current.clone());
                    actions.present_frame = true;
                } else {
                    let params = self.morph_params(current_w, current_h, target_w, target_h);
                    self.retarget_separation(params);
                    self.spring = Some(Spring2D::new(
                        current_w,
                        current_h,
                        self.radius,
                        target_w,
                        target_h,
                        self.target_radius(),
                        params,
                    ));
                    self.spring_last_ms = now_ms;
                    let interval = self.frame_interval_ms.unwrap_or(16);
                    let next = now_ms
                        .saturating_add(interval)
                        .saturating_sub(self.present_cost_ms.min(interval.saturating_sub(1)));
                    self.frame_deadline = Some(next);
                    actions.present_frame = true;
                }
            }
        }
        actions
    }

    /// Loads the animation for `state` (classic pet path only).
    fn load_animation_classic(&mut self, state: VisualState, now_ms: u64) -> Option<i32> {
        self.frame_deadline = None;
        self.current = Self::to_render_size(
            self.render_scale(),
            fallback_frame(state, FALLBACK_FRAME_SIZE, 1.0),
        );

        let path = self.assets.resolve(state)?;
        let mut gif = match GifAnimation::open(&path) {
            Ok(gif) => gif,
            Err(error) => {
                self.animation = AnimationSource::Still(Self::to_render_size(
                    self.render_scale(),
                    fallback_frame(state, FALLBACK_FRAME_SIZE, 1.0),
                ));
                return Some(error_code(&error));
            }
        };

        match gif.next_frame() {
            Ok(frame) => {
                self.current = Self::to_render_size(self.render_scale(), frame.clone());
                if self.reduced_motion {
                    self.animation = AnimationSource::Still(self.current.clone());
                } else {
                    let interval = self.frame_interval_ms;
                    let present_cost = self.present_cost_ms;
                    self.frame_deadline =
                        Some(deadline_for(interval, present_cost, now_ms, frame.delay_ms));
                    self.animation = AnimationSource::Gif(gif);
                }
                None
            }
            Err(error) => {
                self.animation = AnimationSource::Still(Self::to_render_size(
                    self.render_scale(),
                    fallback_frame(state, FALLBACK_FRAME_SIZE, 1.0),
                ));
                Some(error_code(&error))
            }
        }
    }

    /// Advances the active GIF by one frame and re-arms its deadline.
    fn advance_animation(&mut self, now_ms: u64) -> Option<i32> {
        if self.island.is_enabled() {
            return None;
        }
        let interval = self.frame_interval_ms;
        let present_cost = self.present_cost_ms;
        let render_scale = self.render_scale();
        let gif = match &mut self.animation {
            AnimationSource::Gif(gif) => gif,
            AnimationSource::Still(_) => {
                self.frame_deadline = None;
                return None;
            }
        };

        match gif.next_frame() {
            Ok(frame) => {
                self.current = Self::to_render_size(render_scale, frame.clone());
                self.frame_deadline =
                    Some(deadline_for(interval, present_cost, now_ms, frame.delay_ms));
                None
            }
            Err(error) => {
                let code = error_code(&error);
                self.fallback_to_still();
                Some(code)
            }
        }
    }
}

/// Box-downscales a premultiplied BGRA frame to `size x size`.
fn downscale_frame(src: &FrameBuffer, size: u32) -> FrameBuffer {
    let mut out = vec![0u8; (size * size * 4) as usize];
    if src.width == 0 || src.height == 0 {
        return FrameBuffer {
            width: size,
            height: size,
            pixels_pbgra: out,
            delay_ms: 0,
            loop_index: 0,
            scale: 1.0,
        };
    }
    for ty in 0..size {
        let sy0 = ty as u64 * src.height as u64 / size as u64;
        let sy1 = ((ty + 1) as u64 * src.height as u64 / size as u64).max(sy0 + 1);
        for tx in 0..size {
            let sx0 = tx as u64 * src.width as u64 / size as u64;
            let sx1 = ((tx + 1) as u64 * src.width as u64 / size as u64).max(sx0 + 1);
            let mut acc = [0u64; 4];
            let mut count = 0u64;
            for sy in sy0..sy1.min(src.height as u64) {
                for sx in sx0..sx1.min(src.width as u64) {
                    let i = ((sy * u64::from(src.width) + sx) * 4) as usize;
                    for (c, slot) in acc.iter_mut().enumerate() {
                        *slot += src.pixels_pbgra[i + c] as u64;
                    }
                    count += 1;
                }
            }
            let o = ((ty * size + tx) * 4) as usize;
            let count = count.max(1);
            for (c, slot) in out[o..o + 4].iter_mut().enumerate() {
                *slot = (acc[c] / count) as u8;
            }
        }
    }
    FrameBuffer {
        width: size,
        height: size,
        pixels_pbgra: out,
        delay_ms: 0,
        loop_index: 0,
        scale: 1.0,
    }
}

/// The deadline for the next animation frame: the fixed interval when
/// configured (minus the smoothed present cost), else the frame's own GIF
/// delay.
fn deadline_for(
    interval: Option<u64>,
    present_cost_ms: u64,
    now_ms: u64,
    gif_delay_ms: u32,
) -> u64 {
    match interval {
        Some(interval) => now_ms
            .saturating_add(interval)
            .saturating_sub(present_cost_ms.min(interval.saturating_sub(1))),
        None => now_ms.saturating_add(u64::from(gif_delay_ms.max(1))),
    }
}

/// Stable numeric identity for an animation failure, safe for bounded logs.
fn error_code(error: &AnimationError) -> i32 {
    match error {
        AnimationError::Win32(code) => *code as i32,
        AnimationError::ReadFailed => 1,
        AnimationError::Empty => 2,
        AnimationError::Unsupported => 3,
    }
}

#[test]
fn think_bob_bounces_only_while_thinking() {
    use termielle_core::VisualState;
    assert_eq!(Controller::think_bob(VisualState::Thinking, 0, 0), 0);
    assert_ne!(
        Controller::think_bob(VisualState::Thinking, 0, 0),
        Controller::think_bob(VisualState::Thinking, 0, 120)
    );
    assert_eq!(Controller::think_bob(VisualState::Working, 0, 120), 0);
    assert_eq!(Controller::think_bob(VisualState::Idle, 2, 5000), 0);
}

#[test]
fn shake_fires_briefly_then_locks_to_zero() {
    use termielle_core::VisualState;
    assert_eq!(Controller::shake_dx(VisualState::Failed, 0), 0);
    assert_ne!(Controller::shake_dx(VisualState::Failed, 50), 0);
    assert_eq!(Controller::shake_dx(VisualState::Failed, 300), 0);
    assert_eq!(Controller::shake_dx(VisualState::Failed, 5000), 0);
    assert_eq!(Controller::shake_dx(VisualState::Working, 50), 0);
}

#[test]
fn sparkle_flies_out_then_vanishes() {
    let start = Controller::sparkle_dot(100, 100, 0, 0).unwrap();
    let mid = Controller::sparkle_dot(100, 100, 0, 300).unwrap();
    assert!(mid.0 - 100 > start.0 - 100, "radius must grow with age");
    assert!(mid.2 < start.2, "alpha must fade with age");
    assert_eq!(Controller::sparkle_dot(100, 100, 0, 600), None);
    assert_eq!(Controller::sparkle_dot(100, 100, 0, 5000), None);
}

#[test]
fn orbit_circles_with_time() {
    let a = Controller::orbit_dot(50, 50, 10, 0, 0);
    let b = Controller::orbit_dot(50, 50, 10, 0, 150);
    assert_ne!(a, b);
    for t in [0, 100, 250, 599] {
        let (x, y) = Controller::orbit_dot(50, 50, 10, 1, t);
        let d = (((x - 50) * (x - 50) + (y - 50) * (y - 50)) as f32).sqrt();
        assert!((d - 10.0).abs() <= 1.5, "off circle: {d}");
    }
}
