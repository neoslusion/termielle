//! GIF streaming and procedural fallback tests.
//!
//! GIF fixtures are generated in-process with the `gif` crate; no binary
//! fixtures are checked into the repository.

use std::borrow::Cow;

use termielle_app::animation::{AnimationSource, GifAnimation, fallback_frame};
use termielle_core::VisualState;

struct TestFrame {
    palette_index: u8,
    delay_hundredths: u16,
    disposal: gif::DisposalMethod,
    transparent: Option<u8>,
    left: u16,
    top: u16,
    width: u16,
    height: u16,
}

impl TestFrame {
    fn full(palette_index: u8, delay_hundredths: u16, disposal: gif::DisposalMethod) -> Self {
        Self {
            palette_index,
            delay_hundredths,
            disposal,
            transparent: None,
            left: 0,
            top: 0,
            width: 2,
            height: 2,
        }
    }

    fn partial(
        palette_index: u8,
        left: u16,
        top: u16,
        width: u16,
        height: u16,
        disposal: gif::DisposalMethod,
    ) -> Self {
        Self {
            palette_index,
            delay_hundredths: 5,
            disposal,
            transparent: None,
            left,
            top,
            width,
            height,
        }
    }
}

fn write_test_gif(frames: &[TestFrame]) -> tempfile::NamedTempFile {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    {
        let palette = [255, 0, 0, 0, 0, 255, 0, 255, 0];
        let mut encoder = gif::Encoder::new(file.as_file_mut(), 2, 2, &palette).unwrap();
        encoder.set_repeat(gif::Repeat::Infinite).unwrap();
        for spec in frames {
            let mut frame = gif::Frame {
                width: spec.width,
                height: spec.height,
                left: spec.left,
                top: spec.top,
                delay: spec.delay_hundredths,
                dispose: spec.disposal,
                transparent: spec.transparent,
                ..gif::Frame::default()
            };
            frame.buffer = Cow::Owned(vec![
                spec.palette_index;
                (spec.width * spec.height) as usize
            ]);
            encoder.write_frame(&frame).unwrap();
        }
    }
    file.as_file_mut().sync_all().unwrap();
    file
}

#[test]
fn streams_frames_without_retaining_every_decoded_frame() {
    let file = write_test_gif(&[
        TestFrame::full(0, 2, gif::DisposalMethod::Keep),
        TestFrame::full(1, 7, gif::DisposalMethod::Background),
    ]);
    let mut animation = GifAnimation::open(file.path()).unwrap();
    assert_eq!(animation.canvas_bytes(), 2 * 2 * 4);
    assert_eq!(animation.scratch_bytes(), 2 * 2 * 4);
    assert_eq!(animation.next_frame().unwrap().delay_ms, 20);
    assert_eq!(animation.next_frame().unwrap().delay_ms, 70);
    assert_eq!(animation.next_frame().unwrap().loop_index, 1);
}

#[test]
fn composites_partial_frames_into_the_logical_screen() {
    let file = write_test_gif(&[
        TestFrame::full(0, 2, gif::DisposalMethod::Keep),
        TestFrame::partial(2, 1, 1, 1, 1, gif::DisposalMethod::Keep),
    ]);
    let mut animation = GifAnimation::open(file.path()).unwrap();

    let first = animation.next_frame().unwrap();
    assert!(first.contains_pixel_bgra([0, 0, 255, 255]));

    let second = animation.next_frame().unwrap();
    assert_eq!(second.alpha_at(1, 1), 255);
    assert!(second.contains_pixel_bgra([0, 255, 0, 255]));
    assert_eq!(
        second.alpha_at(0, 0),
        255,
        "the base frame must remain visible"
    );
}

#[test]
fn background_disposal_clears_the_previous_rect_before_the_next_frame() {
    let file = write_test_gif(&[
        TestFrame::full(0, 2, gif::DisposalMethod::Keep),
        TestFrame::partial(2, 1, 1, 1, 1, gif::DisposalMethod::Background),
        TestFrame::partial(1, 0, 0, 1, 1, gif::DisposalMethod::Keep),
    ]);
    let mut animation = GifAnimation::open(file.path()).unwrap();

    animation.next_frame().unwrap();
    animation.next_frame().unwrap();
    let third = animation.next_frame().unwrap();

    assert_eq!(
        third.alpha_at(1, 1),
        0,
        "background disposal cleared the green pixel"
    );
    assert_eq!(third.alpha_at(0, 0), 255);
    assert!(third.contains_pixel_bgra([255, 0, 0, 255]));
}

#[test]
fn previous_disposal_restores_the_canvas_under_a_partial_frame() {
    let file = write_test_gif(&[
        TestFrame::full(0, 2, gif::DisposalMethod::Previous),
        TestFrame::partial(2, 1, 1, 1, 1, gif::DisposalMethod::Keep),
        TestFrame::full(0, 2, gif::DisposalMethod::Keep),
    ]);
    let mut animation = GifAnimation::open(file.path()).unwrap();

    animation.next_frame().unwrap();
    animation.next_frame().unwrap();
    let third = animation.next_frame().unwrap();

    // The canvas under the partial green frame was restored from scratch, so
    // the full red frame redraws on a clean background instead of on green.
    assert_eq!(third.alpha_at(1, 1), 255);
    assert!(third.contains_pixel_bgra([0, 0, 255, 255]));
    assert!(
        !third.contains_pixel_bgra([0, 255, 0, 255]),
        "the green frame must be gone before the third frame draws"
    );
}

#[test]
fn reset_restarts_the_animation_at_loop_zero() {
    let file = write_test_gif(&[
        TestFrame::full(0, 2, gif::DisposalMethod::Keep),
        TestFrame::full(1, 2, gif::DisposalMethod::Keep),
    ]);
    let mut animation = GifAnimation::open(file.path()).unwrap();

    animation.next_frame().unwrap();
    animation.next_frame().unwrap();
    animation.next_frame().unwrap();
    animation.next_frame().unwrap();
    assert_eq!(animation.next_frame().unwrap().loop_index, 2);

    animation.reset().unwrap();
    assert_eq!(animation.next_frame().unwrap().loop_index, 0);
}

#[test]
fn rejects_corrupt_input() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("broken.gif");
    std::fs::write(&path, b"definitely not a gif").unwrap();
    assert!(GifAnimation::open(&path).is_err());
}

#[test]
fn failed_fallback_has_visible_red_edge_and_transparent_corners() {
    let frame = fallback_frame(VisualState::Failed, 64);
    assert_eq!(frame.alpha_at(0, 0), 0);
    assert!(frame.contains_pixel_bgra([0, 0, 255, 255]));
}

#[test]
fn every_state_has_a_distinct_fallback_shape() {
    let states = [
        VisualState::Idle,
        VisualState::Thinking,
        VisualState::Working,
        VisualState::NeedsInput,
        VisualState::Ready,
        VisualState::Failed,
    ];
    let frames: Vec<_> = states
        .iter()
        .map(|state| fallback_frame(*state, 64))
        .collect();

    for (index, left) in frames.iter().enumerate() {
        for right in frames.iter().skip(index + 1) {
            assert_ne!(left.pixels_pbgra, right.pixels_pbgra);
        }
    }
}

#[test]
fn gif_source_and_still_source_both_carry_frames() {
    let file = write_test_gif(&[TestFrame::full(0, 2, gif::DisposalMethod::Keep)]);
    let gif = GifAnimation::open(file.path()).unwrap();
    let mut source = AnimationSource::Gif(gif);
    let frame = match &mut source {
        AnimationSource::Gif(animation) => animation.next_frame().unwrap(),
        AnimationSource::Still(_) => panic!("expected a gif source"),
    };
    assert_eq!(frame.width, 2);
    assert_eq!(frame.delay_ms, 20);

    let still = fallback_frame(VisualState::Idle, 32);
    let mut source = AnimationSource::Still(still);
    match &mut source {
        AnimationSource::Still(frame) => {
            assert_eq!(frame.width, 32);
            assert_eq!(frame.delay_ms, 0);
        }
        AnimationSource::Gif(_) => panic!("expected a still source"),
    }
}
