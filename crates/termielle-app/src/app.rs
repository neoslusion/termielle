//! The controller: folds pipe events and timer ticks into window actions.
//!
//! [`Controller`] owns the reducer, the active animation, and the single
//! deadline the window's timer is armed for. It never touches Win32 itself;
//! [`main`](crate::main) maps its actions onto the window, the pipe, and the
//! log.

use crate::animation::notch::BRIDGE_K_MAX;
use crate::animation::{
    AnimationError, AnimationSource, FrameBuffer, GifAnimation, fallback_frame,
};
use crate::tasks::{MediaInfo, WorkerUpdate};
use termielle_core::{
    AssetCatalog, EventMessage, IslandConfig, SessionReducer, VisualState, spring_params,
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
    /// The dashboard opened.
    Expanded,
    /// The dashboard closed.
    Collapsed,
    /// Click landed outside any interactive element.
    None,
}

/// The iOS Dynamic Island morphs between presentations with a spring, not a
/// timing curve — interrupted morphs keep their velocity and settle
/// naturally. This integrator tracks pill width, height, and corner radius
/// in px and velocity (px/s): the radius rides the same spring so the
/// silhouette morphs continuously instead of snapping between the pill and
/// the expanded card.
#[derive(Clone, Copy, Debug)]
struct Spring2D {
    /// Current animated width.
    x: f32,
    /// Current animated height.
    y: f32,
    /// Current animated corner radius.
    z: f32,
    /// Current width velocity in px/s.
    vx: f32,
    /// Current height velocity in px/s.
    vy: f32,
    /// Current radius velocity in px/s.
    vz: f32,
    /// Target width.
    target_x: f32,
    /// Target height.
    target_y: f32,
    /// Target corner radius.
    target_z: f32,
    /// Where this morph started, so interrupted morphs still report progress.
    start_x: f32,
    start_y: f32,
    start_z: f32,
    /// Params from `spring_params(animation_ms, spring_bounce)`.
    stiffness: f32,
    damping: f32,
}

/// Below these the spring is considered settled and snaps to target.
const SETTLE_PX: f32 = 0.5;
const SETTLE_V: f32 = 2.0;

impl Spring2D {
    fn new(
        from_w: u32,
        from_h: u32,
        from_r: f32,
        target_w: u32,
        target_h: u32,
        target_r: f32,
        params: termielle_core::SpringParams,
    ) -> Self {
        Self {
            x: from_w as f32,
            y: from_h as f32,
            z: from_r,
            vx: 0.0,
            vy: 0.0,
            vz: 0.0,
            target_x: target_w as f32,
            target_y: target_h as f32,
            target_z: target_r,
            start_x: from_w as f32,
            start_y: from_h as f32,
            start_z: from_r,
            stiffness: params.stiffness,
            damping: params.damping,
        }
    }

    /// Morph progress toward the target, 0-1, from the remaining
    /// displacement fraction. Content fade/slide derives from this.
    fn progress(&self) -> f32 {
        let total = (self.start_x - self.target_x)
            .abs()
            .max((self.start_y - self.target_y).abs())
            .max((self.start_z - self.target_z).abs());
        if total < 1.0 {
            return 1.0;
        }
        let remaining = (self.x - self.target_x)
            .abs()
            .max((self.y - self.target_y).abs())
            .max((self.z - self.target_z).abs());
        (1.0 - remaining / total).clamp(0.0, 1.0)
    }

    /// Integrates one step of `dt` seconds. Returns ((width, height, radius), settled).
    fn step(&mut self, dt: f32) -> ((u32, u32, u32), bool) {
        let accel_x = -self.stiffness * (self.x - self.target_x) - self.damping * self.vx;
        self.vx += accel_x * dt;
        self.x += self.vx * dt;

        let accel_y = -self.stiffness * (self.y - self.target_y) - self.damping * self.vy;
        self.vy += accel_y * dt;
        self.y += self.vy * dt;

        let accel_z = -self.stiffness * (self.z - self.target_z) - self.damping * self.vz;
        self.vz += accel_z * dt;
        self.z += self.vz * dt;

        let settled_x = (self.x - self.target_x).abs() < SETTLE_PX && self.vx.abs() < SETTLE_V;
        let settled_y = (self.y - self.target_y).abs() < SETTLE_PX && self.vy.abs() < SETTLE_V;
        let settled_z = (self.z - self.target_z).abs() < SETTLE_PX && self.vz.abs() < SETTLE_V;

        if settled_x {
            self.x = self.target_x;
            self.vx = 0.0;
        }
        if settled_y {
            self.y = self.target_y;
            self.vy = 0.0;
        }
        if settled_z {
            self.z = self.target_z;
            self.vz = 0.0;
        }

        let settled = settled_x && settled_y && settled_z;
        (
            (
                self.x.round().max(1.0) as u32,
                self.y.round().max(1.0) as u32,
                self.z.round().max(1.0) as u32,
            ),
            settled,
        )
    }
}

/// 1-D spring for the blob separation in pixels: 0 is the merged single
/// pill, [`BLOB_GAP_PX`] is the fully split island. It runs alongside the
/// size spring so the split and the stretch feel like one motion.
#[derive(Clone, Copy, Debug, Default)]
struct Spring1 {
    x: f32,
    v: f32,
    target: f32,
    stiffness: f32,
    damping: f32,
}

impl Spring1 {
    fn new(from: f32, target: f32, params: termielle_core::SpringParams) -> Self {
        Self {
            x: from,
            v: 0.0,
            target,
            stiffness: params.stiffness,
            damping: params.damping,
        }
    }

    fn step(&mut self, dt: f32) {
        let accel = -self.stiffness * (self.x - self.target) - self.damping * self.v;
        self.v += accel * dt;
        self.x += self.v * dt;
        if (self.x - self.target).abs() < SETTLE_PX && self.v.abs() < SETTLE_V {
            self.x = self.target;
            self.v = 0.0;
        }
    }

    fn settled(&self) -> bool {
        (self.x - self.target).abs() < SETTLE_PX && self.v.abs() < SETTLE_V
    }
}

/// Gap between the two blobs of a split island, in pixels — the Dynamic
/// Island keeps its two live activities close, joined earlier by the
/// liquid bridge.
const BLOB_GAP_PX: f32 = 9.0;

/// Cache key for the frosted-glass layer: geometry, material, and blob layout.
type GlassCacheKey = (u32, u32, u32, bool, bool, u32, i32, u32);

/// Transient alert banner displayed in the Dynamic Island on notifications.
#[derive(Clone, Debug)]
pub struct AlertBanner {
    pub title: String,
    pub subtitle: String,
    pub accent: [u8; 4],
    pub expires_at_ms: u64,
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
    /// Transient notification alert banner.
    alert: Option<AlertBanner>,
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
        let radius = island.height as f32 / 2.0;
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
            alert: None,
        };
        if controller.island.is_enabled() {
            controller.refresh_face(state);
            let (w, h) = controller.target_size(state);
            controller.current = controller.render_island(state, w, h);
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
        let (w, h) = (self.current.width, self.current.height);
        self.current = self.render_island(self.state, w, h);
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
    fn render_island(&mut self, state: VisualState, width: u32, height: u32) -> FrameBuffer {
        use crate::animation::notch::Presentation;

        let presentation = self.presentation();
        if presentation == Presentation::Hidden || height <= 4 {
            self.icon_hits.clear();
            return FrameBuffer {
                width,
                height,
                pixels_pbgra: vec![0; (width * height * 4) as usize],
                delay_ms: 0,
                loop_index: 0,
            };
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
        let mut content = FrameBuffer {
            width,
            height,
            pixels_pbgra: vec![0; (width * height * 4) as usize],
            delay_ms: 0,
            loop_index: 0,
        };
        self.render_content(
            &mut content,
            state,
            &island,
            presentation,
            width,
            height,
            &blobs,
        );
        let (alpha, dy) = Self::content_motion(self.spring.as_ref(), presentation);
        crate::animation::notch::blend_frame_over(&mut frame, &content, dy, alpha);

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
        crate::animation::notch::draw_accent_strip(&mut frame, attached, radius, strip);
        frame
    }

    /// Content motion during a morph: a slight opacity dip while the
    /// container is at its fastest (mid-spring) and a small rise into
    /// place when the target is the expanded card. Settled frames get
    /// full opacity.
    fn content_motion(
        spring: Option<&Spring2D>,
        presentation: crate::animation::notch::Presentation,
    ) -> (u8, i32) {
        use crate::animation::notch::Presentation;
        let Some(spring) = spring else {
            return (255, 0);
        };
        let p = spring.progress();
        let dip = (1.0 - 0.45 * (p * std::f32::consts::PI).sin()).clamp(0.0, 1.0);
        let alpha = (255.0 * dip).round() as u8;
        let dy = match presentation {
            Presentation::Expanded => ((1.0 - p) * 6.0).round() as i32,
            _ => 0,
        };
        (alpha, dy)
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
    ) {
        use crate::animation::notch::Presentation;
        let cy = (height / 2) as i32;
        let accent = crate::system::accent_color_bgra();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        // Render according to the active iOS/macOS presentation class.
        self.icon_hits.clear();

        // 0. Active Notification Alert Banner
        if let Some(alert) = &self.alert {
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
                    "SYSTEM NOTIFICATION",
                    tag_x,
                    15,
                    width.saturating_sub((tag_x as u32) + 40),
                    9,
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
                    [255, 255, 255, 255],
                );
                crate::animation::notch::draw_text(
                    frame,
                    &alert.subtitle,
                    pad,
                    76,
                    text_w,
                    12,
                    false,
                    [185, 190, 205, 220],
                );

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

            return;
        }

        match presentation {
            Presentation::Hidden => {}
            Presentation::Minimal => {
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
                        let t = ((now.saturating_add(phase) % 800) as f32 / 800.0)
                            * std::f32::consts::TAU;
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
            Presentation::Compact => {
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
                    }
                    if state != VisualState::Idle || self.reducer.session_count() > 0 {
                        let (dot, _) = crate::animation::notch::accent_colors(state);
                        let count = if island.has_widget("agents") {
                            self.reducer.session_count().clamp(1, 4)
                        } else {
                            1
                        };
                        let mut dot_x = primary.x + primary.w as i32 - 16;
                        for _ in 0..count {
                            crate::animation::notch::draw_disc(frame, dot_x, cy, 3, dot);
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
                        let t = ((now.saturating_add(phase) % 800) as f32 / 800.0)
                            * std::f32::consts::TAU;
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
                    self.icon_hits.push((
                        HIT_MEDIA_PLAY_PAUSE,
                        media_blob.x,
                        0,
                        media_blob.w,
                        height,
                    ));
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
                    }

                    let mut right_cursor = width as i32 - 14;

                    // Media equalizer, with the album art leading it
                    if self.media_playing() && island.has_widget("music") {
                        let bx = right_cursor - 14;
                        let art_size = (height.saturating_sub(12)).min(22) as i32;
                        let art_x = bx - art_size - 6;
                        if let Some(thumb) = self.media.as_ref().and_then(|m| m.thumbnail.as_ref())
                        {
                            crate::animation::notch::blit_rounded(
                                frame,
                                &crate::animation::FrameBuffer {
                                    width: thumb.width,
                                    height: thumb.height,
                                    pixels_pbgra: thumb.pixels_pbgra.clone(),
                                    delay_ms: 0,
                                    loop_index: 0,
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
                            let t = ((now.saturating_add(phase) % 800) as f32 / 800.0)
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
                        for _ in 0..count {
                            right_cursor -= 10;
                            crate::animation::notch::draw_disc(
                                frame,
                                right_cursor + 4,
                                cy,
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
            Presentation::Expanded => {
                let pad = 18i32;

                if height >= 85 {
                    // Authentic tall card layout dropping vertically downward
                    // 1. Top Header Row
                    if island.has_widget("face") {
                        let face_size = 22i32;
                        let fx = pad;
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
                        pad,
                        40,
                        width.saturating_sub((pad as u32) * 2),
                        1,
                        [255, 255, 255, 22],
                    );

                    if self.media_playing() && island.has_widget("music") {
                        // Header title & mini equalizer in header row
                        let tag_x = if island.has_widget("face") {
                            pad + 30
                        } else {
                            pad
                        };
                        crate::animation::notch::draw_text(
                            frame,
                            "NOW PLAYING",
                            tag_x,
                            16,
                            width.saturating_sub((tag_x as u32) + 50),
                            9,
                            true,
                            accent,
                        );

                        // 4-bar mini equalizer in top right
                        let eq_x = width as i32 - pad - 20;
                        for (i, &phase) in [0u64, 180, 360, 540].iter().enumerate() {
                            let t = ((now.saturating_add(phase) % 700) as f32 / 700.0)
                                * std::f32::consts::TAU;
                            let h = (3.0 + 8.0 * (t + i as f32).sin().abs()).round() as i32;
                            crate::animation::notch::fill_rect_pub(
                                frame,
                                eq_x + i as i32 * 5,
                                26 - h,
                                3,
                                h as u32,
                                accent,
                            );
                        }

                        // Prominent Media Body Card:
                        // High-res 64x64 Album Artwork
                        let art_size = 64i32;
                        let art_x = pad;
                        let art_y = 48i32;

                        if let Some(thumb) = self.media.as_ref().and_then(|m| m.thumbnail.as_ref())
                        {
                            crate::animation::notch::blit_rounded(
                                frame,
                                &crate::animation::FrameBuffer {
                                    width: thumb.width,
                                    height: thumb.height,
                                    pixels_pbgra: thumb.pixels_pbgra.clone(),
                                    delay_ms: 0,
                                    loop_index: 0,
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
                                [accent[0], accent[1], accent[2], 220],
                            );
                        }

                        // Track title, artist, and app
                        let text_x = art_x + art_size + 14;
                        let text_w = width.saturating_sub((text_x as u32) + (pad as u32));
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

                        crate::animation::notch::draw_text(
                            frame,
                            title,
                            text_x,
                            50,
                            text_w,
                            14,
                            true,
                            [255, 255, 255, 255],
                        );
                        crate::animation::notch::draw_text(
                            frame,
                            artist,
                            text_x,
                            74,
                            text_w,
                            11,
                            false,
                            [175, 180, 195, 220],
                        );

                        let app_name = self.media.as_ref().map(|m| m.app.as_str()).unwrap_or("");
                        if !app_name.is_empty() {
                            crate::animation::notch::draw_text(
                                frame,
                                app_name,
                                text_x,
                                94,
                                text_w,
                                9,
                                false,
                                [accent[0], accent[1], accent[2], 210],
                            );
                        }

                        // Progress / timeline bar
                        if height >= 145 {
                            let track_y = 120i32;
                            let track_w = width.saturating_sub((pad as u32) * 2);
                            crate::animation::notch::fill_rect_pub(
                                frame,
                                pad,
                                track_y,
                                track_w,
                                3,
                                [255, 255, 255, 30],
                            );
                            crate::animation::notch::fill_rect_pub(
                                frame,
                                pad,
                                track_y,
                                (track_w as f32 * 0.45) as u32,
                                3,
                                accent,
                            );
                        }

                        // Dedicated Media Controls Row: Prev (⏮), Play/Pause (⏯), Next (⏭)
                        if height >= 165 {
                            let ctrl_y = 148i32;
                            let center_x = width as i32 / 2;

                            // Previous Track Button
                            let prev_x = center_x - 56;
                            let prev_hover = self.hover_point.is_some_and(|(px, py)| {
                                (px - prev_x).pow(2) + (py - ctrl_y).pow(2) <= 16 * 16
                            });
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
                            crate::animation::notch::draw_glyph_prev(
                                frame,
                                prev_x,
                                ctrl_y,
                                [255, 255, 255, 240],
                            );
                            self.icon_hits
                                .push((HIT_MEDIA_PREV, prev_x - 16, ctrl_y - 16, 32, 32));

                            // Play / Pause Button
                            let play_hover = self.hover_point.is_some_and(|(px, py)| {
                                (px - center_x).pow(2) + (py - ctrl_y).pow(2) <= 19 * 19
                            });
                            let play_bg = if play_hover {
                                [accent[0], accent[1], accent[2], 255]
                            } else {
                                [accent[0], accent[1], accent[2], 210]
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
                                crate::animation::notch::draw_glyph_pause(
                                    frame,
                                    center_x,
                                    ctrl_y,
                                    [255, 255, 255, 255],
                                );
                            } else {
                                crate::animation::notch::draw_glyph_play(
                                    frame,
                                    center_x,
                                    ctrl_y,
                                    [255, 255, 255, 255],
                                );
                            }
                            self.icon_hits.push((
                                HIT_MEDIA_PLAY_PAUSE,
                                center_x - 18,
                                ctrl_y - 18,
                                36,
                                36,
                            ));

                            // Next Track Button
                            let next_x = center_x + 56;
                            let next_hover = self.hover_point.is_some_and(|(px, py)| {
                                (px - next_x).pow(2) + (py - ctrl_y).pow(2) <= 16 * 16
                            });
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
                            crate::animation::notch::draw_glyph_next(
                                frame,
                                next_x,
                                ctrl_y,
                                [255, 255, 255, 240],
                            );
                            self.icon_hits
                                .push((HIT_MEDIA_NEXT, next_x - 16, ctrl_y - 16, 32, 32));
                        }
                    } else if state != VisualState::Idle {
                        // Agent Live Activity (e.g. Claude, Codex, Agy, OpenCode is running)
                        let primary = self.reducer.primary_session();
                        let agent_source = primary
                            .as_ref()
                            .map(|(s, _, _)| s.as_str())
                            .unwrap_or("agent");
                        let session_id =
                            primary.as_ref().map(|(_, id, _)| id.as_str()).unwrap_or("");
                        let (sc, _) = crate::animation::notch::accent_colors(state);

                        let tag_x = if island.has_widget("face") {
                            pad + 30
                        } else {
                            pad
                        };
                        let header_title =
                            format!("LIVE ACTIVITY • {}", agent_source.to_uppercase());
                        crate::animation::notch::draw_text(
                            frame,
                            &header_title,
                            tag_x,
                            16,
                            width.saturating_sub((tag_x as u32) + 50),
                            9,
                            true,
                            [sc[0], sc[1], sc[2], 255],
                        );

                        // Session count dots top right
                        let sessions = self.reducer.session_count().clamp(1, 4);
                        for i in 0..sessions {
                            crate::animation::notch::draw_disc(
                                frame,
                                width as i32 - pad - (i as i32 * 10) - 4,
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
                            VisualState::Working => {
                                ("Executing Actions", "Running autonomous tools and edits...")
                            }
                            VisualState::NeedsInput => {
                                ("Action Required", "Waiting for confirmation or input")
                            }
                            VisualState::Ready => ("Turn Complete", "Task finished successfully!"),
                            VisualState::Failed => ("Turn Failed", "Execution stopped with error"),
                        };

                        // Glowing state beacon disc
                        let beacon_x = pad + 16;
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

                        let text_x = beacon_x + 24;
                        let text_w = width.saturating_sub((text_x as u32) + (pad as u32));
                        crate::animation::notch::draw_text(
                            frame,
                            state_title,
                            text_x,
                            52,
                            text_w,
                            15,
                            true,
                            [255, 255, 255, 255],
                        );
                        crate::animation::notch::draw_text(
                            frame,
                            state_detail,
                            text_x,
                            76,
                            text_w,
                            11,
                            false,
                            [175, 180, 195, 210],
                        );

                        // Session ID badge pill
                        if !session_id.is_empty() && height >= 140 {
                            let badge_text =
                                format!("Session: {}", &session_id[..session_id.len().min(26)]);
                            let bw = (badge_text.len() * 6 + 18) as u32;
                            crate::animation::notch::fill_rect_pub(
                                frame,
                                text_x,
                                98,
                                bw,
                                18,
                                [255, 255, 255, 18],
                            );
                            crate::animation::notch::draw_text(
                                frame,
                                &badge_text,
                                text_x + 6,
                                101,
                                bw - 10,
                                9,
                                false,
                                [200, 215, 235, 220],
                            );
                        }

                        // Bottom accent pill bar
                        if height >= 145 {
                            let bar_w = 64u32;
                            let bar_x = (width as i32 - bar_w as i32) / 2;
                            crate::animation::notch::fill_rect_pub(
                                frame,
                                bar_x,
                                height as i32 - 12,
                                bar_w,
                                3,
                                [sc[0], sc[1], sc[2], 255],
                            );
                        }
                    } else {
                        // Idle Dashboard with Open Windows / Task Switcher
                        let tag_x = if island.has_widget("face") {
                            pad + 30
                        } else {
                            pad
                        };
                        crate::animation::notch::draw_text(
                            frame,
                            if !self.tasks.is_empty() {
                                "ACTIVE TASKS & WINDOWS"
                            } else {
                                "TERMIELLE DASHBOARD"
                            },
                            tag_x,
                            16,
                            width.saturating_sub((tag_x as u32) + 50),
                            9,
                            true,
                            accent,
                        );

                        if !self.tasks.is_empty() {
                            let tile_w = 44i32;
                            let tile_h = 44i32;
                            let gap = 12i32;
                            let count = self.tasks.len().min(5) as i32;
                            let total_w = count * tile_w + (count - 1) * gap;
                            let start_x = ((width as i32 - total_w) / 2).max(pad);
                            let tile_y = 56i32;

                            for (i, task) in self.tasks.iter().take(5).enumerate() {
                                let tx = start_x + i as i32 * (tile_w + gap);
                                let is_hover = self.hover_point.is_some_and(|(px, py)| {
                                    px >= tx
                                        && px < tx + tile_w
                                        && py >= tile_y
                                        && py < tile_y + tile_h
                                });
                                let tile_bg = if is_hover {
                                    [255, 255, 255, 40]
                                } else {
                                    [255, 255, 255, 18]
                                };
                                let tile_border = if is_hover {
                                    [accent[0], accent[1], accent[2], 220]
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
                                    },
                                    tx + 4,
                                    tile_y + 4,
                                    (tile_w - 8) as u32,
                                    (tile_h - 8) as u32,
                                    6,
                                );

                                // Hit target for window activation
                                self.icon_hits.push((
                                    task.hwnd,
                                    tx,
                                    tile_y,
                                    tile_w as u32,
                                    tile_h as u32,
                                ));
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
                                pad,
                                116,
                                width.saturating_sub((pad as u32) * 2),
                                11,
                                false,
                                [210, 220, 235, 230],
                            );
                        } else {
                            // Standard idle message
                            crate::animation::notch::draw_text(
                                frame,
                                "Termielle is Idle",
                                pad,
                                56,
                                width.saturating_sub((pad as u32) * 2),
                                15,
                                true,
                                [255, 255, 255, 255],
                            );
                            crate::animation::notch::draw_text(
                                frame,
                                "Standing by for agent instructions or media playback",
                                pad,
                                82,
                                width.saturating_sub((pad as u32) * 2),
                                11,
                                false,
                                [175, 180, 195, 210],
                            );
                        }

                        // Bottom accent pill bar
                        if height >= 145 {
                            let bar_w = 48u32;
                            let bar_x = (width as i32 - bar_w as i32) / 2;
                            crate::animation::notch::fill_rect_pub(
                                frame,
                                bar_x,
                                height as i32 - 12,
                                bar_w,
                                3,
                                [accent[0], accent[1], accent[2], 200],
                            );
                        }
                    }
                } else {
                    // Compact / transitioning view
                    let face_size = (height.saturating_sub(20)).min(36) as i32;
                    if island.has_widget("face") {
                        let fx = pad;
                        let fy = cy - face_size / 2;
                        let (sc, _) = crate::animation::notch::accent_colors(state);
                        crate::animation::notch::draw_disc(
                            frame,
                            fx + face_size / 2,
                            cy,
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
                        let media_start_x = pad + face_size + 14;
                        let art_y = cy - art_size / 2;
                        if let Some(thumb) = self.media.as_ref().and_then(|m| m.thumbnail.as_ref())
                        {
                            crate::animation::notch::blit_rounded(
                                frame,
                                &crate::animation::FrameBuffer {
                                    width: thumb.width,
                                    height: thumb.height,
                                    pixels_pbgra: thumb.pixels_pbgra.clone(),
                                    delay_ms: 0,
                                    loop_index: 0,
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
                                cy,
                                (art_size / 2) as u32,
                                [accent[0], accent[1], accent[2], 230],
                            );
                        }
                        let text_x = media_start_x + art_size + 10;
                        let text_w = (width as i32 - pad - text_x).max(40) as u32;
                        let title = self
                            .media
                            .as_ref()
                            .map(|m| m.title.as_str())
                            .unwrap_or("Playing");
                        crate::animation::notch::draw_text(
                            frame,
                            title,
                            text_x,
                            cy - 7,
                            text_w,
                            11,
                            true,
                            [255, 255, 255, 245],
                        );
                    } else {
                        let left_edge = pad + face_size + 14;
                        let text_w = (width as i32 - pad - left_edge).max(40) as u32;
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
                            cy - 7,
                            text_w,
                            12,
                            true,
                            [255, 255, 255, 240],
                        );
                    }
                }
            }
        }
    }
    /// Cached frosted-glass layer, keyed by geometry and blob layout.
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
        let key = (w, h, r, attached, black, blob_count, right_x, right_w);
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
        if self.alert.is_some() {
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
        let (w, h) = (self.current.width, self.current.height);
        self.current = self.render_island(self.state, w, h);
        self.animation = AnimationSource::Still(self.current.clone());
        true
    }

    /// Triggers a transient alert notification banner.
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
        self.alert = Some(AlertBanner {
            title: title.into(),
            subtitle: subtitle.into(),
            accent,
            expires_at_ms: now_ms.saturating_add(duration_ms),
        });
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
                self.current = self.render_island(self.state, w, h);
                self.animation = AnimationSource::Still(self.current.clone());
                self.frame_deadline = None;
                self.arm_face_deadline(now_ms);
            } else {
                let _ = self.load_animation_classic(self.state, now_ms);
            }
        } else if is_enabled {
            let (w, h) = self.target_size(self.state);
            self.current = self.render_island(self.state, w, h);
            self.animation = AnimationSource::Still(self.current.clone());
            self.arm_face_deadline(now_ms);
        }
    }

    /// Target (width, height) for the current visual and presentation state.
    pub fn target_size(&self, _state: VisualState) -> (u32, u32) {
        if self.alert.is_some() {
            let w = self.island.expanded_width.max(320);
            return (w, 124u32.max(self.island.height));
        }
        match self.presentation() {
            crate::animation::notch::Presentation::Expanded => {
                let exp_w = self.island.expanded_width;
                let exp_h = if self.media_playing() && !self.tasks.is_empty() {
                    210u32
                } else if self.media_playing() || !self.tasks.is_empty() {
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
        let attached = self.island.is_attached();
        let y_offset = if attached { 0 } else { self.island.y_offset };
        Some((attached, y_offset))
    }

    /// Routes a click at frame-local (`x`, `y`):
    /// - over interactive buttons (media control, window switcher, notification dismiss)
    /// - over the pill otherwise: toggles expansion and returns the outcome.
    pub fn handle_click(&mut self, x: i32, y: i32, now_ms: u64) -> ClickOutcome {
        for &(id, hx, hy, hw, hh) in &self.icon_hits {
            if x >= hx && x < hx + hw as i32 && y >= hy && y < hy + hh as i32 {
                match id {
                    HIT_MEDIA_PLAY_PAUSE => return ClickOutcome::MediaToggle,
                    HIT_MEDIA_PREV => return ClickOutcome::MediaPrev,
                    HIT_MEDIA_NEXT => return ClickOutcome::MediaNext,
                    HIT_ALERT_DISMISS => {
                        self.alert = None;
                        let _ = self.morph_to_target(now_ms);
                        return ClickOutcome::AlertDismiss;
                    }
                    hwnd if hwnd > 0 => return ClickOutcome::ActivateWindow(hwnd),
                    _ => {}
                }
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
        if !self.island.is_enabled() {
            return false;
        }
        self.manually_expanded = !self.manually_expanded;
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

    /// Morphs (or snaps, under reduced motion) to the current target size
    /// using the iOS spring model. Interrupted morphs inherit the current
    /// velocity, exactly like the Dynamic Island. The corner radius and
    /// the blob separation ride the same motion.
    fn morph_to_target(&mut self, now_ms: u64) -> bool {
        let (mut target_w, mut target_h) = self.target_size(self.state);
        let mut target_r = self.target_radius();
        // Press swell: inflate the target a few percent while held.
        if self.pressed {
            target_w = ((target_w as f32 * 1.03).round() as u32).max(target_w + 2);
            target_h = ((target_h as f32 * 1.03).round() as u32).max(target_h + 2);
            target_r *= 1.03;
        }
        let params = spring_params(self.island.animation_ms, self.island.spring_bounce);
        self.retarget_separation(params);

        if self.reduced_motion {
            self.spring = None;
            self.radius = target_r;
            self.current = self.render_island(self.state, target_w, target_h);
            self.animation = AnimationSource::Still(self.current.clone());
            self.frame_deadline = None;
            return true;
        }
        let (from_w, from_h, from_r, vel_x, vel_y, vel_z) = match self.spring.take() {
            Some(s) => (s.x, s.y, s.z, s.vx, s.vy, s.vz),
            None => (
                self.current.width as f32,
                self.current.height as f32,
                self.radius,
                0.0,
                0.0,
                0.0,
            ),
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
            self.current = self.render_island(self.state, target_w, target_h);
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
        let kind = event.event;
        let session = event.session_id.clone();
        self.reducer.advance(now_ms);
        self.reducer.apply(event);
        let mut actions = self.sync_state(now_ms);

        if self.island.is_enabled() {
            match kind {
                termielle_core::EventKind::NeedsInput => {
                    self.trigger_alert(
                        "Input Required",
                        format!("Session: {session}"),
                        [255, 180, 50, 255],
                        3500,
                        now_ms,
                    );
                    actions.present_frame = true;
                }
                termielle_core::EventKind::TurnFailed => {
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
        self.reducer.advance(now_ms);
        let mut actions = self.sync_state(now_ms);

        // NOTE: dashboard content arrives from the background worker via
        // `set_task_update`; nothing here may block on window capture or
        // media queries, otherwise hovering and morphing visibly stall.

        // Check if transient notification alert expired
        if let Some(alert) = &self.alert {
            if now_ms >= alert.expires_at_ms {
                self.alert = None;
                self.morph_to_target(now_ms);
                actions.present_frame = true;
            }
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
                    self.current = self.render_island(self.state, target_w, target_h);
                    self.animation = AnimationSource::Still(self.current.clone());
                    self.frame_deadline = None;
                    self.arm_face_deadline(now_ms);
                } else {
                    let (w, h) = current_size;
                    self.current = self.render_island(self.state, w, h);
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
                    self.current = self.render_island(self.state, w, h);
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
        let mut deadline = match (earliest, self.face_deadline) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        if let Some(alert) = &self.alert {
            deadline = match deadline {
                Some(d) => Some(d.min(alert.expires_at_ms)),
                None => Some(alert.expires_at_ms),
            };
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
            self.current = self.render_island(self.state, w, h);
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
                let (target_w, target_h) = self.target_size(state);
                let (current_w, current_h) = (self.current.width, self.current.height);
                if (current_w == target_w && current_h == target_h) || self.reduced_motion {
                    self.current = self.render_island(state, target_w, target_h);
                    self.animation = AnimationSource::Still(self.current.clone());
                    self.spring = None;
                    self.frame_deadline = None;
                    self.arm_face_deadline(now_ms);
                } else {
                    let params = spring_params(self.island.animation_ms, self.island.spring_bounce);
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
            let (current_w, current_h) = (self.current.width, self.current.height);
            if (current_w, current_h) != (target_w, target_h) && self.spring.is_none() {
                if self.reduced_motion {
                    self.current = self.render_island(self.state, target_w, target_h);
                    self.animation = AnimationSource::Still(self.current.clone());
                    actions.present_frame = true;
                } else {
                    let params = spring_params(self.island.animation_ms, self.island.spring_bounce);
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

#[cfg(test)]
mod spring_tests {
    use super::*;

    #[test]
    fn radius_rides_the_same_spring_to_its_target() {
        let mut spring = Spring2D::new(140, 36, 18.0, 320, 154, 28.0, spring_params(500, 0.2));
        for _ in 0..2000 {
            let (_, settled) = spring.step(1.0 / 120.0);
            if settled {
                break;
            }
        }
        assert_eq!(spring.x, 320.0);
        assert_eq!(spring.y, 154.0);
        assert_eq!(spring.z, 28.0, "corner radius must settle at its target");
    }

    #[test]
    fn progress_advances_monotonically_and_preserves_velocity_on_retarget() {
        let mut spring = Spring2D::new(140, 36, 18.0, 320, 36, 18.0, spring_params(350, 0.18));
        let mut last = 0.0;
        for _ in 0..20 {
            spring.step(1.0 / 60.0);
            let p = spring.progress();
            assert!(p >= last - 1e-6, "progress must not regress");
            last = p;
        }
        assert!(spring.vx != 0.0, "spring must be moving mid-flight");

        // Interrupted morph: the new spring inherits the velocity.
        let inherited = spring.vx;
        let mut retargeted = Spring2D::new(
            spring.x.round() as u32,
            36,
            18.0,
            200,
            36,
            18.0,
            spring_params(350, 0.18),
        );
        retargeted.vx = inherited;
        let dt = 1.0 / 60.0;
        retargeted.step(dt);
        let coasted = retargeted.x;
        let mut from_rest = Spring2D::new(
            spring.x.round() as u32,
            36,
            18.0,
            200,
            36,
            18.0,
            spring_params(350, 0.18),
        );
        from_rest.step(dt);
        assert!(
            (coasted - from_rest.x).abs() > 0.01,
            "inherited velocity must carry the morph forward"
        );
    }
}
