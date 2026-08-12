use std::ffi::OsString;
use std::path::PathBuf;

use termielle_core::{
    AppConfig, AssetCatalog, ConfigError, ReducedMotion, RenderMode, VisualState, WindowPosition,
    load_config, save_config_atomic,
};

fn sample_config() -> AppConfig {
    AppConfig {
        scale: 1.5,
        always_on_top: false,
        reduced_motion: ReducedMotion::On,
        ready_hold_ms: 12_000,
        busy_stall_ms: 450_000,
        position: Some(WindowPosition {
            monitor: r"\\.\DISPLAY1".into(),
            x_logical: -120,
            y_logical: 40,
        }),
        render: RenderMode::ColorKey,
        frame_rate: Some(60),
    }
}

fn file_names(directory: &std::path::Path) -> Vec<OsString> {
    let mut names: Vec<OsString> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    names.sort();
    names
}

#[test]
fn invalid_fields_fall_back_individually() {
    let json = br#"{"scale":9.0,"always_on_top":false,"reduced_motion":"off","ready_hold_ms":50}"#;
    let config = AppConfig::from_json(json).unwrap();
    assert_eq!(config.scale, 2.0);
    assert!(!config.always_on_top);
    assert_eq!(config.reduced_motion, ReducedMotion::Off);
    assert_eq!(config.ready_hold_ms, 1_000);
}

#[test]
fn asset_catalog_uses_the_approved_names() {
    let root = tempfile::tempdir().unwrap();
    let catalog = AssetCatalog::new(vec![root.path().to_path_buf()]);
    assert!(
        catalog
            .expected_path_for(VisualState::Thinking)
            .unwrap()
            .ends_with("ai_thingking.gif")
    );
    assert!(
        catalog
            .expected_path_for(VisualState::NeedsInput)
            .unwrap()
            .ends_with("user_typing.gif")
    );
    assert!(
        catalog
            .expected_path_for(VisualState::Ready)
            .unwrap()
            .ends_with("ai_complete_answer.gif")
    );
    assert_eq!(catalog.expected_path_for(VisualState::Failed), None);
}

#[test]
fn defaults_match_the_documented_settings() {
    let config = AppConfig::default();
    assert_eq!(config.scale, 1.0);
    assert!(config.always_on_top);
    assert_eq!(config.reduced_motion, ReducedMotion::System);
    assert_eq!(config.ready_hold_ms, 5_000);
    assert_eq!(config.busy_stall_ms, 300_000);
    assert_eq!(config.position, None);
    assert_eq!(config.render, RenderMode::PerPixel);
}

#[test]
fn render_mode_parses_both_ways() {
    let key = AppConfig::from_json(br#"{"render":"color_key"}"#).unwrap();
    assert_eq!(key.render, RenderMode::ColorKey);

    let pixel = AppConfig::from_json(br#"{"render":"per_pixel"}"#).unwrap();
    assert_eq!(pixel.render, RenderMode::PerPixel);

    assert!(matches!(
        AppConfig::from_json(br#"{"render":"teleport"}"#),
        Err(ConfigError::Malformed(_))
    ));
}

#[test]
fn out_of_range_values_clamp_to_the_nearer_bound() {
    let low =
        AppConfig::from_json(br#"{"scale":0.1,"ready_hold_ms":0,"busy_stall_ms":0}"#).unwrap();
    assert_eq!(low.scale, 0.5);
    assert_eq!(low.ready_hold_ms, 1_000);
    assert_eq!(low.busy_stall_ms, 60_000);

    let high =
        AppConfig::from_json(br#"{"scale":100.0,"ready_hold_ms":999999,"busy_stall_ms":9999999}"#)
            .unwrap();
    assert_eq!(high.scale, 2.0);
    assert_eq!(high.ready_hold_ms, 30_000);
    assert_eq!(high.busy_stall_ms, 3_600_000);

    let inside =
        AppConfig::from_json(br#"{"scale":0.75,"ready_hold_ms":2500,"busy_stall_ms":150000}"#)
            .unwrap();
    assert_eq!(inside.scale, 0.75);
    assert_eq!(inside.ready_hold_ms, 2_500);
    assert_eq!(inside.busy_stall_ms, 150_000);
}

#[test]
fn a_scale_that_overflows_f32_still_clamps() {
    // serde_json widens `1e39` to `f32::INFINITY` rather than rejecting it.
    let config = AppConfig::from_json(br#"{"scale":1e39}"#).unwrap();
    assert_eq!(config.scale, 2.0);

    let config = AppConfig::from_json(br#"{"scale":-1e39}"#).unwrap();
    assert_eq!(config.scale, 0.5);
}

#[test]
fn from_json_reports_malformed_input() {
    assert!(matches!(
        AppConfig::from_json(b"{"),
        Err(ConfigError::Malformed(_))
    ));
    assert!(matches!(
        AppConfig::from_json(br#"{"reduced_motion":"sideways"}"#),
        Err(ConfigError::Malformed(_))
    ));
    assert!(matches!(
        AppConfig::from_json(br#"{"scale":1.0,"nonsense":true}"#),
        Err(ConfigError::Malformed(_))
    ));
}

#[test]
fn atomic_save_round_trips_and_leaves_no_temporary_file() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.json");
    let config = sample_config();

    save_config_atomic(&path, &config).unwrap();

    let bytes = std::fs::read(&path).unwrap();
    serde_json::from_slice::<serde_json::Value>(&bytes).expect("saved file must be valid JSON");
    assert_eq!(load_config(&path).unwrap(), config);
    assert_eq!(
        file_names(directory.path()),
        vec![OsString::from("config.json")]
    );
}

#[test]
fn atomic_save_replaces_an_existing_file() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.json");

    save_config_atomic(&path, &AppConfig::default()).unwrap();

    let replacement = sample_config();
    save_config_atomic(&path, &replacement).unwrap();

    assert_eq!(load_config(&path).unwrap(), replacement);
    assert_eq!(
        file_names(directory.path()),
        vec![OsString::from("config.json")]
    );
}

#[test]
fn saving_an_out_of_range_config_still_reloads() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.json");

    // A caller that mutates a field in memory must not be able to write a file
    // that fails to parse: `f32::NAN` would otherwise serialize as JSON `null`.
    let config = AppConfig {
        scale: f32::NAN,
        ready_hold_ms: 0,
        ..AppConfig::default()
    };
    save_config_atomic(&path, &config).unwrap();

    let reloaded = load_config(&path).unwrap();
    assert_eq!(reloaded.scale, 1.0);
    assert_eq!(reloaded.ready_hold_ms, 1_000);
}

#[test]
fn a_failed_save_leaves_no_temporary_file_behind() {
    let directory = tempfile::tempdir().unwrap();
    // A directory cannot be replaced by a file, so the rename must fail.
    let path = directory.path().join("config.json");
    std::fs::create_dir(&path).unwrap();

    assert!(save_config_atomic(&path, &AppConfig::default()).is_err());
    assert_eq!(
        file_names(directory.path()),
        vec![OsString::from("config.json")]
    );
}

#[test]
fn load_config_recovers_from_a_missing_file() {
    let directory = tempfile::tempdir().unwrap();
    let missing = directory.path().join("absent").join("config.json");
    assert_eq!(load_config(&missing).unwrap(), AppConfig::default());
}

#[test]
fn load_config_recovers_from_a_malformed_file() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.json");

    std::fs::write(&path, b"{ not json at all").unwrap();
    assert_eq!(load_config(&path).unwrap(), AppConfig::default());

    std::fs::write(&path, br#"{"scale":1.0,"unknown_field":true}"#).unwrap();
    assert_eq!(load_config(&path).unwrap(), AppConfig::default());
}

#[test]
fn resolve_returns_the_first_existing_root_in_order() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let catalog = AssetCatalog::new(vec![
        PathBuf::from(first.path()),
        PathBuf::from(second.path()),
    ]);

    assert_eq!(catalog.resolve(VisualState::Idle), None);

    std::fs::write(second.path().join("standby.gif"), b"gif").unwrap();
    assert_eq!(
        catalog.resolve(VisualState::Idle),
        Some(second.path().join("standby.gif"))
    );

    std::fs::write(first.path().join("standby.gif"), b"gif").unwrap();
    assert_eq!(
        catalog.resolve(VisualState::Idle),
        Some(first.path().join("standby.gif"))
    );

    assert_eq!(catalog.resolve(VisualState::Failed), None);
}
