use termielle_app::window::{
    HitTestResult, Rect, alpha_hit_test, clamp_to_work_area, position_within_work_area, scaled_size,
};

const PRIMARY: Rect = Rect {
    left: 0,
    top: 0,
    right: 1920,
    bottom: 1080,
};

#[test]
fn clamp_to_work_area_keeps_valid_position() {
    assert_eq!(
        clamp_to_work_area((900, 300), (360, 360), PRIMARY),
        (900, 300)
    );
}

#[test]
fn clamp_to_work_area_moves_fully_offscreen_into_corners() {
    assert_eq!(clamp_to_work_area((-50, -50), (360, 360), PRIMARY), (0, 0));
    assert_eq!(
        clamp_to_work_area((1920, 1080), (360, 360), PRIMARY),
        (1560, 720)
    );
}

#[test]
fn clamp_to_work_area_handles_negative_origin_monitors() {
    let secondary = Rect {
        left: -1920,
        top: 0,
        right: 0,
        bottom: 1040,
    };
    assert_eq!(
        clamp_to_work_area((-100, 900), (360, 360), secondary),
        (-360, 680)
    );
    assert_eq!(
        clamp_to_work_area((-2000, -50), (360, 360), secondary),
        (-1920, 0)
    );
}

#[test]
fn scaled_size_scales_and_rounds() {
    assert_eq!(scaled_size((360, 360), 1.25), (450, 450));
    assert_eq!(scaled_size((360, 360), 2.0), (720, 720));
    assert_eq!(scaled_size((360, 360), 0.5), (180, 180));
}

#[test]
fn scaled_size_never_collapses_to_zero() {
    assert_eq!(scaled_size((1, 1), 0.5), (1, 1));
}

#[test]
fn position_within_work_area_checks_all_bounds() {
    assert!(position_within_work_area((900, 300), (360, 360), PRIMARY));
    assert!(!position_within_work_area((1900, 300), (360, 360), PRIMARY));
    assert!(!position_within_work_area((900, 1050), (360, 360), PRIMARY));
    assert!(!position_within_work_area((-5, 300), (360, 360), PRIMARY));
}

#[test]
fn alpha_hit_test_uses_threshold() {
    let map = [15, 16, 255];
    assert_eq!(alpha_hit_test(&map, 3, 0, 0), HitTestResult::Transparent);
    assert_eq!(alpha_hit_test(&map, 3, 1, 0), HitTestResult::Caption);
    assert_eq!(alpha_hit_test(&map, 3, 2, 0), HitTestResult::Caption);
}

#[test]
fn alpha_hit_test_out_of_bounds_is_transparent() {
    let map = [255];
    assert_eq!(alpha_hit_test(&map, 1, -1, 0), HitTestResult::Transparent);
    assert_eq!(alpha_hit_test(&map, 1, 1, 0), HitTestResult::Transparent);
    assert_eq!(alpha_hit_test(&map, 1, 0, 2), HitTestResult::Transparent);
}
