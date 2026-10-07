//! Notch/island geometry, presentation state, morph springs, and pointer interaction.

use super::bar::modules::{BarDamage, metrics_damage};
use super::bar::types::BAR_POPUP_GAP;
use super::controller::Controller;
use super::types::ClickOutcome;
use super::types::HIT_ALERT_DISMISS;
use super::types::HIT_MEDIA_NEXT;
use super::types::HIT_MEDIA_PLAY_PAUSE;
use super::types::HIT_MEDIA_PREV;
use crate::animation::FrameBuffer;
use crate::animation::spring::{BLOB_GAP_PX, Spring1, Spring2D};
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
        self.activity.deadline = None;
        if self.island.is_bar() {
            return self.render_bar(state, width, height, now_ms);
        }
        // Every fresh frame re-derives banner visibility: the countdown tick
        // must stop the moment a collapse or cutover leaves no banner on
        // screen, even though that frame never reaches `render_content`.
        self.alert_visible = false;

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
        let radius = (self.radius.round() as u32).min(height / 2);
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
        let (mut motion_alpha, dx, mut dy) =
            Self::content_motion(self.spring.as_ref(), presentation, state, age_ms);
        if !self.alerts.is_empty() && self.spring.is_some() {
            let (target_w, target_h) = self.target_size(state);
            let fit =
                (width as f32 / target_w.max(1) as f32).min(height as f32 / target_h.max(1) as f32);
            motion_alpha =
                (255.0 * crate::animation::notch::smoothstep(0.35, 0.9, fit)).round() as u8;
            dy = ((1.0 - fit).max(0.0) * 4.0).round() as i32;
        }
        self.icon_hits.retain_mut(|hit| {
            if !super::activity::SessionActivity::is_hit(hit.0) {
                return true;
            }
            hit.1 += dx;
            hit.2 += dy;
            let right = (hit.1 + hit.3 as i32).min(width as i32);
            let bottom = (hit.2 + hit.4 as i32).min(height as i32);
            hit.1 = hit.1.max(0);
            hit.2 = hit.2.max(0);
            hit.3 = right.saturating_sub(hit.1).max(0) as u32;
            hit.4 = bottom.saturating_sub(hit.2).max(0) as u32;
            motion_alpha >= 128 && hit.3 > 0 && hit.4 > 0
        });
        let swap = self.content_transition.as_ref().map(|transition| {
            let elapsed = now_ms.saturating_sub(transition.started_ms);
            let linear = (elapsed as f32 / transition.duration_ms.max(1) as f32).clamp(0.0, 1.0);
            (
                transition.previous.clone(),
                crate::animation::notch::smoothstep(0.0, 1.0, linear),
                elapsed >= transition.duration_ms,
            )
        });
        let mut visible_content = self.blank_frame(width, height);
        crate::animation::notch::blend_frame_over(
            &mut visible_content,
            &content,
            dx,
            dy,
            motion_alpha,
        );
        if let Some((previous, eased, complete)) = swap {
            crate::animation::notch::crossfade_content(
                &mut visible_content,
                &previous,
                (eased * 255.0).round() as u8,
            );
            if complete {
                self.content_transition = None;
            }
        }
        let bridge_k = crate::animation::notch::BRIDGE_K_MAX
            * (1.0 - self.separation_now().clamp(0.0, BLOB_GAP_PX) / BLOB_GAP_PX);
        crate::animation::notch::clip_content(&mut visible_content, &blobs, bridge_k);
        crate::animation::notch::blend_frame_over(&mut frame, &visible_content, 0, 0, 255);
        // An interruption starts from the composite the user actually saw,
        // including a partially completed content swap.
        self.content_layer = Some(std::rc::Rc::new(visible_content));

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
            let pulse = (0.5 + 0.5 * ((now_ms % 450) as f32 / 450.0 * std::f32::consts::TAU).sin())
                .clamp(0.0, 1.0);
            let mut lit = strip;
            lit[3] = (140.0 + 115.0 * pulse).round() as u8;
            lit
        } else {
            strip
        };
        crate::animation::notch::draw_accent_strip(&mut frame, attached, radius, strip);
        frame
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
        if self.manually_expanded || self.panel_open || self.is_navigation_open() {
            // Waybar mode is a persistent status surface: an explicit click on
            // the Termielle module opens its focused popup. Control Center is a
            // separate surface that expands the window on its own account.
            return Presentation::Expanded;
        }
        // A bar's pill *is* the island, so hovering it opens the same card a
        // click does. Standalone Island keeps its own smaller hover step,
        // which is why the two layouts differ here.
        if self.hover_expanded {
            return if self.island.is_bar() {
                Presentation::Expanded
            } else {
                Presentation::Compact
            };
        }
        let agent_live = !matches!(self.state, VisualState::Idle);
        if agent_live || self.media_available() {
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
    /// Whether a media session is available for the module, including a
    /// paused session. Playback animation still uses [`Self::media_playing`].
    pub(crate) fn media_available(&self) -> bool {
        self.island.has_widget("music") && self.media.is_some()
    }

    pub(crate) fn hover_content_available(&self) -> bool {
        self.activity_available()
            || self.state != VisualState::Idle
            || self.media_available()
            || !self.alerts.is_empty()
    }

    /// The largest size this surface can reach from here, in logical pixels.
    /// The frosted backdrop captures a rect up front rather than a rect per
    /// frame, so it needs to know how far the surface can travel.
    pub fn max_surface_size(&self) -> (u32, u32) {
        let (mut w, mut h) = self.target_size(self.state);
        // Reserve the tallest combined media/tasks dashboard even while the
        // surface is mid-morph.
        w = w.max(self.island.expanded_width.max(320));
        let activity_height = super::activity::MAX_CARD_HEIGHT
            + if self.island.is_bar() {
                self.island.bar.height + BAR_POPUP_GAP
            } else {
                0
            };
        let navigation_height = 328 + self.island.bar.height + BAR_POPUP_GAP;
        h = h
            .max(activity_height)
            .max(navigation_height)
            .max(self.island.height);
        (w, h)
    }

    /// Whether the surface can still be heading somewhere: mid-morph, held
    /// open, or about to grow because an activity or alert showed up. A
    /// settled compact pill has no reason to capture a larger backdrop.
    pub fn surface_can_grow(&self) -> bool {
        if self.island.is_bar() {
            // The bar popup is always one click away, so it keeps its
            // envelope permanently, exactly like the strip's own.
            return true;
        }
        self.spring.is_some()
            || self.manually_expanded
            || self.hover_expanded
            || !self.alerts.is_empty()
            || self.media_available()
            || !self.tasks.is_empty()
    }

    /// Target (width, height) for the current visual and presentation state.
    pub fn target_size(&self, _state: VisualState) -> (u32, u32) {
        if self.island.is_bar() {
            let w = self.bar_width;
            let base_h = self.island.bar.height;
            return match self.presentation() {
                crate::animation::notch::Presentation::Expanded => {
                    (w, base_h + self.bar_expanded_height() + BAR_POPUP_GAP)
                }
                _ => (w, base_h),
            };
        }
        if !self.alerts.is_empty() {
            let w = self.island.expanded_width.max(320);
            return (w, super::types::ALERT_HEIGHT.max(self.island.height));
        }
        match self.presentation() {
            crate::animation::notch::Presentation::Expanded => {
                let exp_w = self.island.expanded_width;
                if self.activity_available() && !self.panel_open {
                    return (
                        exp_w,
                        self.activity_height().max(154).max(self.island.height),
                    );
                }
                let tasks_active = self.island.has_widget("tasks")
                    && self.island.show_tasks
                    && !self.tasks.is_empty();
                let exp_h = if self.media_available() && tasks_active {
                    210u32
                } else if self.media_available() {
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
                } else if self.media_available() && self.island.has_widget("music") {
                    primary + self.compact_media_width() - 8
                } else {
                    primary
                };
                (width.max(self.island.minimal_width), self.island.height)
            }
            crate::animation::notch::Presentation::Minimal => {
                let mut w = self.island.minimal_width;
                if self.media_available() && self.island.has_widget("music") {
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
        self.fitted_primary_width()
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
            && self.media_available()
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
        if self
            .separation
            .as_ref()
            .is_some_and(|s| s.target == sep_target)
        {
            return;
        }
        let previous = self.separation.take();
        let sep_from = previous.as_ref().map_or(0.0, |s| s.x);
        let sep_vel = previous.as_ref().map_or(0.0, |s| s.v);
        if (sep_target - sep_from).abs() > 0.5 || self.split_active() {
            let mut sep = Spring1::new(sep_from, sep_target, params);
            sep.v = sep_vel;
            self.separation = Some(sep);
        }
    }

    /// The settled corner radius for the current presentation. Compact and
    /// minimal surfaces remain pills; expanded geometry honors the user's
    /// radius, bounded by half the current height.
    pub(crate) fn target_radius(&self) -> f32 {
        match self.presentation() {
            crate::animation::notch::Presentation::Expanded => {
                let max_r = self.target_size(self.state).1 as f32 / 2.0;
                (self.island.corner_radius as f32).min(max_r)
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
        let y_offset =
            if attached || self.presentation() == crate::animation::notch::Presentation::Hidden {
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
                let (_, current_h) = self.current_logical_size();
                let (_, target_h) = self.target_size(self.state);
                self.current =
                    self.render_island(self.state, width_logical, current_h, self.clock_ms);
                if current_h != target_h {
                    self.morph_to_target(self.clock_ms);
                }
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
        // The outgoing snapshot is rasterized at the old DPI. Retire it
        // rather than mixing physically different text sizes in one frame.
        self.content_transition = None;
        self.content_layer = None;
        if self.island.is_enabled() {
            self.current = self.render_island(self.state, logical_w, logical_h, now_ms);
        } else {
            let _ = self.load_animation_classic(self.state, now_ms);
            return true;
        }
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
    pub fn volume_at(&self, x: i32, y: i32) -> bool {
        let (x, y) = (self.to_logical(x), self.to_logical(y));
        self.icon_hits.iter().any(|&(id, hx, hy, hw, hh)| {
            id == crate::bar::HIT_BAR_VOLUME_TOGGLE
                && x >= hx
                && x < hx + hw as i32
                && y >= hy
                && y < hy + hh as i32
        })
    }

    /// Whether the pointer is over the panel's volume row. The row answers
    /// the wheel exactly like the bar's speaker, so the same
    /// `Command::VolumeWheel` path serves both.
    pub fn panel_volume_at(&self, x: i32, y: i32) -> bool {
        if !self.panel_open {
            return false;
        }
        let (x, y) = (self.to_logical(x), self.to_logical(y));
        self.icon_hits.iter().any(|&(id, hx, hy, hw, hh)| {
            id == crate::app::types::HIT_PANEL_VOLUME_TRACK
                && x >= hx
                && x < hx + hw as i32
                && y >= hy
                && y < hy + hh as i32
        })
    }

    pub fn refresh_bar_metrics(&mut self, now_ms: u64) {
        if self.island.is_bar() {
            self.bar_deadline = Some(now_ms);
        }
    }

    pub fn set_bar_metrics(
        &mut self,
        snapshot: crate::bar::metrics::Snapshot,
        now_ms: u64,
    ) -> bool {
        let volume_changed = self
            .bar_metrics_cache
            .as_ref()
            .is_some_and(|previous| previous.volume != snapshot.volume);
        let mut damage = metrics_damage(
            self.bar_metrics_cache.as_ref(),
            &snapshot,
            &self.island.bar,
            self.is_panel_open(),
        );
        self.bar_metrics_cache = Some(snapshot);
        if !self.island.is_bar() {
            return false;
        }
        if volume_changed && self.bar_module("center", "termielle") {
            self.volume_feedback_deadline = Some(now_ms.saturating_add(1_500));
            damage = damage.union(BarDamage::CENTER);
        }
        if damage == BarDamage::NONE {
            return false;
        }
        if damage.contains(BarDamage::CENTER) {
            self.bar_left_cache = None;
            self.bar_right_cache = None;
        }
        let (width, height) = self.current_logical_size();
        self.current = self.render_bar_with_damage(self.state, width, height, now_ms, damage);
        true
    }

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
            if super::navigation::Navigation::is_rail_hit(id)
                || super::navigation::Navigation::is_popup_hit(id)
            {
                return self.handle_navigation_hit(id, now_ms);
            }
            if super::activity::SessionActivity::is_hit(id) {
                return self.handle_activity_hit(id, now_ms);
            }
            match id {
                HIT_MEDIA_PLAY_PAUSE => return ClickOutcome::MediaToggle,
                HIT_ALERT_DISMISS => {
                    let previous_geometry = self.current_logical_size();
                    self.alerts.pop_front();
                    self.arm_front_alert(now_ms);
                    if !self.island.is_bar() && previous_geometry == self.target_size(self.state) {
                        self.begin_content_transition(now_ms);
                    }
                    self.morph_to_target(now_ms);
                    return ClickOutcome::AlertDismiss;
                }
                HIT_MEDIA_PREV => return ClickOutcome::MediaPrev,
                HIT_MEDIA_NEXT => return ClickOutcome::MediaNext,
                id if crate::bar::shell::ShellAction::from_hit(id).is_some() => {
                    return ClickOutcome::Shell(
                        crate::bar::shell::ShellAction::from_hit(id)
                            .unwrap_or(crate::bar::shell::ShellAction::Start),
                    );
                }
                crate::bar::HIT_BAR_VOLUME_TOGGLE => return ClickOutcome::VolumeToggle,
                crate::app::types::HIT_CARD_NOTIFICATIONS => {
                    if self.panel_open
                        && self.panel_kind == crate::app::types::RightPanel::Notifications
                    {
                        self.close_panel(now_ms);
                    } else {
                        self.panel_open = true;
                        self.panel_kind = crate::app::types::RightPanel::Notifications;
                        self.panel_morphing = false;
                        self.unread_notifications = 0;
                        self.bar_right_cache = None;
                        self.interaction_deadline = Some(now_ms.saturating_add(100));
                        self.morph_to_target(now_ms);
                    }
                    return ClickOutcome::PanelToggled;
                }
                crate::app::types::HIT_NOTIFICATIONS_CLEAR => {
                    self.recent_notifications.clear();
                    self.unread_notifications = 0;
                    self.bar_right_cache = None;
                    self.morph_to_target(now_ms);
                    return ClickOutcome::NotificationsCleared;
                }
                crate::app::types::HIT_PANEL_VOLUME_TRACK => {
                    if let Some((_, track_x, _, track_width, _)) = self
                        .icon_hits
                        .iter()
                        .find(|&&(hit, ..)| hit == crate::app::types::HIT_PANEL_VOLUME_TRACK)
                        .copied()
                    {
                        let level = ((x - track_x) * 100
                            / track_width.saturating_sub(1).max(1) as i32)
                            .clamp(0, 100);
                        return ClickOutcome::VolumeSet(level as u8);
                    }
                }
                crate::app::types::HIT_CARD_PANEL => {
                    if self.panel_open && self.panel_kind == crate::app::types::RightPanel::Controls
                    {
                        self.close_panel(now_ms);
                        return ClickOutcome::PanelToggled;
                    }
                    self.panel_open = true;
                    self.panel_kind = crate::app::types::RightPanel::Controls;
                    self.panel_morphing = false;
                    self.interaction_deadline = Some(now_ms.saturating_add(100));
                    self.morph_to_target(now_ms);
                    return ClickOutcome::PanelToggled;
                }
                crate::bar::HIT_BAR_TERMIELLE_MODULE => {
                    if self.manually_expanded || self.hover_expanded {
                        self.close_notch(now_ms);
                        return ClickOutcome::Collapsed;
                    }
                    self.toggle_expand(now_ms);
                    return ClickOutcome::Expanded;
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
        if self.island.is_bar() {
            // Passive status modules are not hidden popup triggers in the
            // persistent Waybar surface. Only the explicit center module hit
            // above may open or close the Termielle popup.
            return ClickOutcome::None;
        }
        if self.collapse_if_expanded(now_ms) {
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
        // Repaint only when the point moved across an icon hit-rect boundary.
        let on_icon = |pt: &Option<(i32, i32)>| {
            pt.and_then(|(px, py)| {
                self.icon_hits.iter().position(|(_, hx, hy, hw, hh)| {
                    px >= *hx && px < hx + *hw as i32 && py >= *hy && py < hy + *hh as i32
                })
            })
        };
        let changed = on_icon(&self.hover_point) != on_icon(&point);
        self.hover_point = point;
        if changed && self.island.is_enabled() {
            let (width, height) = self.current_logical_size();
            self.current = self.render_island(self.state, width, height, self.clock_ms);
        }
        changed
    }

    pub fn toggle_expand(&mut self, now_ms: u64) -> bool {
        if !self.island.is_enabled() {
            return false;
        }
        if self.manually_expanded && self.close_notch(now_ms) {
            return true;
        }
        self.manually_expanded = true;
        self.interaction_deadline = Some(now_ms.saturating_add(100));
        self.hover_deadline = None;
        self.hover_suppressed = false;
        self.morph_to_target(now_ms)
    }

    /// Whether the island's *own* card is open, by click or by hover.
    ///
    /// This is the state the pill's visibility belongs to. The Control Center
    /// is a separate surface that also makes the window tall, so the window's
    /// height cannot answer it.
    pub(crate) fn island_card_open(&self) -> bool {
        self.manually_expanded || self.hover_expanded
    }

    /// Whether the Control Center panel is open. It is a surface of its own,
    /// not a body of the island's card, so callers can ask about it directly.
    pub fn is_panel_open(&self) -> bool {
        self.panel_open && self.panel_kind == crate::app::types::RightPanel::Controls
    }

    pub fn is_notification_center_open(&self) -> bool {
        self.panel_open && self.panel_kind == crate::app::types::RightPanel::Notifications
    }

    pub fn recent_notification_count(&self) -> usize {
        self.recent_notifications.len()
    }

    pub fn unread_notification_count(&self) -> usize {
        self.unread_notifications
    }

    /// Whether the island is currently manually expanded into the full card.
    pub fn is_manually_expanded(&self) -> bool {
        self.manually_expanded
    }

    /// Closes the Control Center, and only the Control Center.
    ///
    /// Every path that ends the panel goes through here, so the panel's close
    /// cannot pick up a habit of tidying the island's hover state on the way
    /// out. That tidying is the interference: the panel used to clear
    /// `hover_deadline` and set `hover_suppressed`, which are the island's
    /// flags, and the island's hover machine is the thing that decides when
    /// its card is open.
    fn close_panel(&mut self, now_ms: u64) -> bool {
        if !self.panel_open {
            return false;
        }
        self.panel_open = false;
        self.panel_morphing = true;
        self.morph_to_target(now_ms)
    }

    /// Closes the island's card, and only the island's card.
    fn close_notch(&mut self, now_ms: u64) -> bool {
        let was_open = self.manually_expanded;
        self.manually_expanded = false;
        self.interaction_deadline = None;
        self.hover_deadline = None;
        if self.island.is_bar() {
            self.hover_expanded = false;
            self.hover_suppressed = true;
        }
        // Always morph: `||` short-circuits, so folding the flag into the
        // return value would skip the morph exactly when the card was open
        // and leave the tall frame on screen.
        let moved = self.morph_to_target(now_ms);
        was_open || moved
    }

    /// Dismisses the Control Center first, then the island card, when both
    /// are open. Escape and click-outside use this shared entry point.
    pub fn collapse_if_expanded(&mut self, now_ms: u64) -> bool {
        if !self.island.is_enabled() {
            return false;
        }
        if self.is_navigation_open() {
            return self.close_navigation(now_ms);
        }
        if self.panel_open {
            return self.close_panel(now_ms);
        }
        if !self.manually_expanded {
            return false;
        }
        self.close_notch(now_ms)
    }

    /// Hover dwell before a bar pill opens its card, in ms. A cursor merely
    /// crossing the strip should not throw a popup open.
    pub const HOVER_DWELL_MS: u64 = 300;
    /// Grace after the pointer leaves the pill, in ms, so it can travel down
    /// into the card it just opened instead of dismissing it on the way.
    pub const HOVER_GRACE_MS: u64 = 500;

    /// Tracks the cursor and opens the bar's popup on hover when
    /// `expand_on_hover` is on.
    ///
    /// Both edges are deferred. Entering waits out [`HOVER_DWELL_MS`] so a
    /// cursor merely crossing the strip does not throw the card open, and
    /// leaving waits [`HOVER_GRACE_MS`] so the pointer can travel down into
    /// the card it just opened instead of dismissing it on the way.
    pub fn set_hover(&mut self, inside: bool, now_ms: u64) -> bool {
        if !self.island.is_enabled() {
            return false;
        }
        if !self.island.expand_on_hover {
            // Turning the setting off has to close whatever hover opened.
            self.hover_deadline = None;
            if !self.hover_expanded {
                return false;
            }
            self.hover_expanded = false;
            return self.morph_to_target(now_ms);
        }
        if !self.hover_content_available() {
            self.hover_deadline = None;
            if !self.hover_expanded {
                return false;
            }
            self.hover_expanded = false;
            return self.morph_to_target(now_ms);
        }
        if self.island.is_bar() {
            // The panel does not suspend this. It used to: `set_hover`
            // returned early whenever the panel was up, which froze the
            // island's dwell wherever it happened to be. A dwell armed a
            // moment before the panel opened then sat pending for as long as
            // the panel stayed up and fired the moment it closed, throwing the
            // card open behind a dismissal - the notch popping out on the
            // command that closed the panel.
            //
            // Running it is also what makes that impossible. The dwell fires
            // and the grace closes it while the panel is still up, because the
            // pointer is on the panel's control, not the pill, so by the time
            // the panel closes the island has already resolved its own hover.
            // Which card is drawn is decided by `presentation`, not by
            // freezing the other surface's state machine.
            return self.set_bar_hover(inside, now_ms);
        }
        if self.state != VisualState::Idle || self.manually_expanded {
            self.hover_expanded = inside;
            return false;
        }
        if self.hover_expanded == inside {
            return false;
        }
        self.hover_expanded = inside;
        self.morph_to_target(now_ms)
    }

    /// The bar half of [`Self::set_hover`]: a click already owns the card when
    /// one is open, and a click that just closed it must not be undone by the
    /// pointer still resting on the pill - hence `hover_suppressed`, which
    /// only clears once the pointer has actually left.
    fn set_bar_hover(&mut self, inside: bool, now_ms: u64) -> bool {
        if inside {
            if self.hover_suppressed {
                return false;
            }
            // A pointer that jitters a pixel off the pill and back must not
            // leave a close armed behind it: the close fires on schedule even
            // though the pointer is sitting on the pill, the card collapses,
            // re-arms open, and the two chase each other. Cancelling on
            // return is what makes the grace a grace rather than a countdown
            // the pointer never sees.
            if self.hover_deadline.is_some_and(|(expand, _)| !expand) {
                self.hover_deadline = None;
            }
            // Already counting down to open. The poll runs on every present,
            // so this guard is what stops it re-arming: each re-arm pushes the
            // deadline to now + dwell, and a deadline that keeps moving is a
            // deadline that never arrives.
            let opening = self.hover_deadline.is_some_and(|(expand, _)| expand);
            if self.hover_expanded || opening || self.manually_expanded {
                return false;
            }
            self.hover_suppressed = false;
            self.hover_deadline = Some((true, now_ms.saturating_add(Self::HOVER_DWELL_MS)));
            return false;
        }
        // Leaving clears the suppression, so the next entry can open again.
        self.hover_suppressed = false;
        if self.manually_expanded {
            return false;
        }
        if !self.hover_expanded {
            // Not open yet: leaving restarts the dwell rather than clearing
            // it. A pointer resting on a pill is never perfectly still, and
            // clearing on every stray pixel meant the dwell was cancelled
            // before it could ever complete. The dwell is continuous time on
            // the pill, not frames sampled from it.
            if self.hover_deadline.is_some_and(|(expand, _)| expand) {
                self.hover_deadline = Some((true, now_ms.saturating_add(Self::HOVER_DWELL_MS)));
            }
            return false;
        }
        if self.hover_deadline.is_some_and(|(expand, _)| !expand) {
            return false;
        }
        self.hover_deadline = Some((false, now_ms.saturating_add(Self::HOVER_GRACE_MS)));
        false
    }

    /// Press feedback: the pointer went down (`true`) or up on the pill.
    /// The island swells ~3% under the pointer, like the Dynamic Island
    /// under the fingertip, and settles back on release.
    pub fn set_pressed(&mut self, pressed: bool, now_ms: u64) -> bool {
        if !self.island.is_enabled() || self.island.is_bar() || self.pressed == pressed {
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
        if self.island.is_bar() && self.panel_open && self.alerts.is_empty() {
            self.alert_pill_morphing = false;
        }
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
        let carried_progress = self.spring.as_ref().map(Spring2D::progress);

        // Metadata and queued alerts do not restart an already-correct morph
        // or discard elapsed time since the last animation frame.
        if !self.reduced_motion
            && self.spring.as_ref().is_some_and(|s| {
                s.target_x == target_w as f32
                    && s.target_y == target_h as f32
                    && s.target_radius() == target_r
            })
        {
            self.current = self.render_island(self.state, from_w, from_h, now_ms);
            return true;
        }

        if self.reduced_motion {
            self.spring = None;
            self.separation = None;
            if self.alerts.is_empty() {
                self.alert_pill_morphing = false;
            }
            if !self.panel_open {
                self.panel_morphing = false;
            }
            self.radius = target_r;
            self.current = self.render_island(self.state, target_w, target_h, now_ms);
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
            if self.alerts.is_empty() {
                self.alert_pill_morphing = false;
            }
            self.radius = target_r;
            self.current = self.render_island(self.state, target_w, target_h, now_ms);
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
        spring.preserve_position(from_w, from_h);
        if let Some(progress) = carried_progress {
            spring.preserve_progress(progress);
        }
        spring.vx = vel_x;
        spring.vy = vel_y;
        spring.vz = vel_z;
        self.spring = Some(spring);
        self.spring_last_ms = now_ms;
        let interval = self.motion_interval_ms;
        let next = now_ms
            .saturating_add(interval)
            .saturating_sub(self.present_cost_ms.min(interval.saturating_sub(1)));
        self.frame_deadline = Some(next);
        // State ownership and pixels change together. Otherwise the event
        // path acknowledges the new state while presenting the previous glyph
        // until the first display-clock wake.
        self.current = self.render_island(
            self.state,
            from_w.round().max(1.0) as u32,
            from_h.round().max(1.0) as u32,
            now_ms,
        );
        true
    }
}
