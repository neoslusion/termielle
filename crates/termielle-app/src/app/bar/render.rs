//! Bar orchestrator: glass silhouette, accent strip, metrics cache, zone dispatch.

use super::super::controller::Controller;
use super::types::{BarCard, BarMetricsCache};
use crate::animation::FrameBuffer;
use termielle_core::VisualState;

impl Controller {
    /// Renders a Waybar-style full-width acrylic status bar with an embedded Dynamic Island.
    pub(crate) fn render_bar(
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
}
