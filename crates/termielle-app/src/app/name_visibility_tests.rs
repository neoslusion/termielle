//! Branding-only toggle: geometry, live content and hit targets stay intact.
use super::{Controller, paint::CardPaintCtx};
use termielle_core::{AssetCatalog, IslandConfig, IslandLayout, VisualState};

fn controller(layout: IslandLayout, show_name: bool) -> Controller {
    let mut island = IslandConfig {
        layout,
        show_name,
        widgets: Vec::new(),
        face_animated: false,
        forward_toasts: false,
        ..IslandConfig::default()
    };
    island.bar.modules_left.clear();
    island.bar.modules_right.clear();
    Controller::new_with_island(
        5_000,
        60_000,
        AssetCatalog::new(Vec::new()),
        true,
        None,
        island,
    )
}

#[test]
fn compact_brand_can_be_hidden_without_hiding_activity_or_changing_geometry() {
    for layout in [IslandLayout::Island, IslandLayout::Notch] {
        for scale in [1.0, 1.25, 2.0] {
            let mut ctrl = controller(layout, true);
            ctrl.set_dpi_scale(scale);
            let paint = |ctrl: &mut Controller, show_name, state| {
                let mut island = ctrl.island_config().clone();
                island.show_name = show_name;
                let mut frame = ctrl.blank_frame(180, 36);
                ctrl.paint_compact_content(&mut frame, state, &island, 180, 36, &[], 1_000);
                frame
            };
            let named = paint(&mut ctrl, true, VisualState::Idle);
            let unnamed = paint(&mut ctrl, false, VisualState::Idle);
            assert_eq!((named.width, named.height), (unnamed.width, unnamed.height));
            assert_ne!(
                named.pixels_pbgra, unnamed.pixels_pbgra,
                "idle branding must be removed"
            );
            assert!(
                unnamed.pixels_pbgra.chunks_exact(4).any(|p| p[3] != 0),
                "idle beacon must survive"
            );
            let active = paint(&mut ctrl, true, VisualState::Working);
            let active_unnamed = paint(&mut ctrl, false, VisualState::Working);
            assert_eq!(
                active.pixels_pbgra, active_unnamed.pixels_pbgra,
                "activity must not be hidden"
            );
        }
    }
}

#[test]
fn hiding_the_brand_does_not_hide_media_content() {
    for layout in [IslandLayout::Bar, IslandLayout::Island, IslandLayout::Notch] {
        let mut ctrl = controller(layout, true);
        ctrl.island.widgets.push("music".into());
        ctrl.media = Some(crate::tasks::MediaInfo {
            title: "A track title".into(),
            artist: "An artist".into(),
            ..Default::default()
        });
        let paint = |ctrl: &mut Controller| {
            if layout == IslandLayout::Bar {
                ctrl.render_bar(VisualState::Idle, 960, 36, 1_000)
            } else {
                let island = ctrl.island_config().clone();
                let mut frame = ctrl.blank_frame(180, 36);
                ctrl.paint_compact_content(
                    &mut frame,
                    VisualState::Idle,
                    &island,
                    180,
                    36,
                    &[],
                    1_000,
                );
                frame
            }
        };
        let named = paint(&mut ctrl);
        ctrl.island.show_name = false;
        let unnamed = paint(&mut ctrl);
        assert_eq!(named.pixels_pbgra, unnamed.pixels_pbgra);
    }
}

#[test]
fn dashboard_brand_can_be_hidden_without_removing_header_space() {
    for layout in [IslandLayout::Island, IslandLayout::Notch] {
        let mut ctrl = controller(layout, true);
        let ctx = CardPaintCtx {
            width: 320,
            height: 120,
            cy: 60,
            pad: 18,
            accent: [200, 100, 50, 255],
            now: 1_000,
            eq_now: 0,
        };
        let mut named = ctrl.blank_frame(320, 120);
        let mut unnamed = ctrl.blank_frame(320, 120);
        let mut island = ctrl.island_config().clone();
        ctrl.paint_standby_dashboard(&mut named, &island, &ctx);
        island.show_name = false;
        ctrl.paint_standby_dashboard(&mut unnamed, &island, &ctx);
        let count_header = |frame: &crate::animation::FrameBuffer| {
            (12..33)
                .flat_map(|y| (18..150).map(move |x| (y * frame.width as usize + x) * 4 + 3))
                .filter(|&offset| frame.pixels_pbgra[offset] != 0)
                .count()
        };
        assert!(count_header(&named) > 0);
        assert_eq!(count_header(&unnamed), 0);
    }
}

#[test]
fn bar_brand_toggle_keeps_the_pill_clickable_and_active_labels_unchanged() {
    for position in [
        termielle_core::BarPosition::Top,
        termielle_core::BarPosition::Bottom,
    ] {
        let mut ctrl = controller(IslandLayout::Bar, true);
        ctrl.island.bar.position = position;
        let named = ctrl.render_bar(VisualState::Idle, 960, 36, 1_000);
        let hits = ctrl.click_regions().to_vec();
        ctrl.island.show_name = false;
        let unnamed = ctrl.render_bar(VisualState::Idle, 960, 36, 1_000);
        assert_ne!(named.pixels_pbgra, unnamed.pixels_pbgra);
        assert_eq!(ctrl.click_regions(), hits.as_slice());
        assert!(
            hits.iter()
                .any(|(id, ..)| *id == crate::bar::HIT_BAR_TERMIELLE_MODULE)
        );
        let working = ctrl.render_bar(VisualState::Working, 960, 36, 1_000);
        ctrl.island.show_name = true;
        let named_working = ctrl.render_bar(VisualState::Working, 960, 36, 1_000);
        assert_eq!(working.pixels_pbgra, named_working.pixels_pbgra);
    }
}
