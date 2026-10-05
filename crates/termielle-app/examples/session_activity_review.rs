//! Synthetic activity/picker render. No live window activation or config writes.
use termielle_app::app::Controller;
use termielle_app::tasks::{MediaInfo, WindowInfo, WorkerUpdate};
use termielle_core::{
    AssetCatalog, BarPosition, EventKind, EventMessage, IslandConfig, IslandLayout, Source,
};

fn main() -> std::io::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let output = args.get(1).expect("output BMP path required");
    let mut config = IslandConfig {
        layout: if args.iter().any(|a| a == "--bar") {
            IslandLayout::Bar
        } else {
            IslandLayout::Island
        },
        expanded_width: 360,
        widgets: vec!["face".into(), "agents".into(), "music".into()],
        ..IslandConfig::default()
    };
    if args.iter().any(|a| a == "--bottom") {
        config.bar.position = BarPosition::Bottom;
    }
    termielle_app::theme::apply_theme(
        &mut config,
        if args.iter().any(|a| a == "--light") {
            "light"
        } else {
            "catppuccin-macchiato"
        },
    );
    let assets = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let mut c = Controller::new_with_island(
        5_000,
        600_000,
        AssetCatalog::new(vec![assets]),
        true,
        None,
        config,
    );
    c.set_bar_width(1000);
    for (source, id, event, at) in [
        ("claude", "review-approval", EventKind::NeedsInput, 30_000),
        ("codex", "review-tests", EventKind::ThinkingEnded, 65_000),
        (
            "opencode",
            "review-reasoning",
            EventKind::ThinkingStarted,
            90_000,
        ),
        (
            "claude",
            "review-completed",
            EventKind::TurnCompleted,
            99_000,
        ),
        ("codex", "review-idle", EventKind::SessionStarted, 95_000),
    ] {
        c.handle_event(
            EventMessage {
                version: termielle_core::PROTOCOL_VERSION,
                source: Source::parse(source).unwrap(),
                session_id: id.into(),
                event,
                timestamp_ms: at,
            },
            100_000,
        );
    }
    c.set_task_update_at(
        WorkerUpdate {
            windows: (1..=8)
                .map(|i| WindowInfo {
                    hwnd: i,
                    process_id: 17,
                    title: format!("Terminal {i} — explicitly selected workspace"),
                    application: None,
                    minimized: false,
                })
                .collect(),
            media: args.iter().any(|a| a == "--media").then(|| MediaInfo {
                title: "Review music".into(),
                playing: true,
                ..Default::default()
            }),
            ..Default::default()
        },
        100_000,
    );
    c.toggle_expand(100_000);
    if args.iter().any(|a| a == "--picker") {
        let hit = c
            .click_regions()
            .iter()
            .find(|hit| hit.0 == -600)
            .copied()
            .expect("activity row");
        c.handle_click(hit.1 + 10, hit.2 + 10, 100_000);
    }
    let frame = c.current_frame();
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
