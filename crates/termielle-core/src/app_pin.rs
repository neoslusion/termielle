//! Explicit, local launch targets for the taskbar's pinned apps.
use serde::{Deserialize, Serialize};

pub const MAX_PINNED_APPS: usize = 32;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum AppLaunchTarget {
    Executable(String),
    AppUserModelId(String),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PinnedApp {
    pub name: String,
    pub target: AppLaunchTarget,
}

impl PinnedApp {
    pub fn key(&self) -> String {
        match &self.target {
            AppLaunchTarget::Executable(path) => {
                format!("exe:{}", path.replace('/', "\\").to_lowercase())
            }
            AppLaunchTarget::AppUserModelId(id) => format!("aumid:{id}"),
        }
    }

    /// Launch only explicit application identities; never window titles,
    /// command lines, arguments, shell commands, or executable host processes.
    pub fn is_valid(&self) -> bool {
        if self.name.trim().is_empty()
            || self.name.chars().count() > 96
            || self.name.chars().any(char::is_control)
        {
            return false;
        }
        match &self.target {
            AppLaunchTarget::Executable(path) => {
                let bytes = path.as_bytes();
                let absolute = (bytes.len() > 3
                    && bytes[0].is_ascii_alphabetic()
                    && bytes[1] == b':'
                    && matches!(bytes[2], b'\\' | b'/'))
                    || path.starts_with("\\\\");
                let leaf = path.rsplit(['\\', '/']).next().unwrap_or("").to_lowercase();
                absolute
                    && path.len() <= 4096
                    && !path
                        .chars()
                        .any(|c| c.is_control() || matches!(c, '"' | '<' | '>' | '|' | '?' | '*'))
                    && leaf.ends_with(".exe")
                    && !matches!(
                        leaf.as_str(),
                        "applicationframehost.exe" | "rundll32.exe" | "dllhost.exe" | "svchost.exe"
                    )
            }
            AppLaunchTarget::AppUserModelId(id) => {
                !id.is_empty()
                    && id.len() <= 256
                    && id
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '!' | '-'))
            }
        }
    }
}

pub(crate) fn sanitize_pins(pins: &mut Vec<PinnedApp>) {
    let mut seen = std::collections::HashSet::new();
    pins.retain(|pin| pin.is_valid() && seen.insert(pin.key()));
    pins.truncate(MAX_PINNED_APPS);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn targets_are_explicit_and_keys_do_not_depend_on_labels() {
        let mut pin = PinnedApp {
            name: "Editor".into(),
            target: AppLaunchTarget::Executable("C:\\Apps\\Editor.exe".into()),
        };
        assert!(pin.is_valid());
        let key = pin.key();
        pin.name = "Work".into();
        pin.target = AppLaunchTarget::Executable("c:/apps/editor.EXE".into());
        assert_eq!(key, pin.key());
        for path in [
            "editor.exe",
            "C:\\Apps\\editor.exe --unsafe",
            "C:\\Windows\\rundll32.exe",
            "C:\\Apps\\editor.exe\0hidden",
        ] {
            pin.target = AppLaunchTarget::Executable(path.into());
            assert!(!pin.is_valid(), "{path:?}");
        }
        pin.target =
            AppLaunchTarget::AppUserModelId("Microsoft.WindowsCalculator_8wekyb3d8bbwe!App".into());
        assert!(pin.is_valid());
        pin.target = AppLaunchTarget::AppUserModelId("shell:evil/command".into());
        assert!(!pin.is_valid());
    }
    #[test]
    fn sanitization_deduplicates_without_reordering() {
        let pin = |name: &str| PinnedApp {
            name: name.into(),
            target: AppLaunchTarget::Executable(format!("C:\\Apps\\{name}.exe")),
        };
        let mut pins = vec![pin("B"), pin("A"), pin("b")];
        sanitize_pins(&mut pins);
        assert_eq!(
            pins.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            ["B", "A"]
        );
        let json = serde_json::to_string(&pins).unwrap();
        assert_eq!(serde_json::from_str::<Vec<PinnedApp>>(&json).unwrap(), pins);
    }
}
