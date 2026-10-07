use termielle_core::{AppConfig, IslandLayout, load_config, save_config_atomic};

#[test]
fn old_profiles_keep_the_name_visible_and_false_survives_all_layouts() {
    assert!(AppConfig::from_json(b"{}").unwrap().island.show_name);
    assert!(
        AppConfig::from_json(br#"{"island":{"layout":"notch"}}"#)
            .unwrap()
            .island
            .show_name
    );
    for layout in ["bar", "notch", "island", "classic"] {
        let json = format!(r#"{{"island":{{"layout":"{layout}","show_name":false}}}}"#);
        assert!(
            !AppConfig::from_json(json.as_bytes())
                .unwrap()
                .island
                .show_name
        );
    }
    assert!(AppConfig::from_json(br#"{"island":{"show_name":"false"}}"#).is_err());
}

#[test]
fn atomic_explicit_profile_round_trip_preserves_the_name_and_material() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("explicit-profile.json");
    let mut config = AppConfig::default();
    config.island.layout = IslandLayout::Notch;
    config.island.glass.tint = [31, 41, 59, 180];
    config.island.glass.blur_radius = 0;
    config.island.show_name = false;
    save_config_atomic(&path, &config).unwrap();
    assert_eq!(load_config(&path).unwrap(), config);
    config.island.show_name = true;
    save_config_atomic(&path, &config).unwrap();
    assert_eq!(load_config(&path).unwrap(), config);
    assert!(!directory.path().join("config.json").exists());
}
