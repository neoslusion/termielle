use termielle_app::window::{Rect, clamp_to_work_area};

#[test]
fn a_2560px_bar_survives_a_temporary_1920px_monitor() {
    let work = Rect {
        left: 0,
        top: 0,
        right: 1920,
        bottom: 1040,
    };
    // This produced the installed crash: clamp(0, -640) inside WM_DISPLAYCHANGE.
    assert_eq!(clamp_to_work_area((0, 0), (2560, 36), work), (0, 0));
}

#[test]
fn oversized_surfaces_keep_their_origin_visible_on_negative_origin_monitors() {
    let work = Rect {
        left: -1920,
        top: -1080,
        right: 0,
        bottom: -40,
    };
    assert_eq!(
        clamp_to_work_area((100, 100), (2560, 1440), work),
        (-1920, -1080)
    );
}

#[test]
fn empty_or_inverted_work_areas_do_not_panic_during_resume() {
    for work in [
        Rect::default(),
        Rect {
            left: 50,
            top: 60,
            right: 0,
            bottom: 0,
        },
    ] {
        assert_eq!(
            clamp_to_work_area((999, -999), (2560, 36), work),
            (work.left, work.top)
        );
    }
}

#[test]
fn extreme_coordinates_and_negative_sizes_have_valid_clamp_bounds() {
    let work = Rect {
        left: i32::MIN,
        top: i32::MIN,
        right: i32::MAX,
        bottom: i32::MAX,
    };
    assert_eq!(
        clamp_to_work_area((i32::MAX, i32::MAX), (i32::MAX, i32::MAX), work),
        (0, 0)
    );
    assert_eq!(clamp_to_work_area((10, 20), (-1, -1), work), (10, 20));
}
