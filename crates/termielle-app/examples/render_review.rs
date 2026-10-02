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
    if std::env::args().any(|arg| arg == "--macchiato") {
        config.theme = "catppuccin-macchiato".into();
        termielle_app::theme::apply_theme(&mut config, "catppuccin-macchiato");
    }
    if std::env::args().any(|arg| arg == "--replace") {
        config.bar.replace_taskbar = true;
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
    if state == "notification" {
        controller.trigger_alert(
            termielle_app::app::AlertKind::System,
            "Dinner tonight?",
            "Messages: Are we still on for 7?",
            [240, 154, 92, 255],
            6_000,
            10_000,
            "review:notification",
        );
    } else if state == "media" {
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
                    _ => panic!(
                        "expected idle, thinking, working, ready, failed, waiting, or notification"
                    ),
                },
                timestamp_ms: 10000,
            },
            10000,
        );
    }
    // Bar review: a real width, the shell controls, and the metrics they sit
    // next to, so the left zone can be judged in place.
    if controller.island_config().is_bar() {
        if let Some(width) = std::env::args()
            .find_map(|arg| arg.strip_prefix("--width=").map(str::to_owned))
            .map(|value| value.parse::<u32>().expect("width must be pixels"))
        {
            controller.set_bar_width(width);
        }
        let battery = std::env::args()
            .find_map(|arg| arg.strip_prefix("--battery=").map(str::to_owned))
            .map(|value| {
                let (percent, charging) = match value.split_once('/') {
                    Some((percent, "charging")) => (percent, true),
                    _ => (value.as_str(), false),
                };
                (
                    Some(
                        percent
                            .parse::<u8>()
                            .expect("battery percent must be a number"),
                    ),
                    charging,
                    true,
                )
            });
        let volume = std::env::args()
            .find_map(|arg| arg.strip_prefix("--volume=").map(str::to_owned))
            .map(|value| value.parse::<u8>().expect("volume must be a number"))
            .map(|level| termielle_app::bar::volume::VolumeSnapshot {
                level,
                muted: level == 0,
            });
        let metrics = termielle_app::bar::metrics::Snapshot {
            workspaces: termielle_app::bar::workspaces::WorkspaceSnapshot {
                total: 4,
                active: 2,
            },
            foreground_hwnd: if std::env::args().any(|arg| arg == "--tasks") {
                2
            } else {
                0
            },
            window_title: "Editor — main.rs — termielle".into(),
            foreground_icon: std::env::args()
                .any(|arg| arg == "--macchiato")
                .then(|| TaskIcon {
                    hwnd: 1,
                    title: "Editor — main.rs — termielle".into(),
                    width: 16,
                    height: 16,
                    pixels_pbgra: [196, 173, 138, 255].repeat(16 * 16),
                }),
            time_str: "14:32".into(),
            battery: battery.unwrap_or((None, false, false)),
            volume: volume.unwrap_or(termielle_app::bar::volume::VolumeSnapshot {
                level: 64,
                muted: false,
            }),
            connectivity: termielle_app::bar::connectivity::ConnectivitySnapshot {
                network: termielle_app::bar::connectivity::NetworkState::Online,
                bluetooth: termielle_app::bar::connectivity::BluetoothState::On,
            },
            cpu_pct: 24,
            memory_pct: 58,
            ..Default::default()
        };
        controller.set_bar_metrics(metrics.clone(), 10000);
        if std::env::args().any(|arg| arg == "--volume-feedback") {
            let mut baseline = metrics.clone();
            baseline.volume.muted = !metrics.volume.muted;
            controller.set_bar_metrics(baseline, 10000);
            controller.set_bar_metrics(metrics, 10000);
        }
        if std::env::args().any(|arg| arg == "--tasks") {
            controller.set_task_update(WorkerUpdate {
                tasks: (1..=3)
                    .map(|index| TaskIcon {
                        hwnd: index,
                        title: format!("Review window {index}"),
                        width: 16,
                        height: 16,
                        pixels_pbgra: [
                            (60 + index as u8 * 50),
                            (110 + index as u8 * 30),
                            (160 + index as u8 * 20),
                            255,
                        ]
                        .repeat(16 * 16),
                    })
                    .collect(),
                ..Default::default()
            });
        }
    }
    if std::env::args().any(|arg| arg == "--panel") {
        controller.open_control_panel(10000);
    }
    if std::env::args().any(|arg| arg == "--notifications") {
        for (index, source, title, body) in [
            (1, "Messages", "A new message", "Dinner tonight at seven?"),
            (
                2,
                "Codex",
                "Agent finished",
                "The build completed successfully.",
            ),
        ] {
            controller.trigger_alert_from(
                termielle_app::app::AlertKind::System,
                source,
                title,
                body,
                [180, 120, 225, 255],
                100,
                10000,
                format!("review:{index}"),
            );
        }
        if let Some((_, x, y, width, height)) = controller
            .click_regions()
            .iter()
            .find(|(id, ..)| *id == termielle_app::app::HIT_CARD_NOTIFICATIONS)
            .copied()
        {
            controller.handle_click(x + width as i32 / 2, y + height as i32 / 2, 10000);
        }
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
