use termielle_core::{AppConfig, MonitorSelection, load_config, save_config_atomic};
#[test]
fn old_profiles_keep_legacy_monitor_and_visibility_policy() {
    let config =
        AppConfig::from_json(br#"{"island":{"bar":{"follow_active_monitor":false}}}"#).unwrap();
    assert_eq!(config.island.monitor, MonitorSelection::Automatic);
    assert!(!config.island.hide_on_fullscreen);
    assert!(!config.island.bar.follow_active_monitor);
    for json in [
        br#"{"island":{"monitor":{"mode":"other"}}}"#.as_slice(),
        br#"{"island":{"hide_on_fullscreen":"yes"}}"#.as_slice(),
    ] {
        assert!(AppConfig::from_json(json).is_err());
    }
}
#[test]
fn explicit_display_and_fullscreen_round_trip_preserve_material_and_shell_policy() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("custom.json");
    for choice in [
        MonitorSelection::Automatic,
        MonitorSelection::Primary,
        MonitorSelection::Pointer,
        MonitorSelection::Named(r"\\.\DISPLAY9".into()),
    ] {
        let mut config = AppConfig::default();
        config.island.monitor = choice;
        config.island.hide_on_fullscreen = true;
        config.island.glass.tint = [37, 41, 59, 213];
        config.island.glass.blur_radius = 7;
        config.island.bar.follow_active_monitor = false;
        config.island.bar.reserve_space = true;
        save_config_atomic(&path, &config).unwrap();
        assert_eq!(load_config(&path).unwrap(), config);
    }
}
