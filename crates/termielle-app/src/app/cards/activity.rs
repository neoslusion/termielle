//! Multi-session activity card and explicit local window picker.

use super::super::activity::{
    ActivityAction, PAGE_SIZE, ROW_HEIGHT, SessionKey, WindowLink, elapsed_label, last_page,
};
use super::super::controller::Controller;
use super::super::paint::CardPaintCtx;
use crate::animation::FrameBuffer;
use termielle_core::{EventKind, IslandConfig, VisualState};

impl Controller {
    pub(crate) fn paint_agent_activity(
        &mut self,
        frame: &mut FrameBuffer,
        _state: VisualState,
        island: &IslandConfig,
        ctx: &CardPaintCtx,
    ) {
        self.activity.painted_actions.clear();
        let sessions = self.reducer.session_summaries();
        let picking = self.activity.picking.clone();
        self.activity.deadline = picking.is_none().then(|| ctx.now.saturating_add(1_000));
        let header_x = ctx.pad + if island.has_widget("face") { 30 } else { 0 };
        let title = if let Some(key) = &picking {
            format!("Link · {}", key.source.as_str())
        } else {
            format!("Sessions · {}", sessions.len())
        };
        crate::animation::notch::draw_text(
            frame,
            &title,
            header_x,
            16,
            ctx.width.saturating_sub(header_x as u32 + 76),
            11,
            true,
            self.ink(),
        );

        if let Some(key) = &picking {
            crate::animation::notch::draw_text(
                frame,
                &key.id,
                header_x,
                30,
                ctx.width.saturating_sub(header_x as u32 + 76),
                8,
                false,
                self.ink_dim(),
            );
        }
        let row_width = ctx.width.saturating_sub(ctx.pad as u32 * 2);
        let (page, count) = if let Some(key) = picking {
            self.activity_button(
                frame,
                ctx,
                ActivityAction::Back,
                "Back",
                (ctx.width as i32 - ctx.pad - 56, 12, 56, 26),
            );
            let page = self.activity.window_page;
            let tasks: Vec<_> = self
                .windows
                .iter()
                .skip(page * PAGE_SIZE)
                .take(PAGE_SIZE)
                .map(|t| {
                    (
                        t.title.clone(),
                        WindowLink {
                            hwnd: t.hwnd,
                            process_id: t.process_id,
                        },
                    )
                })
                .collect();
            for (index, (title, link)) in tasks.into_iter().enumerate() {
                let y = 52 + index as i32 * ROW_HEIGHT as i32;
                self.activity_button(
                    frame,
                    ctx,
                    ActivityAction::Bind(key.clone(), link),
                    &title,
                    (ctx.pad, y, row_width, 44),
                );
            }
            if self.windows.is_empty() {
                crate::animation::notch::draw_text(
                    frame,
                    "No open windows available",
                    ctx.pad,
                    60,
                    row_width,
                    11,
                    false,
                    self.ink_dim(),
                );
            }
            if self.activity.links.contains_key(&key) {
                self.activity_button(
                    frame,
                    ctx,
                    ActivityAction::Unlink(key),
                    "Unlink",
                    (ctx.pad, ctx.height as i32 - 32, 56, 24),
                );
            }
            (page, self.windows.len())
        } else {
            let page = self.activity.session_page;
            if self.media_available() {
                let rect = (ctx.width as i32 - ctx.pad - 56, 12, 56, 26);
                crate::animation::notch::draw_rounded_rect(
                    frame,
                    rect.0,
                    rect.1,
                    rect.2,
                    rect.3,
                    8,
                    [255, 255, 255, 20],
                    [255, 255, 255, 25],
                );
                crate::animation::notch::draw_text_in_rect(
                    frame,
                    if self.media_playing() {
                        "Pause"
                    } else {
                        "Play"
                    },
                    rect,
                    10,
                    false,
                    self.ink(),
                    false,
                );
                self.icon_hits.push((
                    super::super::types::HIT_MEDIA_PLAY_PAUSE,
                    rect.0,
                    rect.1,
                    rect.2,
                    rect.3,
                ));
            }
            for (index, session) in sessions
                .iter()
                .skip(page * PAGE_SIZE)
                .take(PAGE_SIZE)
                .enumerate()
            {
                let key = SessionKey::from(session);
                let y = 52 + index as i32 * ROW_HEIGHT as i32;
                let text_width = row_width.saturating_sub(76);
                let link = self.activity.links.get(&key);
                let window_title = link
                    .and_then(|link| self.windows.iter().find(|task| link.matches(task)))
                    .map(|task| task.title.clone());
                let action = if window_title.is_some() {
                    ActivityAction::Focus(key.clone())
                } else {
                    ActivityAction::Select(key.clone())
                };
                let hit = self.activity.register(action);
                self.icon_hits
                    .push((hit, ctx.pad, y, text_width, ROW_HEIGHT - 4));
                let hovered = self.hover_point.is_some_and(|(x, py)| {
                    x >= ctx.pad
                        && x < ctx.pad + text_width as i32
                        && py >= y
                        && py < y + ROW_HEIGHT as i32 - 4
                });
                if hovered {
                    crate::animation::notch::draw_rounded_rect(
                        frame,
                        ctx.pad,
                        y,
                        text_width,
                        ROW_HEIGHT - 4,
                        8,
                        [255, 255, 255, 18],
                        [0; 4],
                    );
                }
                let (accent, _) = crate::animation::notch::accent_colors(session.state);
                crate::animation::notch::draw_disc(frame, ctx.pad + 4, y + 9, 3, accent);
                if session.state == VisualState::Ready && !self.reduced_motion {
                    let age = ctx.now.saturating_sub(session.state_since_ms);
                    for i in 0..8 {
                        if let Some((x, py, alpha)) = Self::sparkle_dot(ctx.pad + 4, y + 9, i, age)
                        {
                            crate::animation::notch::draw_disc(
                                frame,
                                x,
                                py,
                                1,
                                [accent[0], accent[1], accent[2], alpha],
                            );
                        }
                    }
                }
                let short_id: String = session.session_id.chars().take(18).collect();
                let title = format!("{} · {}", session.source.as_str(), short_id);
                crate::animation::notch::draw_text(
                    frame,
                    &title,
                    ctx.pad + 14,
                    y + 1,
                    text_width.saturating_sub(14),
                    11,
                    true,
                    self.ink(),
                );
                let finished = session.last_event == EventKind::TurnCompleted
                    && matches!(session.state, VisualState::Ready | VisualState::Idle);
                let status = if finished {
                    "Finished"
                } else if session.state == VisualState::NeedsInput {
                    "Needs you"
                } else {
                    session.state.display_name()
                };
                let since = if finished {
                    session.last_activity_ms
                } else {
                    session.state_since_ms
                };
                let detail = format!(
                    "{status} · {}",
                    elapsed_label(ctx.now.saturating_sub(since))
                );
                crate::animation::notch::draw_text(
                    frame,
                    &detail,
                    ctx.pad + 14,
                    y + 17,
                    text_width.saturating_sub(14),
                    10,
                    false,
                    accent,
                );
                let hint = window_title
                    .as_deref()
                    .unwrap_or("Choose a terminal window");
                crate::animation::notch::draw_text(
                    frame,
                    hint,
                    ctx.pad + 14,
                    y + 33,
                    text_width.saturating_sub(14),
                    9,
                    false,
                    self.ink_dim(),
                );
                self.activity_button(
                    frame,
                    ctx,
                    ActivityAction::Select(key),
                    if window_title.is_some() {
                        "Change"
                    } else {
                        "Link"
                    },
                    (ctx.width as i32 - ctx.pad - 62, y + 8, 62, 30),
                );
            }
            (page, sessions.len())
        };
        let footer_y = ctx.height as i32 - 32;
        if page > 0 {
            self.activity_button(
                frame,
                ctx,
                ActivityAction::Page(false),
                "Prev",
                (ctx.width as i32 - ctx.pad - 110, footer_y, 50, 24),
            );
        }
        if page < last_page(count) {
            self.activity_button(
                frame,
                ctx,
                ActivityAction::Page(true),
                "Next",
                (ctx.width as i32 - ctx.pad - 50, footer_y, 50, 24),
            );
        }
        if self.activity.picking.is_none() {
            let label = format!("{} / {}", page + 1, last_page(count) + 1);
            crate::animation::notch::draw_text(
                frame,
                &label,
                ctx.pad,
                footer_y + 4,
                row_width.saturating_sub(120),
                10,
                false,
                self.ink_dim(),
            );
        }
    }

    fn activity_button(
        &mut self,
        frame: &mut FrameBuffer,
        _ctx: &CardPaintCtx,
        action: ActivityAction,
        label: &str,
        rect: (i32, i32, u32, u32),
    ) {
        let (x, y, w, h) = rect;
        let hovered = self
            .hover_point
            .is_some_and(|(px, py)| px >= x && px < x + w as i32 && py >= y && py < y + h as i32);
        crate::animation::notch::draw_rounded_rect(
            frame,
            x,
            y,
            w,
            h,
            8,
            [255, 255, 255, if hovered { 32 } else { 15 }],
            [255, 255, 255, 25],
        );
        crate::animation::notch::draw_text_in_rect(
            frame,
            label,
            (x + 6, y, w.saturating_sub(12), h),
            10,
            false,
            self.ink(),
            false,
        );
        let id = self.activity.register(action);
        self.icon_hits.push((id, x, y, w, h));
    }
}
