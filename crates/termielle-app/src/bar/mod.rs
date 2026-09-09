//! Waybar-style modular status bar components and hit testing.

pub mod appbar;
pub mod volume;
pub mod workspaces;

pub const HIT_BAR_WORKSPACE_BASE: isize = -100;
pub const HIT_BAR_VOLUME_TOGGLE: isize = -200;
pub const HIT_BAR_ISLAND_PILL: isize = -300;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarHitTarget {
    Workspace(usize),
    ActiveWindow,
    Island,
    Cpu,
    Memory,
    Volume,
    Battery,
    Clock,
    TaskbarToggle,
}
