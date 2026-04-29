// standard crates
use std::collections::HashSet;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

// internal crates
use miru_agent::errors::{Code, Error, HTTPCode};
use miru_agent::filesys::{Dir, PathExt};
use miru_agent::logs::{self, LogLevel, LogsErr, Options};

// external crates
use serial_test::serial;
use tracing_subscriber::{fmt, prelude::*, registry::Registry, reload, EnvFilter};

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
        assert_eq!(
            result,
            LogLevel::Info,
            "non-string input {input} should fall back to Info"
        );
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

// Note: most layer-construction coverage lives here via `logs::build_layers`,
// which returns the composite layer plus worker guard, reload handle, and
// env-filter-locked flag without ever calling `set_global_default`. Only tests
// that actually need `set_global_default` to fire (or that need a non-`RUST_LOG=off`
// environment, which the shared integration binary inherits from
// `scripts/test.sh`) live in dedicated integration-test binaries under
// `agent/tests/logs_init_*.rs`.

// ========================= reload =============================== //

#[derive(Clone, Default)]
struct CapturingWriter(Arc<Mutex<Vec<u8>>>);

impl Write for CapturingWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> fmt::MakeWriter<'a> for CapturingWriter {
    type Writer = CapturingWriter;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[test]
fn test_reload_level_changes_filter() {
    // Build a fresh subscriber with a captured-buffer writer and install it
    // thread-locally via `set_default`. This is hermetic against any global
    // subscriber installed by other tests.
    let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let writer = CapturingWriter(buf.clone());

    let (filter_layer, handle) = reload::Layer::new(EnvFilter::new("warn"));
    let subscriber = Registry::default()
        .with(filter_layer)
        .with(fmt::layer().with_writer(writer));
    let _guard = tracing::subscriber::set_default(subscriber);

    tracing::debug!("before-reload");
    handle
        .reload(EnvFilter::new("debug"))
        .expect("reload should succeed");
    tracing::debug!("after-reload");

    let captured = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
    assert!(
        !captured.contains("before-reload"),
        "pre-reload debug event should be filtered out: {captured}"
    );
    assert!(
        captured.contains("after-reload"),
        "post-reload debug event should be emitted: {captured}"
    );
}

// `test_reload_level_no_op_when_env_filter_locked` lives in
// `agent/tests/logs_init_locked.rs` because it calls `logs::init`, which
// installs a process-global subscriber and depends on the `RUST_LOG=off`
// environment that `scripts/test.sh` exports.

// ========================= LogsErr ============================== //

#[test]
fn test_logs_err_reload_failed_display() {
    let err = LogsErr::ReloadFailed("handle dropped".to_string());
    let s = format!("{err}");
    assert!(
        s.contains("failed to reload tracing filter"),
        "Display should include the prefix, got: {s}"
    );
    assert!(
        s.contains("handle dropped"),
        "Display should include the inner message, got: {s}"
    );
}

#[test]
fn test_logs_err_uses_default_error_trait() {
    // LogsErr only implements Error to opt into the project's error machinery;
    // it relies on the trait defaults.
    let err = LogsErr::ReloadFailed("anything".to_string());
    assert_eq!(err.code().as_str(), Code::InternalServerError.as_str());
    assert_eq!(err.http_status(), HTTPCode::INTERNAL_SERVER_ERROR);
    assert!(err.params().is_none());
    assert!(!err.is_network_conn_err());
}

// ========================= build_layers ========================= //

/// RAII guard that restores `RUST_LOG` to its prior value (or unset state)
/// when dropped. Tests that mutate `RUST_LOG` must hold one of these for the
/// duration of the test and serialize via `#[serial(rust_log)]` because the
/// env var is process-wide.
struct RustLogGuard {
    prior: Option<String>,
}

impl RustLogGuard {
    fn capture() -> Self {
        Self {
            prior: std::env::var("RUST_LOG").ok(),
        }
    }
}

impl Drop for RustLogGuard {
    fn drop(&mut self) {
        // SAFETY: the surrounding test is `#[serial(rust_log)]`, so no other
        // test mutates `RUST_LOG` concurrently; the only readers are tracing's
        // env-filter constructors invoked synchronously inside the test body.
        unsafe {
            match &self.prior {
                Some(v) => std::env::set_var("RUST_LOG", v),
                None => std::env::remove_var("RUST_LOG"),
            }
        }
    }
}

async fn build_layers_tempdir(prefix: &str) -> PathBuf {
    let dir = Dir::create_temp_dir(prefix).await.unwrap();
    dir.path().clone()
}

#[tokio::test]
async fn test_build_layers_stdout_debug() {
    let log_dir = build_layers_tempdir("miru_test_build_layers_stdout").await;
    let options = Options {
        stdout: true,
        log_level: LogLevel::Debug,
        log_dir,
    };
    let (layer, _worker, handle, _locked) = logs::build_layers(options);

    // Reload handle is well-formed: swapping the inner filter should succeed.
    handle
        .reload(EnvFilter::new("warn"))
        .expect("reload on freshly-built handle should succeed");

    // Attaching the composite via `with_default` (thread-local install) must
    // not panic and must accept emission of an event at the active level.
    let subscriber = Registry::default().with(layer);
    tracing::subscriber::with_default(subscriber, || {
        tracing::warn!("hello from build_layers stdout test");
    });
}

#[tokio::test]
async fn test_build_layers_file_only_warn() {
    let log_dir = build_layers_tempdir("miru_test_build_layers_file").await;
    let options = Options {
        stdout: false,
        log_level: LogLevel::Warn,
        log_dir,
    };
    let (layer, _worker, handle, _locked) = logs::build_layers(options);

    handle
        .reload(EnvFilter::new("error"))
        .expect("reload on freshly-built handle should succeed");

    let subscriber = Registry::default().with(layer);
    tracing::subscriber::with_default(subscriber, || {
        tracing::error!("hello from build_layers file-only test");
    });
}

#[tokio::test]
#[serial(rust_log)]
async fn test_build_layers_respects_rust_log_when_set() {
    let _guard = RustLogGuard::capture();
    // SAFETY: `#[serial(rust_log)]` excludes other rust_log-touching tests;
    // the guard restores prior state on drop so we don't leak `info` to the
    // next serialized test.
    unsafe {
        std::env::set_var("RUST_LOG", "info");
    }

    let log_dir = build_layers_tempdir("miru_test_build_layers_rustlog_set").await;
    let options = Options {
        stdout: false,
        log_level: LogLevel::Debug,
        log_dir,
    };
    let (_layer, _worker, _handle, env_filter_locked) = logs::build_layers(options);
    assert!(
        env_filter_locked,
        "build_layers should report env_filter_locked=true when RUST_LOG is set"
    );
}

#[tokio::test]
#[serial(rust_log)]
async fn test_build_layers_uses_options_when_rust_log_unset() {
    let _guard = RustLogGuard::capture();
    // SAFETY: see test_build_layers_respects_rust_log_when_set.
    unsafe {
        std::env::remove_var("RUST_LOG");
    }

    let log_dir = build_layers_tempdir("miru_test_build_layers_rustlog_unset").await;
    let options = Options {
        stdout: false,
        log_level: LogLevel::Debug,
        log_dir,
    };
    let (_layer, _worker, _handle, env_filter_locked) = logs::build_layers(options);
    assert!(
        !env_filter_locked,
        "build_layers should report env_filter_locked=false when RUST_LOG is unset"
    );
}

#[tokio::test]
#[serial(rust_log)]
async fn test_build_layers_reload_handle_changes_filter() {
    // We need to exercise emission semantics through the reload handle that
    // build_layers returns. The composite contains the file fmt-layer, which
    // writes to a non-blocking worker — capturing those bytes is awkward, so
    // instead we attach the build_layers composite alongside an additional
    // CapturingWriter fmt layer at the same Registry. The capturing layer has
    // no env filter of its own; it inherits the reload-handle filter from the
    // composite via Registry composition. That gives us a single source of
    // truth for whether the reload took effect.
    let _guard = RustLogGuard::capture();
    // SAFETY: see test_build_layers_respects_rust_log_when_set.
    unsafe {
        std::env::remove_var("RUST_LOG");
    }

    let log_dir = build_layers_tempdir("miru_test_build_layers_reload").await;
    let options = Options {
        stdout: true,
        log_level: LogLevel::Warn,
        log_dir,
    };
    let (layer, _worker, handle, env_filter_locked) = logs::build_layers(options);
    assert!(
        !env_filter_locked,
        "RUST_LOG was cleared, env filter should not be locked"
    );

    let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let writer = CapturingWriter(buf.clone());
    let subscriber = Registry::default()
        .with(layer)
        .with(fmt::layer().with_writer(writer));

    tracing::subscriber::with_default(subscriber, || {
        tracing::debug!("before-reload");
        handle
            .reload(EnvFilter::new("debug"))
            .expect("reload should succeed on a non-locked filter");
        tracing::debug!("after-reload");
    });

    let captured = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
    assert!(
        !captured.contains("before-reload"),
        "pre-reload debug event should be filtered out: {captured}"
    );
    assert!(
        captured.contains("after-reload"),
        "post-reload debug event should be emitted: {captured}"
    );
}
