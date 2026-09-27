//! Export an isolated controller render for visual review; no desktop hooks or config writes.
use termielle_app::app::Controller;
use termielle_app::tasks::{MediaInfo, TaskIcon, WorkerUpdate};
use termielle_core::{AssetCatalog, EventKind, EventMessage, IslandConfig, IslandLayout, Source};

fn main() -> std::io::Result<()> {
    let output = std::env::args().nth(1).expect("output BMP path required");
    let state = std::env::args().nth(2).unwrap_or_else(|| "thinking".into());
    let compact = std::env::args().any(|arg| arg == "--compact");
    let elapsed = std::env::args()
        .find_map(|arg| arg.strip_prefix("--elapsed=").map(str::to_owned))
        .map(|value| value.parse::<u64>().expect("elapsed must be milliseconds"));
    let mut config = IslandConfig {
        layout: if std::env::args().any(|arg| arg == "--bar") {
            IslandLayout::Bar
        } else {
            IslandLayout::Island
        },
        auto_hide: false,
        widgets: vec![
            "face".into(),
            "agents".into(),
            "music".into(),
            "tasks".into(),
        ],
        show_tasks: true,
        ..Default::default()
    };
    if std::env::args().any(|arg| arg == "--light") {
        termielle_app::theme::apply_theme(&mut config, "light");
    }
    let assets = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let mut controller = Controller::new_with_island(
        5000,
        60000,
        AssetCatalog::new(vec![assets]),
        elapsed.is_none(),
        None,
        config,
    );
    if state == "media" {
        controller.set_task_update(WorkerUpdate {
            media: Some(MediaInfo {
                title: "A quieter afternoon".into(),
                artist: "Review artist".into(),
                app: "Music".into(),
                playing: true,
                ..Default::default()
            }),
            ..Default::default()
        });
    } else if state == "tasks" {
        controller.set_task_update(WorkerUpdate {
            tasks: (1..=4)
                .map(|index| TaskIcon {
                    hwnd: index,
                    title: format!("Review window {index}"),
                    width: 16,
                    height: 16,
                    pixels_pbgra: [120, 140, 160, 255].repeat(16 * 16),
                })
                .collect(),
            ..Default::default()
        });
    } else if state != "idle" {
        controller.handle_event(
            EventMessage {
                version: termielle_core::PROTOCOL_VERSION,
                source: Source::parse("claude").unwrap(),
                session_id: "visual-review".into(),
                event: match state.as_str() {
                    "working" => EventKind::ThinkingEnded,
                    "ready" => EventKind::TurnCompleted,
                    "failed" => EventKind::TurnFailed,
                    "waiting" => EventKind::NeedsInput,
                    "thinking" => EventKind::PromptSubmitted,
                    _ => panic!("expected idle, thinking, working, ready, failed, or waiting"),
                },
                timestamp_ms: 10000,
            },
            10000,
        );
    }
    if !compact {
        controller.toggle_expand(10000);
    }
    if let Some(elapsed) = elapsed {
        for time in (0..=elapsed).step_by(16) {
            controller.on_timer(10000 + time);
        }
        controller.on_timer(10000 + elapsed);
    }
    let frame = controller.current_frame();
    let stride = (frame.width * 3 + 3) & !3;
    let size = 54 + stride * frame.height;
    let mut bmp = Vec::with_capacity(size as usize);
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&size.to_le_bytes());
    bmp.extend_from_slice(&[0; 4]);
    bmp.extend_from_slice(&54u32.to_le_bytes());
    bmp.extend_from_slice(&40u32.to_le_bytes());
    bmp.extend_from_slice(&frame.width.to_le_bytes());
    bmp.extend_from_slice(&frame.height.to_le_bytes());
    bmp.extend_from_slice(&1u16.to_le_bytes());
    bmp.extend_from_slice(&24u16.to_le_bytes());
    bmp.extend_from_slice(&[0; 24]);
    for y in (0..frame.height).rev() {
        for x in 0..frame.width {
            let i = ((y * frame.width + x) * 4) as usize;
            let pixel = &frame.pixels_pbgra[i..i + 4];
            for channel in &pixel[..3] {
                bmp.push(
                    (u32::from(*channel) + 48 * (255 - u32::from(pixel[3])) / 255).min(255) as u8,
                );
            }
        }
        bmp.extend(std::iter::repeat_n(0, (stride - frame.width * 3) as usize));
    }
    std::fs::write(output, bmp)
}
