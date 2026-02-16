// internal crates
use miru_agent::logs::{self, LogLevel, Options};

// external crates
use std::collections::HashSet;

// ========================= deserialize ========================= //

#[test]
fn deserialize_log_level() {
    struct TestCase {
        _name: &'static str,
        input: &'static str,
        expected: LogLevel,
    }

    let test_cases = vec![
        TestCase {
            _name: "trace",
            input: "\"trace\"",
            expected: LogLevel::Trace,
        },
        TestCase {
            _name: "debug",
            input: "\"debug\"",
            expected: LogLevel::Debug,
        },
        TestCase {
            _name: "info",
            input: "\"info\"",
            expected: LogLevel::Info,
        },
        TestCase {
            _name: "warn",
            input: "\"warn\"",
            expected: LogLevel::Warn,
        },
        TestCase {
            _name: "warning",
            input: "\"warning\"",
            expected: LogLevel::Warn,
        },
        TestCase {
            _name: "error",
            input: "\"error\"",
            expected: LogLevel::Error,
        },
        TestCase {
            _name: "Case-insensitive trace",
            input: "\"TRaCE\"",
            expected: LogLevel::Trace,
        },
        TestCase {
            _name: "Case-insensitive debug",
            input: "\"DEbuG\"",
            expected: LogLevel::Debug,
        },
        TestCase {
            _name: "Case-insensitive info",
            input: "\"INFO\"",
            expected: LogLevel::Info,
        },
        TestCase {
            _name: "Case-insensitive warn",
            input: "\"WARNING\"",
            expected: LogLevel::Warn,
        },
        TestCase {
            _name: "Case-insensitive error",
            input: "\"ERROR\"",
            expected: LogLevel::Error,
        },
        TestCase {
            _name: "unknown",
            input: "\"unknown\"",
            expected: LogLevel::Info,
        },
    ];

    let mut variants = LogLevel::variants().into_iter().collect::<HashSet<_>>();

    for test_case in test_cases {
        variants.remove(&test_case.expected);
        let deserialized = serde_json::from_str::<LogLevel>(test_case.input).unwrap();
        assert_eq!(deserialized, test_case.expected);
    }

    assert!(variants.is_empty(), "variants: {variants:?}");
}

#[test]
fn deserialize_non_string_falls_back_to_default() {
    // non-string JSON types should hit the Err branch and return the default (Info)
    let cases = vec![
        serde_json::json!(42),
        serde_json::json!(true),
        serde_json::json!(null),
        serde_json::json!([1, 2]),
        serde_json::json!({"key": "val"}),
    ];
    for input in &cases {
        let result: LogLevel = serde_json::from_value(input.clone()).unwrap();
        assert_eq!(result, LogLevel::Info, "non-string input {input} should fall back to Info");
    }
}

// ========================= serialize =========================== //

#[test]
fn serialize_log_level() {
    let cases = [
        (LogLevel::Trace, "\"trace\""),
        (LogLevel::Debug, "\"debug\""),
        (LogLevel::Info, "\"info\""),
        (LogLevel::Warn, "\"warn\""),
        (LogLevel::Error, "\"error\""),
    ];
    for (level, expected) in cases {
        let serialized = serde_json::to_string(&level).unwrap();
        assert_eq!(serialized, expected, "LogLevel::{:?}", level);
    }
}

#[test]
fn serialize_deserialize_roundtrip() {
    for level in LogLevel::variants() {
        let json = serde_json::to_string(&level).unwrap();
        let roundtripped: LogLevel = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtripped, level, "roundtrip failed for {:?}", level);
    }
}

// ========================= display ============================= //

#[test]
fn test_log_level_display() {
    assert_eq!(format!("{}", LogLevel::Trace), "trace");
    assert_eq!(format!("{}", LogLevel::Debug), "debug");
    assert_eq!(format!("{}", LogLevel::Info), "info");
    assert_eq!(format!("{}", LogLevel::Warn), "warn");
    assert_eq!(format!("{}", LogLevel::Error), "error");
}

// ========================= ordering ============================ //

#[test]
fn test_log_level_ordering() {
    assert!(LogLevel::Trace < LogLevel::Debug);
    assert!(LogLevel::Debug < LogLevel::Info);
    assert!(LogLevel::Info < LogLevel::Warn);
    assert!(LogLevel::Warn < LogLevel::Error);

    // sorting should produce Trace..Error order
    let mut levels = vec![
        LogLevel::Error,
        LogLevel::Trace,
        LogLevel::Warn,
        LogLevel::Info,
        LogLevel::Debug,
    ];
    levels.sort();
    assert_eq!(
        levels,
        vec![
            LogLevel::Trace,
            LogLevel::Debug,
            LogLevel::Info,
            LogLevel::Warn,
            LogLevel::Error,
        ]
    );
}

// ========================= defaults ============================ //

#[test]
fn test_log_level_default() {
    assert_eq!(LogLevel::default(), LogLevel::Info);
}

#[test]
fn test_log_options_default() {
    let options = Options::default();
    assert!(options.stdout);
    assert_eq!(options.log_level, LogLevel::Info);
    assert_eq!(options.log_dir, std::path::PathBuf::from("/var/log/miru"));
}

// ========================= variants ============================ //

#[test]
fn test_log_level_variants() {
    let variants = LogLevel::variants();
    assert_eq!(variants.len(), 5);
    assert_eq!(variants[0], LogLevel::Trace);
    assert_eq!(variants[1], LogLevel::Debug);
    assert_eq!(variants[2], LogLevel::Info);
    assert_eq!(variants[3], LogLevel::Warn);
    assert_eq!(variants[4], LogLevel::Error);
}

// ========================= init ================================ //

#[test]
fn test_init_stdout() {
    let tmp = std::env::temp_dir().join("miru_test_logs_stdout");
    let _ = std::fs::create_dir_all(&tmp);
    let options = Options {
        stdout: true,
        log_level: LogLevel::Debug,
        log_dir: tmp.clone(),
    };
    let guard = logs::init(options);
    assert!(guard.is_ok(), "init with stdout=true should succeed");
    drop(guard);
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_init_file_only() {
    let tmp = std::env::temp_dir().join("miru_test_logs_file");
    let _ = std::fs::create_dir_all(&tmp);
    let options = Options {
        stdout: false,
        log_level: LogLevel::Warn,
        log_dir: tmp.clone(),
    };
    let guard = logs::init(options);
    assert!(guard.is_ok(), "init with stdout=false should succeed");
    drop(guard);
    let _ = std::fs::remove_dir_all(&tmp);
}
