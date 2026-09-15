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
}
