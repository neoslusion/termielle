//! The controller: folds pipe events and timer ticks into window actions.
//!
//! [`Controller`] owns the reducer, the active animation, and the single
//! deadline the window's timer is armed for. It never touches Win32 itself;
//! [`main`](crate::main) maps its actions onto the window, the pipe, and the
//! log.
//!
//! Split into focused modules: `types` (shared vocabulary), `controller`
//! (fold + classic path), `island` (geometry/morph/interaction), `bar`
//! (status bar), `face` (termielle face cache), `paint` (content painters),
//! `cards` (expanded-card sections).

pub(crate) mod bar;
pub(crate) mod cards;
pub(crate) mod controller;
pub(crate) mod face;
pub(crate) mod island;
pub(crate) mod paint;
pub(crate) mod types;

pub use controller::Controller;
pub use types::{
    AlertBanner, AlertKind, ClickOutcome, ControllerActions, FALLBACK_FRAME_SIZE,
    HIT_ALERT_DISMISS, HIT_CARD_NOTIFICATIONS, HIT_CARD_PANEL, HIT_MEDIA_NEXT,
    HIT_MEDIA_PLAY_PAUSE, HIT_MEDIA_PREV, HIT_NOTIFICATIONS_CLEAR, HIT_PANEL_VOLUME_TRACK,
};
