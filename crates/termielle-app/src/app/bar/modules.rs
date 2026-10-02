//! Declarative bar-module metadata and damage classification.
//!
//! Painters remain focused, but module identity, zone ownership, and the
//! data that can invalidate a zone live in one place. This is deliberately
//! typed rather than a script/command language: agent events never execute
//! arbitrary user commands.

use super::types::BarMetricsCache;
use termielle_core::BarConfig;

/// Logical bar zone owning a module.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BarZone {
    Left,
    Center,
    Right,
}

/// Bit mask of regions invalidated by an update.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct BarDamage(u8);

impl BarDamage {
    pub(crate) const NONE: Self = Self(0);
    pub(crate) const LEFT: Self = Self(1 << 0);
    pub(crate) const CENTER: Self = Self(1 << 1);
    pub(crate) const RIGHT: Self = Self(1 << 2);
    pub(crate) const FULL: Self = Self(Self::LEFT.0 | Self::CENTER.0 | Self::RIGHT.0);

    pub(crate) fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub(crate) fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

/// Static descriptor for one supported native module.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BarModule {
    pub(crate) id: &'static str,
    pub(crate) zone: BarZone,
}

pub(crate) const MODULES: &[BarModule] = &[
    BarModule {
        id: "apps",
        zone: BarZone::Left,
    },
    BarModule {
        id: "workspaces",
        zone: BarZone::Left,
    },
    BarModule {
        id: "window",
        zone: BarZone::Left,
    },
    BarModule {
        id: "termielle",
        zone: BarZone::Center,
    },
    BarModule {
        id: "cpu",
        zone: BarZone::Right,
    },
    BarModule {
        id: "memory",
        zone: BarZone::Right,
    },
    BarModule {
        id: "network",
        zone: BarZone::Right,
    },
    BarModule {
        id: "volume",
        zone: BarZone::Right,
    },
    BarModule {
        id: "battery",
        zone: BarZone::Right,
    },
    BarModule {
        // A menu-bar item between the status icons and the clock, as macOS
        // has it. An unregistered module id is silently ignored, so this has
        // to be in the table for the name to mean anything.
        id: "control_center",
        zone: BarZone::Right,
    },
    BarModule {
        id: "clock",
        zone: BarZone::Right,
    },
];

/// Returns whether a known native module is enabled in its configured zone.
pub(crate) fn module_enabled(config: &BarConfig, zone: BarZone, id: &str) -> bool {
    let Some(module) = MODULES
        .iter()
        .find(|module| module.id == id && module.zone == zone)
    else {
        return false;
    };
    let list = match module.zone {
        BarZone::Left => &config.modules_left,
        BarZone::Center => &config.modules_center,
        BarZone::Right => &config.modules_right,
    };
    list.iter().any(|name| name == module.id)
}

/// Classifies a metrics snapshot delta into the regions it can affect.
pub(crate) fn metrics_damage(
    previous: Option<&BarMetricsCache>,
    next: &BarMetricsCache,
) -> BarDamage {
    let Some(previous) = previous else {
        return BarDamage::FULL;
    };
    let mut damage = BarDamage::NONE;
    if previous.workspaces != next.workspaces
        || previous.foreground_hwnd != next.foreground_hwnd
        || previous.window_title != next.window_title
        || previous.foreground_icon != next.foreground_icon
    {
        damage = damage.union(BarDamage::LEFT);
    }
    if previous.time_str != next.time_str
        || previous.battery != next.battery
        || previous.volume != next.volume
        || previous.connectivity != next.connectivity
        || previous.memory_pct != next.memory_pct
        || previous.cpu_pct != next.cpu_pct
    {
        damage = damage.union(BarDamage::RIGHT);
    }
    damage
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bar::metrics::Snapshot;
    use termielle_core::BarConfig;

    #[test]
    fn descriptors_own_the_expected_zones() {
        let config = BarConfig::default();
        assert!(module_enabled(&config, BarZone::Left, "apps"));
        assert!(module_enabled(&config, BarZone::Center, "termielle"));
        assert!(module_enabled(&config, BarZone::Right, "clock"));
        assert!(!module_enabled(&config, BarZone::Right, "termielle"));
    }

    #[test]
    fn snapshot_deltas_are_routed_to_one_zone() {
        let empty = Snapshot::empty();
        let mut volume = empty.clone();
        volume.volume.level = 42;
        assert_eq!(metrics_damage(Some(&empty), &volume), BarDamage::RIGHT);

        let mut title = empty.clone();
        title.window_title = "editor".into();
        assert_eq!(metrics_damage(Some(&empty), &title), BarDamage::LEFT);
        assert_eq!(metrics_damage(Some(&empty), &empty), BarDamage::NONE);

        let mut icon = empty.clone();
        icon.foreground_icon = Some(crate::tasks::TaskIcon {
            hwnd: 1,
            title: "editor".into(),
            width: 1,
            height: 1,
            pixels_pbgra: vec![20, 80, 180, 255],
        });
        assert_eq!(metrics_damage(Some(&empty), &icon), BarDamage::LEFT);
    }

    #[test]
    fn combined_metric_damage_rebuilds_both_side_zones() {
        let empty = Snapshot::empty();
        let mut both = empty.clone();
        both.window_title = "editor".into();
        both.time_str = "12:01".into();
        let damage = metrics_damage(Some(&empty), &both);
        assert!(damage.contains(BarDamage::LEFT));
        assert!(damage.contains(BarDamage::RIGHT));
        assert!(!damage.contains(BarDamage::CENTER));
    }
}
