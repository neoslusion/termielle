use std::ffi::OsString;

use termielle_app::{BoundedLog, LogComponent, LogEvent, LogLevel, LogRecord};

const MAX_BYTES: u64 = 1_024;
const BACKUPS: u8 = 2;

fn record(index: u64) -> LogRecord {
    LogRecord {
        timestamp_ms: 1_700_000_000_000 + index,
        level: LogLevel::Warning,
        component: LogComponent::Ipc,
        event: LogEvent::ReadFailed,
        error_code: -2147024809,
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
fn rotation_keeps_only_the_configured_backups_within_the_limit() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("termielle.log");
    let log = BoundedLog::new(path.clone(), MAX_BYTES, BACKUPS);

    let one_record = serde_json::to_vec(&record(0)).unwrap().len() as u64 + 1;
    // Enough records to overflow the limit at least twice over.
    let count = (MAX_BYTES / one_record) * 3 + 3;
    for index in 0..count {
        log.write(&record(index)).unwrap();
    }

    assert_eq!(
        file_names(directory.path()),
        vec![
            OsString::from("termielle.log"),
            OsString::from("termielle.log.1"),
            OsString::from("termielle.log.2"),
        ]
    );

    for name in file_names(directory.path()) {
        let size = std::fs::metadata(directory.path().join(&name))
            .unwrap()
            .len();
        assert!(
            size <= MAX_BYTES + one_record,
            "{name:?} grew to {size} bytes, above the {MAX_BYTES} byte limit plus one record"
        );
    }
}

#[test]
fn records_carry_only_privacy_safe_fields() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("termielle.log");
    let log = BoundedLog::new(path.clone(), MAX_BYTES, BACKUPS);

    log.write(&LogRecord {
        timestamp_ms: 1_700_000_000_000,
        level: LogLevel::Error,
        component: LogComponent::Animation,
        event: LogEvent::DecodeFailed,
        error_code: 5,
    })
    .unwrap();

    let contents = std::fs::read_to_string(&path).unwrap();
    let mut lines = contents.lines();
    let line = lines.next().expect("one record was written");
    assert_eq!(lines.next(), None);

    let value: serde_json::Value = serde_json::from_str(line).unwrap();
    let object = value.as_object().unwrap();
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec!["component", "error_code", "event", "level", "timestamp_ms"]
    );

    assert_eq!(
        object["timestamp_ms"],
        serde_json::json!(1_700_000_000_000u64)
    );
    assert_eq!(object["level"], serde_json::json!("error"));
    assert_eq!(object["component"], serde_json::json!("animation"));
    assert_eq!(object["event"], serde_json::json!("decode_failed"));
    assert_eq!(object["error_code"], serde_json::json!(5));
}

#[test]
fn writing_creates_a_missing_log_directory() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("logs").join("termielle.log");
    let log = BoundedLog::new(path.clone(), MAX_BYTES, BACKUPS);

    log.write(&record(0)).unwrap();

    assert!(path.is_file());
}

#[test]
fn backups_run_from_newest_to_oldest() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("termielle.log");
    // One byte forces a rotation before every append after the first.
    let log = BoundedLog::new(path.clone(), 1, BACKUPS);

    for index in 0..4 {
        log.write(&record(index)).unwrap();
    }

    let timestamp = |name: &str| -> u64 {
        let contents = std::fs::read_to_string(directory.path().join(name)).unwrap();
        let value: serde_json::Value = serde_json::from_str(contents.trim()).unwrap();
        value["timestamp_ms"].as_u64().unwrap()
    };

    assert!(timestamp("termielle.log") > timestamp("termielle.log.1"));
    assert!(timestamp("termielle.log.1") > timestamp("termielle.log.2"));
}

#[test]
fn a_record_larger_than_the_limit_is_still_written() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("termielle.log");
    let log = BoundedLog::new(path.clone(), 1, BACKUPS);

    log.write(&record(0)).unwrap();

    // Rotating an empty file forever would silently discard every record.
    assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 1);
}
