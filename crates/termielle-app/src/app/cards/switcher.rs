//! Expanded-card section: opt-in open-windows switcher tiles.

use super::super::controller::Controller;
use super::super::paint::CardPaintCtx;
use crate::animation::FrameBuffer;
use termielle_core::IslandConfig;

impl Controller {
    /// Expanded card section: opt-in open-windows switcher tiles.
    pub(crate) fn paint_task_switcher(
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
            "Open windows",
            tag_x,
            16,
            ctx.width.saturating_sub((tag_x as u32) + 50),
            10,
            true,
            self.ink_dim(),
        );

        let tile_w = 44i32;
        let tile_h = 44i32;
        let gap = 12i32;
        let max_thumbnails = island.max_thumbnails as usize;
        let count = self.tasks.len().min(max_thumbnails) as i32;
        let total_w = if count > 0 {
            count * tile_w + (count - 1) * gap
        } else {
            0
        };
        let start_x = ((ctx.width as i32 - total_w) / 2).max(ctx.pad);
        let tile_y = 56i32;

        for (i, task) in self.tasks.iter().take(max_thumbnails).enumerate() {
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
            crate::animation::notch::draw_rounded_rect(
                frame,
                tx,
                tile_y,
                tile_w as u32,
                tile_h as u32,
                10,
                tile_bg,
                tile_border,
            );

            // Blit high-res window app icon inside tile
            crate::animation::notch::blit_rounded_pixels(
                frame,
                &task.pixels_pbgra,
                task.width,
                task.height,
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
        let hovered_task = self.tasks.iter().take(max_thumbnails).find(|t| {
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
        if let Some(task) = hovered_task {
            crate::animation::notch::draw_text(
                frame,
                &task.title,
                ctx.pad,
                116,
                ctx.width.saturating_sub((ctx.pad as u32) * 2),
                11,
                false,
                self.ink_dim(),
            );
        }
    }
}
