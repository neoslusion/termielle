//! Expanded-card sections, one submodule per section.
//!
//! `paint.rs` dispatches into these from `paint_expanded_content`; the
//! frosted-glass cache lives in `glass`.

pub(crate) mod activity;
pub(crate) mod dashboard;
pub(crate) mod glass;
pub(crate) mod media;
pub(crate) mod panel;
pub(crate) mod short;
pub(crate) mod switcher;
