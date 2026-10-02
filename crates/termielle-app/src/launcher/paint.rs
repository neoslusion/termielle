use super::model::Model;
use crate::animation::{FrameBuffer, icons, notch};
use termielle_core::GlassConfig;

pub(super) const WIDTH: u32 = 640;
pub(super) const ROW_TOP: i32 = 92;
pub(super) const ROW_HEIGHT: u32 = 52;

pub(super) fn height(model: &Model) -> u32 {
    ROW_TOP as u32 + model.results.len().max(1) as u32 * ROW_HEIGHT + 40
}

pub(super) fn render(
    model: &Model,
    glass: &GlassConfig,
    scale: f32,
    hotkey_available: bool,
) -> FrameBuffer {
    let width = (WIDTH as f32 * scale).round() as u32;
    let height = (height(model) as f32 * scale).round() as u32;
    let mut background = glass.tint;
    background[3] = 255;
    let mut frame = FrameBuffer {
        width,
        height,
        pixels_pbgra: background.repeat((width * height) as usize),
        delay_ms: 0,
        loop_index: 0,
        scale,
    };
    let (primary, secondary) = notch::ink_pair(glass);
    icons::draw_icon(&mut frame, icons::SEARCH, 24, 27, 28, primary);
    notch::fill_rect_pub(&mut frame, 20, 76, WIDTH - 40, 1, [24, 24, 24, 24]);
    if model.results.is_empty() {
        let message = if model.indexing {
            "Indexing installed apps…"
        } else {
            "No matching apps"
        };
        notch::draw_text(
            &mut frame,
            message,
            24,
            ROW_TOP + 16,
            WIDTH - 48,
            14,
            false,
            secondary,
        );
    }
    for (position, index) in model.results.iter().enumerate() {
        let app = &model.apps[*index];
        let row_y = ROW_TOP + position as i32 * ROW_HEIGHT as i32;
        if position == model.selected {
            notch::draw_rounded_rect(
                &mut frame,
                12,
                row_y,
                WIDTH - 24,
                ROW_HEIGHT - 2,
                10,
                [255, 255, 255, 24],
                [255, 255, 255, 22],
            );
        }
        if let Some(icon) = &app.icon {
            notch::blit_scaled(&mut frame, icon, 24, row_y + 7, 36, 36);
        } else {
            icons::draw_icon(
                &mut frame,
                icons::DEVICE_DESKTOP,
                28,
                row_y + 11,
                28,
                secondary,
            );
        }
        notch::draw_text(
            &mut frame,
            &app.name,
            76,
            row_y + 6,
            WIDTH - 156,
            15,
            true,
            primary,
        );
        notch::draw_text(
            &mut frame,
            "Application",
            76,
            row_y + 28,
            WIDTH - 156,
            10,
            false,
            secondary,
        );
        if position == model.selected {
            notch::draw_text(
                &mut frame,
                "↵",
                WIDTH as i32 - 46,
                row_y + 15,
                24,
                17,
                false,
                secondary,
            );
        }
    }
    let footer = if let Some(error) = &model.error {
        error.as_str()
    } else if model.launching {
        "Opening application…"
    } else if !hotkey_available {
        "Alt+Space is in use · Open from Today → Search"
    } else {
        "↑↓ Select    Enter Open    Esc Close                 Alt+Space"
    };
    notch::draw_text(
        &mut frame,
        footer,
        24,
        (height as f32 / scale).round() as i32 - 29,
        WIDTH - 48,
        11,
        false,
        secondary,
    );
    frame
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launcher::model::App;

    #[test]
    #[ignore = "Writes a live app-catalog render for local visual review"]
    fn live_launcher_visual_review() {
        let _apartment =
            crate::apartment::Apartment::new(windows::Win32::System::WinRT::RO_INIT_SINGLETHREADED)
                .unwrap();
        let mut model = Model::default();
        model.set_apps(super::super::catalog::enumerate().unwrap());
        model.set_query("chrome".into());
        for index in model.results.clone() {
            model.apps[index].icon = super::super::catalog::read_icon(&model.apps[index].target);
        }
        let mut island = termielle_core::IslandConfig::default();
        crate::theme::apply_theme(&mut island, "catppuccin-macchiato");
        let mut frame = render(&model, &island.glass, 1.5, true);
        let primary = notch::ink_pair(&island.glass).0;
        notch::draw_text(
            &mut frame,
            &model.query,
            66,
            24,
            WIDTH - 90,
            22,
            false,
            primary,
        );
        let size = 54 + frame.width * frame.height * 4;
        let mut bmp = Vec::new();
        bmp.extend_from_slice(b"BM");
        bmp.extend_from_slice(&size.to_le_bytes());
        bmp.extend_from_slice(&[0; 4]);
        bmp.extend_from_slice(&54u32.to_le_bytes());
        bmp.extend_from_slice(&40u32.to_le_bytes());
        bmp.extend_from_slice(&frame.width.to_le_bytes());
        bmp.extend_from_slice(&(-(frame.height as i32)).to_le_bytes());
        bmp.extend_from_slice(&1u16.to_le_bytes());
        bmp.extend_from_slice(&32u16.to_le_bytes());
        bmp.extend_from_slice(&[0; 24]);
        bmp.extend_from_slice(&frame.pixels_pbgra);
        let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/launcher-review.bmp");
        std::fs::write(&output, bmp).unwrap();
        println!("Review: {}", output.display());
    }

    #[test]
    fn result_count_controls_height_and_render_stays_premultiplied_at_high_dpi() {
        let mut model = Model::default();
        let empty_height = height(&model);
        model.set_apps(
            (0..6)
                .map(|index| App {
                    name: format!("App {index}"),
                    key: format!("app {index}"),
                    target: vec![0, 0],
                    icon: None,
                })
                .collect(),
        );
        assert!(height(&model) > empty_height);
        let frame = render(&model, &GlassConfig::default(), 1.5, true);
        assert_eq!(frame.width, WIDTH * 3 / 2);
        assert_eq!(frame.height, height(&model) * 3 / 2);
        assert!(
            frame
                .pixels_pbgra
                .chunks_exact(4)
                .all(|pixel| pixel[..3].iter().all(|channel| *channel <= pixel[3]))
        );
        model.set_query("missing".into());
        assert_eq!(height(&model), empty_height);
    }
}
