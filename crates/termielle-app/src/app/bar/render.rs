//! Bar orchestrator: glass silhouette, cached zone damage, and center dispatch.

use super::super::controller::Controller;
use super::modules::BarDamage;
use super::types::BAR_POPUP_GAP;
use super::types::BAR_ZONE_GAP;
use super::types::{BarCard, BarMetricsCache, BarZoneCache};
use crate::animation::FrameBuffer;
use crate::window::scaled_size;
use std::rc::Rc;
use termielle_core::VisualState;

/// Composites a transparent cached layer over a logical x-range of the
/// destination, at the given logical row offset. Both frames share the render
/// scale, so the conversion stays in device pixels.
///
/// The layer is *blended*, not copied: a cached side zone only carries the
/// pixels it actually paints, and a raw copy would replace every transparent
/// pixel it owns with nothing. Copying over the strip therefore erased the
/// row's glass everywhere except the gap between the zones, leaving the bar
/// unpainted and the desktop showing through it raw.
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
        let src_row = row * src.width as usize;
        let dst_row = dy as usize * dst.width as usize;
        for col in x0..x1 {
            let si = (src_row + col) * 4;
            let sa = src.pixels_pbgra[si + 3] as u32;
            if sa == 0 {
                continue;
            }
            let di = (dst_row + col) * 4;
            let ia = 255 - sa;
            let d = &mut dst.pixels_pbgra[di..di + 4];
            d[0] = (src.pixels_pbgra[si] as u32 + d[0] as u32 * ia / 255).min(255) as u8;
            d[1] = (src.pixels_pbgra[si + 1] as u32 + d[1] as u32 * ia / 255).min(255) as u8;
            d[2] = (src.pixels_pbgra[si + 2] as u32 + d[2] as u32 * ia / 255).min(255) as u8;
            d[3] = (sa + d[3] as u32 * ia / 255) as u8;
        }
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
    /// Exclusive right edge the left zone may paint into. The center pill and
    /// the right zone own everything past it, so left-zone content stops
    /// here instead of being clipped away after the fact.
    left_limit: i32,
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
        // See `render_island`: banner visibility is re-derived per frame so a
        // collapsed popup cannot keep the countdown tick alive.
        self.alert_visible = false;
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
        // An edge-to-edge bar is a transparent band, not a surface: the
        // desktop shows through it and the only thing on screen that carries
        // the theme's material is the center pill. A strip that is inset by a
        // margin or given a corner radius *is* a shape of its own, so it gets
        // the row's glass. Deciding this here rather than letting it fall out
        // of layer compositing keeps the clear band the documented default
        // instead of an accident of draw order.
        let strip_is_surface = margin > 0 || bar_r > 0;
        let row = strip_is_surface.then(|| {
            let row_blob = [crate::animation::notch::BlobRect {
                x: bar_x,
                y: vis_off,
                w: bar_w,
                h: vis_h,
                r: bar_r,
                attached: true,
            }];
            self.glass_layer_blobs(width, bar_h, 0, true, &row_blob, false)
        });
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

        let left_end = pill_cx.max(0) as u32;
        let right_start = (pill_cx + pill_w as i32).max(0) as u32;
        let zone = ZonePaint {
            metrics,
            width,
            bar_h,
            bar_x,
            pill_off,
            pill_h,
            left_limit: left_end.saturating_sub(BAR_ZONE_GAP) as i32,
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

        if can_partial {
            let mut frame = self.current.clone();
            if damage.contains(BarDamage::LEFT) {
                clear_region(&mut frame, bar_y, 0, left_end, bar_h);
                if let Some(row) = row.as_ref() {
                    composite_region(&mut frame, row, bar_y, 0, left_end);
                }
                composite_region(&mut frame, &left_frame, bar_y, 0, left_end);
            }
            if damage.contains(BarDamage::RIGHT) {
                clear_region(&mut frame, bar_y, right_start, width, bar_h);
                if let Some(row) = row.as_ref() {
                    composite_region(&mut frame, row, bar_y, right_start, width);
                }
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
        if let Some(row) = row.as_ref() {
            composite_region(&mut frame, row, bar_y, 0, width);
        }
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
            zone.bar_x,
            zone.pill_off,
            zone.pill_h,
            zone.left_limit,
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

    /// The compact pill is painted into a strip-local canvas that is then
    /// composited at `bar_y`. On a bottom bar `bar_y` is non-zero, so content
    /// addressed with the shifted y lands outside that canvas and is clipped
    /// away: the strip keeps its glass but loses the pill's face, label and
    /// glyph. Pinning the whole strip against the top bar catches that
    /// without having to guess which pixel is content.
    #[test]
    fn bottom_bar_strip_matches_the_top_bar_strip() {
        fn bar_controller(position: termielle_core::BarPosition) -> Controller {
            let mut island = IslandConfig {
                layout: IslandLayout::Bar,
                ..IslandConfig::default()
            };
            island.bar.height = 36;
            island.bar.position = position;
            island.bar.modules_center = vec!["termielle".to_string()];
            let mut controller = Controller::new_with_island(
                5_000,
                60_000,
                AssetCatalog::new(Vec::new()),
                false,
                None,
                island,
            );
            controller.set_bar_width(1536);
            controller
        }

        let width = 1536u32;
        let bar_h = 36u32;
        // Tall enough that a bottom bar's strip starts below the frame origin
        // (so strip-local and bar-frame y differ), short enough that the
        // compact pill is still the thing on screen.
        let height = 40u32;

        let mut top = bar_controller(termielle_core::BarPosition::Top);
        let top_frame = top.render_bar(VisualState::Idle, width, height, 60_000);
        let mut bottom = bar_controller(termielle_core::BarPosition::Bottom);
        let bottom_frame = bottom.render_bar(VisualState::Idle, width, height, 60_000);

        let bar_y = (height - bar_h) as usize;
        let differing = (0..bar_h as usize)
            .flat_map(|row| (0..width as usize).map(move |col| (row, col)))
            .find(|(row, col)| {
                let top_idx = (row * width as usize + col) * 4;
                let bottom_idx = ((bar_y + row) * width as usize + col) * 4;
                top_frame.pixels_pbgra[top_idx..top_idx + 4]
                    != bottom_frame.pixels_pbgra[bottom_idx..bottom_idx + 4]
            });
        assert!(
            differing.is_none(),
            "bottom bar strip differs from the top bar strip at {differing:?}"
        );

        // The click target is in bar-frame coordinates, so it has to move with
        // the strip rather than stay on the pill's strip-local y.
        let (pill_cx, pill_off, _, pill_h) = bottom.bar_pill_rect(width);
        let hit = bottom
            .icon_hits
            .iter()
            .find(|(id, ..)| *id == crate::bar::HIT_BAR_TERMIELLE_MODULE)
            .copied()
            .expect("collapsed pill registers a click target");
        assert_eq!(
            (hit.1, hit.2, hit.4),
            (pill_cx, bar_y as i32 + pill_off, pill_h)
        );
    }

    fn bar_controller(margin: u32, corner_radius: u32) -> Controller {
        let mut island = IslandConfig {
            layout: IslandLayout::Bar,
            ..IslandConfig::default()
        };
        island.bar.height = 36;
        island.bar.edge_to_edge = margin == 0;
        island.bar.margin = margin;
        island.bar.corner_radius = corner_radius;
        island.bar.modules_left = vec!["workspaces".to_string(), "window".to_string()];
        island.bar.modules_right = vec!["cpu".to_string(), "clock".to_string()];
        let mut controller = Controller::new_with_island(
            5_000,
            60_000,
            AssetCatalog::new(Vec::new()),
            false,
            None,
            island,
        );
        controller.set_bar_width(1536);
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
        controller
    }

    /// The default bar is edge to edge with no margin and no corner radius, so
    /// it is a transparent band and the desktop shows through it. Only the
    /// center pill carries the theme's material. A strip that is inset or
    /// rounded is a shape of its own and does get the row's glass.
    #[test]
    fn only_an_inset_or_rounded_strip_carries_the_row_glass() {
        let width = 1536u32;

        let mut edge = bar_controller(0, 0);
        let frame = edge.render_bar(VisualState::Idle, width, 36, 60_000);
        let alpha = |f: &crate::animation::FrameBuffer, x: u32, y: u32| -> u8 {
            let idx = ((y as usize * f.width as usize) + x as usize) * 4;
            f.pixels_pbgra[idx + 3]
        };
        let (pill_cx, pill_off, pill_w, pill_h) = edge.bar_pill_rect(width);
        let mid = (pill_off + (pill_h / 2) as i32) as u32;

        // The pill itself is glass, and nothing around it is.
        assert!(
            alpha(&frame, pill_cx as u32 + pill_w / 2, mid) > 0,
            "the center pill must carry the glass background"
        );
        for x in [
            4u32,
            pill_cx as u32 / 2,
            pill_cx as u32 + pill_w + 8,
            width - 4,
        ] {
            assert_eq!(
                alpha(&frame, x, mid),
                0,
                "an edge-to-edge bar must stay clear at x={x}"
            );
        }

        for (margin, corner_radius) in [(12u32, 0u32), (0, 10)] {
            let mut floating = bar_controller(margin, corner_radius);
            let frame = floating.render_bar(VisualState::Idle, width, 36, 60_000);
            let y = (floating.bar_row().1 + 4) as u32;
            for x in [margin + 4, width / 2, width - margin - 5] {
                assert!(
                    alpha(&frame, x, y) > 0,
                    "a strip with margin={margin} radius={corner_radius} must be glass at x={x}"
                );
            }
        }
    }
}
