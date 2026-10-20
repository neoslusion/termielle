use termielle_core::IslandConfig;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Palette {
    pub surface: [u8; 4],
    pub raised: [u8; 4],
    pub border: [u8; 4],
    pub hover_border: [u8; 4],
    pub primary: [u8; 4],
    pub secondary: [u8; 4],
    pub accent: [u8; 4],
    pub selection: [u8; 4],
    pub selection_secondary: [u8; 4],
}

/// How far a fully pressed control's fill moves toward the primary ink.
pub(crate) const PRESS_MIX: f32 = 0.09;

impl Palette {
    /// `fill` as it looks while held down: lifted toward the primary ink, so a
    /// control answers the pointer on the frame it goes down.
    pub(crate) fn pressed(&self, fill: [u8; 4]) -> [u8; 4] {
        self.press_lift(fill, 1.0)
    }

    /// `fill` lifted by `amount` (0..=1) of a full press.
    pub(crate) fn press_lift(&self, fill: [u8; 4], amount: f32) -> [u8; 4] {
        std::array::from_fn(|index| {
            (f32::from(fill[index])
                + (f32::from(self.primary[index]) - f32::from(fill[index])) * PRESS_MIX * amount)
                .round() as u8
        })
    }

    pub fn ink_pair(island: &IslandConfig) -> ([u8; 4], [u8; 4]) {
        if island.theme == "macos-dark" {
            ([247, 245, 245, 255], [198, 193, 193, 245])
        } else {
            crate::animation::notch::ink_pair(&island.glass)
        }
    }

    pub fn for_island(island: &IslandConfig) -> Self {
        let (primary, secondary) = Self::ink_pair(island);
        if island.theme == "macos-dark" {
            return Self {
                surface: [255, 255, 255, 18],
                raised: [255, 255, 255, 32],
                border: [255, 255, 255, 16],
                hover_border: [255, 255, 255, 30],
                primary,
                secondary,
                accent: [255, 132, 10, 255],
                selection: [255, 112, 0, 230],
                selection_secondary: [247, 245, 245, 255],
            };
        }
        Self {
            surface: [255, 255, 255, 20],
            raised: [255, 255, 255, 32],
            border: [0; 4],
            hover_border: [0; 4],
            primary,
            secondary,
            accent: crate::system::accent_color_bgra(),
            selection: [255, 255, 255, 24],
            selection_secondary: secondary,
        }
    }

    pub fn for_bar(island: &IslandConfig) -> Self {
        if island.theme == "catppuccin-macchiato" {
            return Self {
                surface: [79, 58, 54, 232],
                raised: [100, 77, 73, 255],
                border: [246, 160, 198, 44],
                hover_border: [246, 160, 198, 100],
                primary: [245, 211, 202, 255],
                secondary: [246, 160, 198, 255],
                accent: [127, 169, 245, 255],
                selection: [100, 77, 73, 255],
                selection_secondary: [246, 160, 198, 255],
            };
        }
        Self::for_island(island)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_uses_neutral_ink_and_a_blue_accent_independent_of_windows() {
        let island = IslandConfig {
            theme: "macos-dark".into(),
            ..Default::default()
        };
        let palette = Palette::for_island(&island);
        assert_eq!(palette, Palette::for_bar(&island));
        assert_eq!(palette.accent, [255, 132, 10, 255]);
        assert!(palette.accent[0] > palette.accent[2]);
        assert!(palette.primary[..3].iter().all(|channel| *channel > 240));
        assert!(palette.raised[3] > palette.surface[3]);
    }
}
