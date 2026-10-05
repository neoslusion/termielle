//! Synthetic navigation prototype: no live app launches, focus, or config writes.
use termielle_app::{
    app::Controller,
    tasks::{TaskIcon, WindowInfo, WorkerUpdate},
};
use termielle_core::{
    AppLaunchTarget, AssetCatalog, BarPosition, IslandConfig, IslandLayout, PinnedApp,
};
fn app(name: &str) -> PinnedApp {
    PinnedApp {
        name: name.into(),
        target: AppLaunchTarget::Executable(format!("C:\\Synthetic\\{name}.exe")),
    }
}
fn main() -> std::io::Result<()> {
    let args = std::env::args().collect::<Vec<_>>();
    let output = args.get(1).expect("output BMP path");
    let mut island = IslandConfig {
        layout: IslandLayout::Bar,
        ..Default::default()
    };
    island.bar.pinned_apps = vec![app("Editor"), app("Browser"), app("Notes")];
    if args.iter().any(|a| a == "--bottom") {
        island.bar.position = BarPosition::Bottom;
    }
    termielle_app::theme::apply_theme(
        &mut island,
        if args.iter().any(|a| a == "--light") {
            "light"
        } else {
            "catppuccin-macchiato"
        },
    );
    let assets = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let mut c = Controller::new_with_island(
        5000,
        60000,
        AssetCatalog::new(vec![assets]),
        true,
        None,
        island,
    );
    c.set_bar_width(if args.iter().any(|a| a == "--narrow") {
        560
    } else {
        1280
    });
    c.set_bar_metrics(
        termielle_app::bar::metrics::Snapshot {
            foreground_hwnd: 101,
            window_title: "Project Alpha — Editor".into(),
            time_str: "14:32".into(),
            battery: (Some(82), false, false),
            ..Default::default()
        },
        1000,
    );
    let mut windows = vec![];
    for (name, base, titles) in [
        (
            "Editor",
            100,
            vec![
                "Project Alpha — Editor",
                "Project Beta — Editor",
                "Tests — Editor",
            ],
        ),
        (
            "Browser",
            200,
            vec!["Documentation — Browser", "Issue tracker — Browser"],
        ),
        (
            "Terminal",
            300,
            vec!["Development — Terminal", "Build — Terminal"],
        ),
        ("File Explorer", 400, vec!["Projects — File Explorer"]),
        ("Music", 500, vec!["Music"]),
        ("Chat", 600, vec!["Team chat"]),
        ("Calculator", 700, vec!["Calculator"]),
        ("Mail", 800, vec!["Mail"]),
        ("Calendar", 900, vec!["Calendar"]),
        ("Design", 1000, vec!["Design workspace"]),
    ] {
        for (i, title) in titles.into_iter().enumerate() {
            let hwnd = base + i as isize + 1;
            windows.push(WindowInfo {
                hwnd,
                process_id: base as u32,
                title: title.into(),
                application: Some(app(name)),
                minimized: hwnd == 102,
            });
        }
    }
    let colors = [
        [174, 126, 231, 255],
        [227, 173, 91, 255],
        [136, 201, 161, 255],
        [205, 163, 240, 255],
        [124, 184, 249, 255],
        [211, 174, 139, 255],
    ];
    let tasks = windows
        .iter()
        .take(6)
        .map(|w| {
            let name = &w.application.as_ref().unwrap().name;
            let color = colors[match name.as_str() {
                "Editor" => 0,
                "Browser" => 1,
                "Terminal" => 2,
                _ => 3,
            }];
            let mut frame = termielle_app::animation::FrameBuffer {
                width: 24,
                height: 24,
                pixels_pbgra: color.repeat(24 * 24),
                delay_ms: 0,
                loop_index: 0,
                scale: 1.0,
            };
            termielle_app::animation::notch::draw_text_in_rect(
                &mut frame,
                &name.chars().next().unwrap().to_string(),
                (0, 0, 24, 24),
                14,
                true,
                [255; 4],
                true,
            );
            TaskIcon {
                hwnd: w.hwnd,
                title: w.title.clone(),
                width: 24,
                height: 24,
                pixels_pbgra: frame.pixels_pbgra,
            }
        })
        .collect();
    c.set_task_update_at(
        WorkerUpdate {
            windows,
            tasks,
            ..Default::default()
        },
        1000,
    );
    if args.iter().any(|a| a == "--chooser") {
        let hit = c
            .click_regions()
            .iter()
            .find(|h| h.0 == -1001)
            .copied()
            .unwrap();
        c.handle_context_click(hit.1 + 10, hit.2 + 10, 1010);
    } else if args.iter().any(|a| a == "--overflow") {
        let hit = c
            .click_regions()
            .iter()
            .rfind(|h| h.0 <= -1000 && h.0 > -2000)
            .copied()
            .unwrap();
        c.handle_click(hit.1 + 10, hit.2 + 10, 1010);
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
    let background = if args.iter().any(|a| a == "--light") {
        225
    } else {
        38
    };
    for y in (0..frame.height).rev() {
        for x in 0..frame.width {
            let i = ((y * frame.width + x) * 4) as usize;
            let pixel = &frame.pixels_pbgra[i..i + 4];
            for ch in &pixel[..3] {
                bmp.push(
                    (u32::from(*ch) + background * (255 - u32::from(pixel[3])) / 255).min(255)
                        as u8,
                );
            }
        }
        bmp.extend(std::iter::repeat_n(0, (stride - frame.width * 3) as usize));
    }
    std::fs::write(output, bmp)
}
