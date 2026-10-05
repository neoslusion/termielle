use termielle_core::{
    AppConfig, AppLaunchTarget, BarPosition, PinnedApp, load_config, save_config_atomic,
};
fn pin(name: &str) -> PinnedApp {
    PinnedApp {
        name: name.into(),
        target: AppLaunchTarget::Executable(format!("C:\\Apps\\{name}.exe")),
    }
}
#[test]
fn older_configs_default_to_no_pins_without_enabling_replacement() {
    let mut value = serde_json::to_value(AppConfig::default()).unwrap();
    value["island"]["bar"]
        .as_object_mut()
        .unwrap()
        .remove("pinned_apps");
    let config: AppConfig = serde_json::from_value(value).unwrap();
    assert!(config.island.bar.pinned_apps.is_empty());
    assert!(!config.island.bar.replace_taskbar);
}
#[test]
fn pins_round_trip_atomically_without_changing_other_preferences() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.json");
    let mut config = AppConfig {
        scale: 1.25,
        ..Default::default()
    };
    config.island.bar.position = BarPosition::Bottom;
    config.island.bar.follow_active_monitor = false;
    config.island.glass.tint = [71, 52, 83, 199];
    config.island.bar.pinned_apps = vec![pin("Editor"), pin("Browser")];
    save_config_atomic(&path, &config).unwrap();
    let loaded = load_config(&path).unwrap();
    assert_eq!(
        serde_json::to_value(loaded).unwrap(),
        serde_json::to_value(config).unwrap()
    );
}
#[test]
fn invalid_duplicates_and_excess_pins_are_sanitized_in_place() {
    let mut config = AppConfig::default();
    config.island.bar.pinned_apps = (0..40).map(|i| pin(&format!("App{i}"))).collect();
    config.island.bar.pinned_apps.insert(0, pin("App2"));
    config.island.bar.pinned_apps.insert(
        0,
        PinnedApp {
            name: "Invalid".into(),
            target: AppLaunchTarget::Executable("cmd.exe /c something".into()),
        },
    );
    config.island.clamp();
    assert_eq!(
        config.island.bar.pinned_apps.len(),
        termielle_core::MAX_PINNED_APPS
    );
    assert_eq!(config.island.bar.pinned_apps[0].name, "App2");
}
