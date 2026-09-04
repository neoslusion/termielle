//! The controller: folds pipe events and timer ticks into window actions.
//!
//! [`Controller`] owns the reducer, the active animation, and the single
//! deadline the window's timer is armed for. It never touches Win32 itself;
//! [`main`](crate::main) maps its actions onto the window, the pipe, and the
//! log.

use crate::animation::{
    AnimationError, AnimationSource, FrameBuffer, GifAnimation, fallback_frame, island_frame,
};
use crate::tasks::{MediaInfo, TaskIcon, WorkerUpdate};
use termielle_core::{
    AssetCatalog, EventMessage, IslandConfig, IslandGeometry, SessionReducer, VisualState,
    spring_params,
};

/// Edge length of the termielle face rendered inside the notch.
const FACE_SIZE: u32 = 44;

/// Square edge length of procedural fallback frames.
pub const FALLBACK_FRAME_SIZE: u32 = 360;

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

/// What a click on the island should do.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClickOutcome {
    /// Activate (foreground) the task window under the clicked icon.
    ActivateWindow(isize),
    /// The dashboard opened.
    Expanded,
    /// The dashboard closed.
    Collapsed,
    /// Click landed outside any interactive element.
    None,
}

/// The iOS Dynamic Island morphs between presentations with a spring, not a
/// timing curve — interrupted morphs keep their velocity and settle
/// naturally. This integrator tracks pill width (px) and velocity (px/s).
#[derive(Clone, Copy, Debug)]
struct Spring {
    /// Current animated width.
    x: f32,
    /// Current velocity in px/s (kept across interruptions).
    v: f32,
    /// Target width.
    target: f32,
    /// Params from `spring_params(animation_ms, spring_bounce)`.
    stiffness: f32,
    damping: f32,
}

/// Below these the spring is considered settled and snaps to target.
const SETTLE_PX: f32 = 0.5;
const SETTLE_V: f32 = 2.0;

impl Spring {
    fn new(from_w: u32, target_w: u32, params: termielle_core::SpringParams) -> Self {
        Self {
            x: from_w as f32,
            v: 0.0,
            target: target_w as f32,
            stiffness: params.stiffness,
            damping: params.damping,
        }
    }

    /// Integrates one step of `dt` seconds. Returns (width, settled).
    fn step(&mut self, dt: f32) -> (u32, bool) {
        let accel = -self.stiffness * (self.x - self.target) - self.damping * self.v;
        self.v += accel * dt;
        self.x += self.v * dt;
        let settled = (self.x - self.target).abs() < SETTLE_PX && self.v.abs() < SETTLE_V;
        if settled {
            self.x = self.target;
            self.v = 0.0;
        }
        (self.x.round().max(1.0) as u32, settled)
    }
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
    /// Fixed interval between animation frames in milliseconds; when set, it
    /// overrides each frame's GIF delay so the animation plays at a constant
    /// frame rate (e.g. 60 fps) instead of the file's own timing.
    frame_interval_ms: Option<u64>,
    /// Smoothed cost of presenting one frame, in milliseconds.
    present_cost_ms: u64,
    island: IslandConfig,
    /// Active width spring (island mode); `None` when settled or classic.
    spring: Option<Spring>,
    /// Timestamp of the last spring integration (for real elapsed dt).
    spring_last_ms: u64,
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
    icons: Vec<TaskIcon>,
    /// Hit rects for the task icons of the last rendered frame:
    /// (hwnd, x, y, w, h) in frame coordinates. Used for click activation.
    icon_hits: Vec<(isize, i32, i32, u32, u32)>,
    /// Cursor position in frame coordinates (from the hover poll), so the
    /// icon under the cursor draws its accent border.
    hover_point: Option<(i32, i32)>,
    /// Cached frosted-glass layer keyed by (w, h, radius, attached). The
    /// single-pass paint is ~1ms; caching makes face ticks ~free.
    glass_cache: Option<((u32, u32, u32, bool), FrameBuffer)>,
    /// Last now-playing media state from the background worker.
    media: Option<MediaInfo>,
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
        let face_frame = fallback_frame(state, FACE_SIZE);
        // Overwritten below for both island and classic modes.
        let still = fallback_frame(state, FALLBACK_FRAME_SIZE);
        let (current, animation, frame_deadline) =
            (still.clone(), AnimationSource::Still(still), None);
        let _ = &island;
        let mut controller = Self {
            reducer: SessionReducer::new(ready_hold_ms, busy_stall_ms),
            assets,
            reduced_motion,
            state,
            animation,
            current,
            frame_deadline,
            frame_interval_ms,
            present_cost_ms: 0,
            island,
            spring: None,
            spring_last_ms: 0,
            manually_expanded: false,
            hover_expanded: false,
            face_frames: Vec::new(),
            face_delays: Vec::new(),
            face_idx: 0,
            face_decoder: None,
            face_deadline: None,
            face_frame,
            icons: Vec::new(),
            icon_hits: Vec::new(),
            hover_point: None,
            media: None,
            glass_cache: None,
        };
        if controller.island.is_enabled() {
            controller.refresh_face(state);
            let w = controller.target_width(state);
            controller.current = controller.render_island(state, w);
            controller.arm_face_deadline(0);
        } else {
            let _ = controller.load_animation_classic(VisualState::Idle, 0);
        }
        controller
    }

    fn geom(config: &IslandConfig, width: u32) -> IslandGeometry {
        IslandGeometry {
            width,
            height: config.height,
            radius: config.corner_radius,
            attached: config.is_attached(),
            y_offset: config.y_offset,
        }
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
        self.face_frame = fallback_frame(state, FACE_SIZE);
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
        // Fill one more loop frame per tick (one WIC decode ≈ 1ms).
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
        let w = self.current.width;
        self.current = self.render_island(self.state, w);
        self.animation = AnimationSource::Still(self.current.clone());
    }

    /// Renders one island frame for `state`. The frosted-glass layer is
    /// cached per geometry; content elements (face, task icons, session
    /// dots) are **evenly distributed** across the pill width instead of
    /// clumping left, and icon hit-rects are recorded for click activation.
    fn render_island(&mut self, state: VisualState, width: u32) -> FrameBuffer {
        use crate::animation::notch::Presentation;

        let island = self.island.clone();
        let presentation = self.presentation();
        let mut frame = self.glass_layer(
            width,
            island.height,
            island.corner_radius,
            island.is_attached(),
        );
        let cy = (island.height / 2) as i32;
        let pad = 16i32;
        let accent = crate::system::accent_color_bgra();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        // 1. Collect the elements to lay out.
        enum Elem {
            Face,
            Media,
            Icon { index: usize, hwnd: isize },
            Dots { count: usize },
        }
        let mut elements: Vec<Elem> = Vec::new();
        if island.has_widget("face") {
            elements.push(Elem::Face);
        }
        if island.has_widget("music") && self.media_playing() {
            elements.push(Elem::Media);
        }
        if island.show_tasks && island.has_widget("tasks") && presentation != Presentation::Minimal
        {
            let max = island.max_thumbnails as usize;
            for (index, icon) in self.icons.iter().enumerate().take(max) {
                elements.push(Elem::Icon {
                    index,
                    hwnd: icon.hwnd,
                });
            }
        }
        let dots = if island.has_widget("agents") && presentation != Presentation::Minimal {
            self.reducer.session_count().min(6)
        } else {
            0
        };
        if dots > 0 {
            elements.push(Elem::Dots { count: dots });
        }

        // 2. Even distribution: equal gaps between and around elements.
        let elem_w = |e: &Elem| -> i32 {
            match e {
                Elem::Face => (island.height.saturating_sub(12)).min(48) as i32,
                Elem::Media => 44, // artwork 24 + gap + three 3px bars
                Elem::Icon { .. } => crate::animation::notch::ICON_PX as i32,
                Elem::Dots { count } => (*count as i32 * 6) + (*count as i32 - 1) * 4,
            }
        };
        let content: i32 = elements.iter().map(&elem_w).sum();
        let gaps = elements.len().saturating_sub(1) as i32;
        let free = width as i32 - 2 * pad - content;
        let gap = if gaps > 0 {
            (free / gaps).clamp(8, 48)
        } else {
            0
        };
        let total = content + gap * gaps;
        let mut x = (width as i32 - total) / 2;

        // 3. Draw, recording icon hit-rects and hover highlight.
        self.icon_hits.clear();
        for e in &elements {
            match e {
                Elem::Face => {
                    let size = elem_w(e);
                    crate::animation::notch::blit_scaled(
                        &mut frame,
                        &self.face_frame,
                        x,
                        cy - size / 2,
                        size as u32,
                        size as u32,
                    );
                }
                Elem::Media => {
                    // Artwork when the source exposes it, else an accent disc.
                    if let Some(thumb) = self.media.as_ref().and_then(|m| m.thumbnail.as_ref()) {
                        crate::animation::notch::blit_scaled(
                            &mut frame,
                            &crate::animation::FrameBuffer {
                                width: thumb.width,
                                height: thumb.height,
                                pixels_pbgra: thumb.pixels_pbgra.clone(),
                                delay_ms: 0,
                                loop_index: 0,
                            },
                            x,
                            cy - 12,
                            24,
                            24,
                        );
                    } else {
                        crate::animation::notch::draw_disc(
                            &mut frame,
                            x + 12,
                            cy,
                            12,
                            [accent[0], accent[1], accent[2], 255],
                        );
                    }
                    // Three equalizer bars oscillating with the wall clock.
                    let bx = x + 28;
                    let base = cy + 8;
                    for (i, &phase) in [0u64, 170, 340].iter().enumerate() {
                        let t = ((now.saturating_add(phase) % 900) as f32 / 900.0)
                            * std::f32::consts::TAU;
                        let h = (4.0 + 8.0 * (t + i as f32).sin().abs()).round() as i32;
                        crate::animation::notch::fill_rect_pub(
                            &mut frame,
                            bx + i as i32 * 5,
                            base - h,
                            3,
                            h as u32,
                            accent,
                        );
                    }
                    // hwnd sentinel 0: the media element toggles playback.
                    self.icon_hits.push((0, x, cy - 12, 44, 24));
                }
                Elem::Icon { index, hwnd } => {
                    let size = elem_w(e);
                    let hovered = self.hover_point.is_some_and(|(hx, hy)| {
                        hx >= x && hx < x + size && hy >= cy - size / 2 && hy < cy + size / 2
                    });
                    crate::animation::notch::blit_icon(
                        &mut frame,
                        &self.icons[*index],
                        x,
                        cy - size / 2,
                        hovered,
                        crate::system::accent_color_bgra(),
                    );
                    self.icon_hits
                        .push((*hwnd, x, cy - size / 2, size as u32, size as u32));
                }
                Elem::Dots { count } => {
                    let (dot, _) = crate::animation::notch::accent_colors(state);
                    let mut dx = x;
                    for _ in 0..*count {
                        crate::animation::notch::draw_disc(
                            &mut frame,
                            dx + 3,
                            cy,
                            3,
                            [dot[0], dot[1], dot[2], dot[3]],
                        );
                        dx += 10;
                    }
                }
            }
            x += elem_w(e) + gap;
        }

        // 4. Accent strip: agent state color; accent color while media plays.
        let (state_color, _) = crate::animation::notch::accent_colors(state);
        let strip = if state != VisualState::Idle {
            state_color
        } else if self.media_playing() && island.has_widget("music") {
            crate::system::accent_color_bgra()
        } else {
            return frame;
        };
        crate::animation::notch::draw_accent_strip(
            &mut frame,
            island.is_attached(),
            island.corner_radius,
            strip,
        );
        frame
    }

    /// Cached frosted-glass layer, keyed by geometry. Theme/material changes
    /// clear it via [`Controller::set_island_config`].
    fn glass_layer(&mut self, w: u32, h: u32, r: u32, attached: bool) -> FrameBuffer {
        let key = (w, h, r, attached);
        if let Some((cached_key, buf)) = &self.glass_cache {
            if *cached_key == key {
                return buf.clone();
            }
        }
        let buf = crate::animation::notch::glass_layer(w, h, r, attached, &self.island.glass);
        self.glass_cache = Some((key, buf.clone()));
        buf
    }

    /// Whether the idle pill is currently expanded (manual toggle or hover).
    fn is_expanded_idle(&self) -> bool {
        self.manually_expanded || self.hover_expanded
    }

    /// The iOS presentation the pill should rest in right now:
    /// Expanded while hovered/pinned, Compact while an agent session or
    /// media is live, Minimal when nothing is live.
    fn presentation(&self) -> crate::animation::notch::Presentation {
        use crate::animation::notch::Presentation;
        if self.is_expanded_idle() {
            return Presentation::Expanded;
        }
        let media_live = self.media.as_ref().is_some_and(|m| m.playing);
        let agent_live = !matches!(self.state, VisualState::Idle);
        if agent_live || media_live {
            return Presentation::Compact;
        }
        Presentation::Minimal
    }

    /// Whether the pill currently shows the wide dashboard layout (icons,
    /// agent usage, media) rather than the compact face+text layout.
    fn dashboard_expanded(&self) -> bool {
        if !self.island.is_enabled() || self.spring.is_some() {
            return false;
        }
        let agent_active = !matches!(self.state, VisualState::Idle | VisualState::Failed);
        agent_active || self.is_expanded_idle()
    }

    /// Whether the teal media strip shows (widget on + something playing).
    fn media_playing(&self) -> bool {
        self.island.has_widget("music") && self.media.as_ref().is_some_and(|m| m.playing)
    }

    /// Stores a background worker round (task icons + media state).
    /// Returns true when the visible frame should repaint.
    pub fn set_task_update(&mut self, update: WorkerUpdate) -> bool {
        self.icons = update.icons;
        self.media = update.media;
        if !self.island.is_enabled() || !self.island.show_tasks {
            return false;
        }
        if self.spring.is_some() {
            // A morph is in flight; its end frame will pick up the new
            // content. Don't stomp the interpolated frame.
            return false;
        }
        if !self.dashboard_expanded() {
            // Collapsed pills still refresh for a visible ring or the media
            // strip, which change without any event or tick.
            if !self.media_playing() {
                return false;
            }
        }
        if !self.island.has_widget("tasks") && !self.media_playing() {
            return false;
        }
        let w = self.current.width;
        self.current = self.render_island(self.state, w);
        self.animation = AnimationSource::Still(self.current.clone());
        true
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
                let w = self.target_width(self.state);
                self.current = self.render_island(self.state, w);
                self.animation = AnimationSource::Still(self.current.clone());
                self.frame_deadline = None;
                self.arm_face_deadline(now_ms);
            } else {
                let _ = self.load_animation_classic(self.state, now_ms);
            }
        } else if is_enabled {
            let w = self.target_width(self.state);
            self.current = self.render_island(self.state, w);
            self.animation = AnimationSource::Still(self.current.clone());
            self.arm_face_deadline(now_ms);
        }
    }

    fn target_width(&self, _state: VisualState) -> u32 {
        match self.presentation() {
            crate::animation::notch::Presentation::Expanded => self.island.expanded_width,
            crate::animation::notch::Presentation::Compact => self.island.collapsed_width,
            crate::animation::notch::Presentation::Minimal => self.island.minimal_width,
        }
    }

    /// The active island configuration.
    pub fn island_config(&self) -> &IslandConfig {
        &self.island
    }

    /// Routes a click at frame-local (`x`, `y`):
    /// - over a task icon: returns the window handle to activate;
    /// - over the pill otherwise: toggles expansion and returns the outcome.
    pub fn handle_click(&mut self, x: i32, y: i32, now_ms: u64) -> ClickOutcome {
        for (hwnd, hx, hy, hw, hh) in &self.icon_hits {
            if x >= *hx && x < hx + *hw as i32 && y >= *hy && y < hy + *hh as i32 {
                return ClickOutcome::ActivateWindow(*hwnd);
            }
        }
        if self.manually_expanded || self.hover_expanded {
            // Collapse on any non-icon click while open (iOS behavior).
            self.manually_expanded = false;
            self.hover_expanded = false;
            let _ = self.morph_to_target(now_ms);
            return ClickOutcome::Collapsed;
        }
        if self.toggle_expand(now_ms) {
            return ClickOutcome::Expanded;
        }
        ClickOutcome::None
    }

    /// Stores the cursor position in frame coordinates for icon hover
    /// highlighting. Triggers a repaint only when the highlighted icon
    /// changed.
    pub fn set_hover_point(&mut self, point: Option<(i32, i32)>) -> bool {
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
        if !self.island.is_enabled() || self.state != VisualState::Idle {
            return false;
        }
        self.manually_expanded = !self.manually_expanded;
        // A click takes control from hover: without this the pill could never
        // collapse by click while the cursor is still inside it.
        self.hover_expanded = false;
        self.morph_to_target(now_ms)
    }

    /// Tracks the cursor entering (`inside = true`) or leaving the pill.
    /// Hover expands the idle pill when `expand_on_hover` is set; leaving
    /// collapses it again unless it was manually toggled open.
    pub fn set_hover(&mut self, inside: bool, now_ms: u64) -> bool {
        if !self.island.is_enabled() || !self.island.expand_on_hover {
            return false;
        }
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

    /// Morphs (or snaps, under reduced motion) to the current target width
    /// using the iOS spring model. Interrupted morphs inherit the current
    /// velocity, exactly like the Dynamic Island.
    fn morph_to_target(&mut self, now_ms: u64) -> bool {
        let target_w = self.target_width(self.state);
        if self.reduced_motion {
            self.spring = None;
            self.current = self.render_island(self.state, target_w);
            self.animation = AnimationSource::Still(self.current.clone());
            self.frame_deadline = None;
            return true;
        }
        let (from_w, vel) = match self.spring.take() {
            Some(s) => (s.x, s.v),
            None => (self.current.width as f32, 0.0),
        };
        if (from_w - target_w as f32).abs() < 1.0 && vel.abs() < 1.0 {
            self.spring = None;
            self.current = self.render_island(self.state, target_w);
            self.animation = AnimationSource::Still(self.current.clone());
            self.frame_deadline = None;
            return true;
        }
        let params = spring_params(self.island.animation_ms, self.island.spring_bounce);
        self.spring = Some(Spring::new(from_w.round() as u32, target_w, params));
        self.spring.as_mut().unwrap().v = vel;
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
        self.reducer.advance(now_ms);
        self.reducer.apply(event);
        let mut actions = self.sync_state(now_ms);
        actions.next_deadline_ms = self.next_deadline_ms();
        actions
    }

    /// Fires every reducer and animation deadline due at `now_ms`.
    pub fn on_timer(&mut self, now_ms: u64) -> ControllerActions {
        self.reducer.advance(now_ms);
        let mut actions = self.sync_state(now_ms);

        // NOTE: dashboard content arrives from the background worker via
        // `set_task_update`; nothing here may block on window capture or
        // media queries, otherwise hovering and morphing visibly stall.

        // Spring morph tick: integrate the width toward the target with the
        // iOS spring model. While morphing, intermediate frames render the
        // cheap glass+glyph path; the full dashboard appears when settled.
        if self.island.is_enabled() {
            if let Some(spring) = self.spring.as_mut() {
                // Integrate the real elapsed time in ≤16ms sub-steps: explicit
                // Euler is only stable for small dt, and a late tick (or a
                // test jumping 100ms) would otherwise diverge.
                let mut elapsed = now_ms.saturating_sub(self.spring_last_ms).min(200);
                self.spring_last_ms = now_ms;
                let omega = spring.stiffness.sqrt().max(1.0);
                let sub_ms = ((0.35 / omega) * 1000.0).ceil().max(2.0) as u64;
                let mut w = spring.x.round() as u32;
                let mut settled = false;
                while elapsed > 0 && !settled {
                    let dt = (elapsed.min(sub_ms) as f32 / 1000.0).max(0.0005);
                    elapsed = elapsed.saturating_sub(sub_ms);
                    let (nw, ns) = spring.step(dt);
                    w = nw;
                    settled = ns;
                }
                let target_reached = settled || w == spring.target.round() as u32;
                if target_reached {
                    let target_w = spring.target.round() as u32;
                    self.spring = None;
                    self.current = self.render_island(self.state, target_w);
                    self.animation = AnimationSource::Still(self.current.clone());
                    self.frame_deadline = None;
                    self.arm_face_deadline(now_ms);
                } else {
                    let geom = Self::geom(&self.island, w);
                    self.current = island_frame(self.state, geom, &self.island.glass, 0.5);
                    self.animation = AnimationSource::Still(self.current.clone());
                    let interval = self.frame_interval_ms.unwrap_or(16);
                    let next = now_ms
                        .saturating_add(interval)
                        .saturating_sub(self.present_cost_ms.min(interval.saturating_sub(1)));
                    self.frame_deadline = Some(next);
                }
                actions.present_frame = true;
            }
        }

        // Face-animation tick: cheap (pre-decoded frames + cached icons),
        // paused while a morph is in flight so it stays smooth.
        if self.island.is_enabled() && self.spring.is_none() {
            if let Some(face_at) = self.face_deadline {
                if now_ms >= face_at {
                    self.advance_face(now_ms);
                    actions.present_frame = true;
                }
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

    /// The earliest moment [`Controller::handle_event`] or [`Controller::on_timer`]
    /// can change anything, or `None` when no deadline is pending.
    pub fn next_deadline_ms(&self) -> Option<u64> {
        let earliest = match (self.reducer.next_deadline_ms(), self.frame_deadline) {
            (Some(reducer_at), Some(frame_at)) => Some(reducer_at.min(frame_at)),
            (reducer_at, frame_at) => reducer_at.or(frame_at),
        };
        match (earliest, self.face_deadline) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
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
            let w = self.target_width(self.state);
            self.spring = None;
            self.current = self.render_island(self.state, w);
            self.animation = AnimationSource::Still(self.current.clone());
            self.frame_deadline = None;
            self.arm_face_deadline(0);
        } else {
            let still = fallback_frame(self.state, FALLBACK_FRAME_SIZE);
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
            actions.visible_state = Some(state);
            actions.present_frame = true;
            if state != VisualState::Idle {
                self.manually_expanded = false;
            }
            if self.island.is_enabled() {
                self.refresh_face(state);
                let target_w = self.target_width(state);
                let current_w = self.current.width;
                if current_w == target_w || self.reduced_motion {
                    self.current = self.render_island(state, target_w);
                    self.animation = AnimationSource::Still(self.current.clone());
                    self.spring = None;
                    self.frame_deadline = None;
                    self.arm_face_deadline(now_ms);
                } else {
                    let params = spring_params(self.island.animation_ms, self.island.spring_bounce);
                    self.spring = Some(Spring::new(current_w, target_w, params));
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
        }
        actions
    }

    /// Loads the animation for `state` (classic pet path only).
    fn load_animation_classic(&mut self, state: VisualState, now_ms: u64) -> Option<i32> {
        self.frame_deadline = None;
        self.current = fallback_frame(state, FALLBACK_FRAME_SIZE);

        let path = self.assets.resolve(state)?;
        let mut gif = match GifAnimation::open(&path) {
            Ok(gif) => gif,
            Err(error) => {
                self.animation = AnimationSource::Still(fallback_frame(state, FALLBACK_FRAME_SIZE));
                return Some(error_code(&error));
            }
        };

        match gif.next_frame() {
            Ok(frame) => {
                self.current = frame.clone();
                if self.reduced_motion {
                    self.animation = AnimationSource::Still(frame.clone());
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
                self.animation = AnimationSource::Still(fallback_frame(state, FALLBACK_FRAME_SIZE));
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
        let gif = match &mut self.animation {
            AnimationSource::Gif(gif) => gif,
            AnimationSource::Still(_) => {
                self.frame_deadline = None;
                return None;
            }
        };

        match gif.next_frame() {
            Ok(frame) => {
                self.current = frame.clone();
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
