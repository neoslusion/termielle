//! Notch/island geometry, presentation state, morph springs, and pointer interaction.

use super::controller::Controller;
use super::types::ClickOutcome;
use super::types::HIT_ALERT_DISMISS;
use super::types::HIT_MEDIA_NEXT;
use super::types::HIT_MEDIA_PLAY_PAUSE;
use super::types::HIT_MEDIA_PREV;
use crate::animation::spring::{BLOB_GAP_PX, Spring1, Spring2D};
use crate::animation::{AnimationSource, FrameBuffer};
use crate::window::scaled_size;
use termielle_core::{IslandConfig, VisualState, spring_params};

impl Controller {
    /// Renders one island frame for `state`. The frosted-glass layer is
    /// cached per geometry; content elements (face, task icons, session
    /// dots) are **evenly distributed** across the pill width instead of
    /// clumping left, and icon hit-rects are recorded for click activation.
    /// Renders one island frame for `state`: the blob silhouette material,
    /// the presentation content composited over it so content rides the
    /// morph, then the accent strip. Icon hit-rects are recorded for click
    /// activation.
    pub(crate) fn render_island(
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

    /// Whether the idle pill is currently expanded by user click.
    pub(crate) fn is_expanded_idle(&self) -> bool {
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
    pub(crate) fn dashboard_expanded(&self) -> bool {
        if !self.island.is_enabled() || self.spring.is_some() {
            return false;
        }
        matches!(
            self.presentation(),
            crate::animation::notch::Presentation::Expanded
        )
    }

    /// Whether the teal media strip shows (widget on + something playing).
    pub(crate) fn media_playing(&self) -> bool {
        self.island.has_widget("music") && self.media.as_ref().is_some_and(|m| m.playing)
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
    pub(crate) fn compact_primary_width(&self) -> u32 {
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
    pub(crate) fn compact_media_width(&self) -> u32 {
        if !self.island.has_widget("music") {
            return 0;
        }
        22 + 20 + 12
    }

    /// Whether the island currently presents as two blobs — the agent
    /// session pill plus a detached media pill, the Dynamic Island split —
    /// which is the compact presentation with both activities live.
    pub(crate) fn split_active(&self) -> bool {
        self.island.is_enabled()
            && self.presentation() == crate::animation::notch::Presentation::Compact
            && self.media_playing()
            && self.island.has_widget("music")
            && (self.state != VisualState::Idle || self.reducer.session_count() > 0)
    }

    /// The current blob-separation distance in pixels.
    pub(crate) fn separation_now(&self) -> f32 {
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
    pub(crate) fn blob_rects(
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
    pub(crate) fn retarget_separation(&mut self, params: termielle_core::SpringParams) {
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
    pub(crate) fn target_radius(&self) -> f32 {
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
    pub(crate) fn blank_frame(&self, width: u32, height: u32) -> FrameBuffer {
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
    pub(crate) fn to_logical(&self, v: i32) -> i32 {
        (v as f32 / self.render_scale()).round() as i32
    }

    /// The current frame's size in logical units: the window and the spring
    /// both reason in logical pixels while [`FrameBuffer`] holds device pixels.
    /// Derived from the frame's own authoring scale, never the live render
    /// scale — right after a DPI change the two differ, and the live scale
    /// would unpick the wrong logical size.
    pub(crate) fn current_logical_size(&self) -> (u32, u32) {
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
    pub(crate) fn to_render_size(scale: f32, frame: FrameBuffer) -> FrameBuffer {
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
    pub(crate) fn ink(&self) -> [u8; 4] {
        crate::animation::notch::ink_pair(&self.island.glass).0
    }

    /// Dim companion ink for secondary copy.
    pub(crate) fn ink_dim(&self) -> [u8; 4] {
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
    pub(crate) fn morph_params(
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
    pub(crate) fn morph_to_target(&mut self, now_ms: u64) -> bool {
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
}
