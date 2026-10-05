//! Draft/commit rules shared by the native preferences UI and tests.
use termielle_core::IslandConfig;

#[derive(Clone, Debug)]
pub enum Request {
    Preview(Box<IslandConfig>),
    Apply {
        baseline: Box<IslandConfig>,
        draft: Box<IslandConfig>,
    },
    Revert,
    TurnOff,
}

pub fn validate(mut draft: IslandConfig, baseline: &IslandConfig) -> Result<IslandConfig, String> {
    // Review builds never silently remove Windows' tray or accessibility fallback.
    if draft.bar.replace_taskbar && !baseline.bar.replace_taskbar {
        return Err("Taskbar replacement remains disabled until tray, accessibility, monitor and recording acceptance is complete.".into());
    }
    if draft.theme.trim().is_empty() || draft.theme.chars().any(char::is_control) {
        return Err("Choose a valid theme name.".into());
    }
    for (zone, allowed) in [
        (
            &draft.bar.modules_left,
            &["apps", "window", "workspaces"][..],
        ),
        (
            &draft.bar.modules_right,
            &[
                "network",
                "volume",
                "battery",
                "control_center",
                "clock",
                "cpu",
                "memory",
            ][..],
        ),
    ] {
        if zone.iter().any(|name| !allowed.contains(&name.as_str())) {
            return Err("Unknown item or item in the wrong bar zone.".into());
        }
        let mut seen = std::collections::HashSet::new();
        if zone.iter().any(|name| !seen.insert(name)) {
            return Err("Each bar item may appear only once per zone.".into());
        }
    }
    if !draft.bar.modules_left.iter().any(|name| name == "apps") {
        return Err("Keep Apps enabled so navigation and preferences remain reachable.".into());
    }
    if !["control_center", "clock"]
        .iter()
        .all(|name| draft.bar.modules_right.iter().any(|item| item == name))
    {
        return Err(
            "Keep Control Center and Clock enabled while reviewing replacement features.".into(),
        );
    }
    draft.clamp();
    Ok(draft)
}

pub fn prepare_apply(
    baseline: &IslandConfig,
    current: &IslandConfig,
    draft: IslandConfig,
) -> Result<IslandConfig, String> {
    if baseline != current {
        return Err("Preferences changed outside this window. Revert to load the latest profile before applying.".into());
    }
    validate(draft, baseline)
}

pub fn move_pin(config: &mut IslandConfig, index: usize, delta: i32) -> Option<usize> {
    let target = index.checked_add_signed(delta as isize)?;
    if index >= config.bar.pinned_apps.len() || target >= config.bar.pinned_apps.len() {
        return None;
    }
    config.bar.pinned_apps.swap(index, target);
    Some(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use termielle_core::{AppLaunchTarget, PinnedApp};
    #[test]
    fn validation_preserves_custom_material_and_unedited_values() {
        let mut original = IslandConfig::default();
        original.glass.tint = [58, 39, 36, 224];
        original.y_offset = 37;
        let mut draft = original.clone();
        draft.bar.height = 44;
        let applied = prepare_apply(&original, &original, draft).unwrap();
        assert_eq!(applied.glass, original.glass);
        assert_eq!(applied.y_offset, 37);
        assert_eq!(original.bar.height, 36);
    }
    #[test]
    fn stale_drafts_do_not_overwrite_external_changes() {
        let baseline = IslandConfig::default();
        let mut current = baseline.clone();
        current.expand_on_hover = false;
        assert!(prepare_apply(&baseline, &current, baseline.clone()).is_err());
    }
    #[test]
    fn unsafe_replacement_and_hidden_entry_points_are_rejected() {
        let baseline = IslandConfig::default();
        let mut draft = baseline.clone();
        draft.bar.replace_taskbar = true;
        assert!(validate(draft, &baseline).is_err());
        let mut draft = baseline.clone();
        draft.bar.modules_left.clear();
        assert!(validate(draft, &baseline).is_err());
        let mut draft = baseline.clone();
        draft.bar.modules_right.push("clock".into());
        assert!(validate(draft, &baseline).is_err());
    }
    #[test]
    fn pins_move_in_the_draft_only_and_bounds_are_checked() {
        let mut config = IslandConfig::default();
        config.bar.pinned_apps = (0..3)
            .map(|i| PinnedApp {
                name: format!("App {i}"),
                target: AppLaunchTarget::Executable(format!("C:\\Apps\\App{i}.exe")),
            })
            .collect();
        let baseline = config.clone();
        assert_eq!(move_pin(&mut config, 1, -1), Some(0));
        assert_eq!(config.bar.pinned_apps[0], baseline.bar.pinned_apps[1]);
        assert_eq!(move_pin(&mut config, 0, -1), None);
        assert_eq!(move_pin(&mut config, 20, 1), None);
        assert_eq!(baseline.bar.pinned_apps[0].name, "App 0");
    }
}
