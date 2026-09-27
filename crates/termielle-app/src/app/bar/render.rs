//! Bar orchestrator: glass silhouette, cached zone damage, and center dispatch.

use super::super::controller::Controller;
use super::modules::BarDamage;
use super::types::BAR_POPUP_GAP;
use super::types::{BarCard, BarMetricsCache, BarZoneCache};
use crate::animation::FrameBuffer;
use crate::window::scaled_size;
use std::rc::Rc;
use termielle_core::VisualState;

/// Copies a logical x-range from a transparent cached layer into the
/// destination, at the given logical row offset. Both frames share the render
/// scale, so the conversion stays in device pixels.
fn composite_region(dst: &mut FrameBuffer, src: &FrameBuffer, y: i32, x0: u32, x1: u32) {
    let x0 = (x0 as f32 * dst.scale).round() as usize;
    let x1 = ((x1 as f32 * dst.scale).round() as usize).min(dst.width as usize);
    if x0 >= x1 {
        return;
    }
    let y = (y as f32 * dst.scale).round() as i32;
    for row in 0..src.height as usize {
        let dy = y + row as i32;
        if dy < 0 || dy >= dst.height as i32 {
            continue;
        }
        let length = (x1 - x0) * 4;
        let src_start = (row * src.width as usize + x0) * 4;
        let dst_start = (dy as usize * dst.width as usize + x0) * 4;
        dst.pixels_pbgra[dst_start..dst_start + length]
            .copy_from_slice(&src.pixels_pbgra[src_start..src_start + length]);
    }
}

fn clear_region(dst: &mut FrameBuffer, y: i32, x0: u32, x1: u32, rows: u32) {
    let x0 = (x0 as f32 * dst.scale).round() as usize;
    let x1 = ((x1 as f32 * dst.scale).round() as usize).min(dst.width as usize);
    let rows = ((rows as f32 * dst.scale).round() as usize).min(dst.height as usize);
    if x0 >= x1 {
        return;
    }
    let y = (y as f32 * dst.scale).round() as i32;
    for row in 0..rows {
        let dy = y + row as i32;
        if dy < 0 || dy >= dst.height as i32 {
            continue;
        }
        let start = (dy as usize * dst.width as usize + x0) * 4;
        dst.pixels_pbgra[start..start + (x1 - x0) * 4].fill(0);
    }
}

fn is_side_hit(id: isize) -> bool {
    id == crate::bar::HIT_BAR_VOLUME_TOGGLE
        || (id <= crate::bar::HIT_BAR_WORKSPACE_BASE
            && id > crate::bar::HIT_BAR_WORKSPACE_BASE - 50)
}

fn cache_key(controller: &Controller, width: u32, bar_h: u32) -> (u32, u32, f32, u32) {
    (
        width,
        bar_h,
        controller.render_scale(),
        controller.island.bar.margin,
    )
}

struct ZonePaint {
    metrics: BarMetricsCache,
    width: u32,
    bar_h: u32,
    bar_x: i32,
    pill_off: i32,
    pill_h: u32,
    key: (u32, u32, f32, u32),
}

impl Controller {
    /// Renders a full bar. Animation/layout changes use this path; metric-only
    /// updates use [`Self::render_bar_with_damage`] below.
    pub(crate) fn render_bar(
        &mut self,
        state: VisualState,
        width: u32,
        height: u32,
        now_ms: u64,
    ) -> FrameBuffer {
        self.render_bar_with_damage(state, width, height, now_ms, BarDamage::FULL)
    }

    /// Renders only the bar zones invalidated by a metrics update. The
    /// center surface and the other zone are retained from the previous frame.
    pub(crate) fn render_bar_with_damage(
        &mut self,
        state: VisualState,
        width: u32,
        height: u32,
        now_ms: u64,
        damage: BarDamage,
    ) -> FrameBuffer {
        let bar_h = self.island.bar.height;
        let is_top = self.island.bar.position == termielle_core::BarPosition::Top;
        let bar_y = if is_top {
            0
        } else {
            (height.saturating_sub(bar_h)) as i32
        };
        let expanded = height > bar_h + BAR_POPUP_GAP;
        let exp_h = height.saturating_sub(bar_h + BAR_POPUP_GAP);
        let content_w = self
            .island
            .expanded_width
            .min(width.max(1).saturating_sub(32));
        let content_h = self.bar_expanded_height();
        let progress = (exp_h as f32 / content_h as f32).clamp(0.0, 1.0);
        let (pill_cx, _, pill_w, _) = self.bar_pill_rect(width);
        let island_w =
            (pill_w as f32 + (content_w as f32 - pill_w as f32) * progress).round() as u32;
        let island_x = ((width.saturating_sub(island_w)) / 2) as i32;
        let island_y = if is_top {
            (bar_h + BAR_POPUP_GAP) as i32
        } else {
            0
        };

        let (margin, vis_off, vis_h, pill_off, pill_h, bar_r) = self.bar_row();
        let bar_x = margin as i32;
        let bar_w = width.saturating_sub(margin * 2);
        let row_blob = [crate::animation::notch::BlobRect {
            x: bar_x,
            y: vis_off,
            w: bar_w,
            h: vis_h,
            r: bar_r,
            attached: true,
        }];
        let row = self.glass_layer_blobs(width, bar_h, 0, true, &row_blob, false);
        let key = cache_key(self, width, bar_h);
        let left_valid = self
            .bar_left_cache
            .as_ref()
            .is_some_and(|cache| cache.key == key);
        let right_valid = self
            .bar_right_cache
            .as_ref()
            .is_some_and(|cache| cache.key == key);
        let (device_w, device_h) = scaled_size((width.max(1), height.max(1)), self.render_scale());
        let can_partial = !expanded
            && !damage.contains(BarDamage::CENTER)
            && left_valid
            && right_valid
            && self.current.width == device_w
            && self.current.height == device_h;

        if !can_partial {
            self.icon_hits.clear();
        }
        let metrics = self
            .bar_metrics_cache
            .clone()
            .unwrap_or_else(BarMetricsCache::empty);

        let zone = ZonePaint {
            metrics,
            width,
            bar_h,
            bar_x,
            pill_off,
            pill_h,
            key,
        };
        let metric_refresh = !damage.contains(BarDamage::CENTER);
        if !left_valid || (metric_refresh && damage.contains(BarDamage::LEFT)) {
            self.bar_left_cache = Some(self.build_left_zone(&zone));
        }
        if !right_valid || (metric_refresh && damage.contains(BarDamage::RIGHT)) {
            self.bar_right_cache = Some(self.build_right_zone(&zone));
        }
        let left = self.bar_left_cache.as_ref().expect("left zone cache");
        let right = self.bar_right_cache.as_ref().expect("right zone cache");
        let left_frame = left.frame.clone();
        let right_frame = right.frame.clone();
        let left_hits: Vec<_> = left
            .hits
            .iter()
            .map(|(id, x, y, w, h)| (*id, *x, *y + bar_y, *w, *h))
            .collect();
        let right_hits: Vec<_> = right
            .hits
            .iter()
            .map(|(id, x, y, w, h)| (*id, *x, *y + bar_y, *w, *h))
            .collect();

        let left_end = pill_cx.max(0) as u32;
        let right_start = (pill_cx + pill_w as i32).max(0) as u32;
        if can_partial {
            let mut frame = self.current.clone();
            if damage.contains(BarDamage::LEFT) {
                clear_region(&mut frame, bar_y, 0, left_end, bar_h);
                composite_region(&mut frame, &row, bar_y, 0, left_end);
                composite_region(&mut frame, &left_frame, bar_y, 0, left_end);
            }
            if damage.contains(BarDamage::RIGHT) {
                clear_region(&mut frame, bar_y, right_start, width, bar_h);
                composite_region(&mut frame, &row, bar_y, right_start, width);
                composite_region(&mut frame, &right_frame, bar_y, right_start, width);
            }
            let mut hits = left_hits;
            hits.extend(right_hits);
            let old_center: Vec<_> = self
                .icon_hits
                .iter()
                .filter(|(id, ..)| !is_side_hit(*id))
                .copied()
                .collect();
            hits.extend(old_center);
            self.icon_hits = hits;
            return frame;
        }

        let mut frame = self.blank_frame(width, height);
        composite_region(&mut frame, &row, bar_y, 0, width);
        composite_region(&mut frame, &left_frame, bar_y, 0, left_end);
        composite_region(&mut frame, &right_frame, bar_y, right_start, width);
        if expanded && self.bar_module("center", "termielle") {
            let tint = self.island.glass.tint;
            crate::animation::notch::draw_rounded_rect(
                &mut frame,
                island_x,
                island_y,
                island_w,
                exp_h,
                20.min(exp_h / 2),
                tint,
                [0; 4],
            );
        }

        let card = BarCard {
            bar_y,
            island_x,
            island_y,
            island_w,
            exp_h,
            expanded,
            content_w,
            content_h,
            progress,
        };
        let mut bar_hits = left_hits;
        bar_hits.extend(right_hits);
        self.paint_bar_center(&mut frame, state, now_ms, width, card, bar_hits);
        frame
    }

    fn build_left_zone(&self, zone: &ZonePaint) -> BarZoneCache {
        let mut frame = self.blank_frame(zone.width, zone.bar_h);
        let hits = self.paint_bar_left(
            &mut frame,
            &zone.metrics,
            crate::system::accent_color_bgra(),
            zone.bar_x,
            zone.pill_off,
            zone.pill_h,
        );
        BarZoneCache {
            key: zone.key,
            frame: Rc::new(frame),
            hits,
        }
    }

    fn build_right_zone(&self, zone: &ZonePaint) -> BarZoneCache {
        let mut frame = self.blank_frame(zone.width, zone.bar_h);
        let hits = self.paint_bar_right(
            &mut frame,
            &zone.metrics,
            zone.width,
            zone.bar_x,
            zone.pill_off,
            zone.pill_h,
        );
        BarZoneCache {
            key: zone.key,
            frame: Rc::new(frame),
            hits,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bar::metrics::Snapshot;
    use crate::bar::workspaces::WorkspaceSnapshot;
    use termielle_core::{AssetCatalog, IslandConfig, IslandLayout};

    #[test]
    fn full_render_installs_cached_side_hit_regions() {
        let mut island = IslandConfig {
            layout: IslandLayout::Bar,
            ..IslandConfig::default()
        };
        island.bar.height = 36;
        let mut controller = Controller::new_with_island(
            5_000,
            60_000,
            AssetCatalog::new(Vec::new()),
            false,
            None,
            island,
        );
        controller.set_bar_width(1920);
        controller.set_bar_metrics(
            Snapshot {
                workspaces: WorkspaceSnapshot {
                    total: 2,
                    active: 1,
                },
                ..Snapshot::empty()
            },
            0,
        );
        assert!(controller.icon_hits.iter().any(|hit| {
            hit.0 <= crate::bar::HIT_BAR_WORKSPACE_BASE
                && hit.0 > crate::bar::HIT_BAR_WORKSPACE_BASE - 50
        }));
    }
}
