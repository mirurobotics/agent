use serde::de::DeserializeOwned;
use serde::Serialize;
use std::collections::HashSet;
use std::fmt::Debug;
use std::hash::Hash;

// ─── field descriptors ───────────────────────────────────────────────────────

/// A JSON field that MUST be present for deserialization to succeed.
pub struct RequiredField {
    /// The JSON key (may differ from the Rust field name, e.g. `device_id` vs `id`).
    pub key: &'static str,
    /// A representative test value.
    pub value: serde_json::Value,
}

/// A JSON field that CAN be absent — the model's custom `Deserialize` impl will apply a
/// default.
pub struct OptionalField {
    /// The JSON key.
    pub key: &'static str,
    /// A representative non-default value used when building a "full" test payload.
    pub value: serde_json::Value,
    /// The expected serialized form of the default that gets applied when this
    /// field is absent from input (e.g. `json!("1970-01-01T00:00:00Z")` for an
    /// epoch-defaulting `DateTime`).
    pub default_value: serde_json::Value,
}

// ─── trait ────────────────────────────────────────────────────────────────────

/// Implement for each model to get a standard serde test suite via `serde_tests!`.
///
/// Models declare their fields as **required** or **optional** and the harness
/// builds all test instances and validation logic from those declarations.
pub trait ModelFixture: Serialize + DeserializeOwned + PartialEq + Debug {
    /// JSON fields that MUST be present for deserialization to succeed.
    fn required_fields() -> Vec<RequiredField>;

    /// JSON fields that CAN be absent — defaults will be applied on
    /// deserialization. Return empty if every field is required.
    fn optional_fields() -> Vec<OptionalField> {
        vec![]
    }
}

// ─── helpers ─────────────────────────────────────────────────────────────────

/// Build a JSON object containing all required + optional field values.
fn build_full_json<T: ModelFixture>() -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for f in T::required_fields() {
        map.insert(f.key.to_string(), f.value);
    }
    for f in T::optional_fields() {
        map.insert(f.key.to_string(), f.value);
    }
    serde_json::Value::Object(map)
}

/// Build a JSON object containing only the required field values.
fn build_required_json<T: ModelFixture>() -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for f in T::required_fields() {
        map.insert(f.key.to_string(), f.value);
    }
    serde_json::Value::Object(map)
}

// ─── generic assertion helpers ───────────────────────────────────────────────

/// Deserialize all-fields JSON to `a`, serialize `a`, deserialize to `b`,
/// assert `a == b`.
pub fn assert_roundtrip<T: ModelFixture>() {
    let all_json = build_full_json::<T>();
    let a: T = serde_json::from_value(all_json).expect("deserialize all-fields JSON failed");
    let serialized = serde_json::to_string(&a).expect("serialize failed");
    let b: T = serde_json::from_str(&serialized).expect("roundtrip deserialize failed");
    assert_eq!(a, b, "roundtrip mismatch");
}

/// Non-object inputs must be rejected.
pub fn assert_rejects_invalid_json<T: ModelFixture>() {
    for input in ["not-json", "42", "null", "\"a string\"", "[]"] {
        assert!(
            serde_json::from_str::<T>(input).is_err(),
            "should reject: {input}"
        );
    }
}

/// An empty JSON object must be rejected (assumes at least one required field).
pub fn assert_rejects_empty_object<T: ModelFixture>() {
    let empty = serde_json::json!({});
    assert!(
        serde_json::from_value::<T>(empty).is_err(),
        "should reject empty object"
    );
}

/// For each required field, build a payload with *all other* fields present but
/// that one omitted, and assert deserialization fails.
pub fn assert_rejects_missing_required<T: ModelFixture>() {
    let required = T::required_fields();
    let optional = T::optional_fields();

    for i in 0..required.len() {
        let mut map = serde_json::Map::new();
        for (j, f) in required.iter().enumerate() {
            if i != j {
                map.insert(f.key.to_string(), f.value.clone());
            }
        }
        for f in &optional {
            map.insert(f.key.to_string(), f.value.clone());
        }
        let json = serde_json::Value::Object(map);
        assert!(
            serde_json::from_value::<T>(json).is_err(),
            "should reject payload missing required field: {}",
            required[i].key
        );
    }
}

/// Deserialize required-only JSON, re-serialize, then verify:
/// - each required field's serialized value matches its declared `value`
/// - each optional field's serialized value matches its `default_value`
pub fn assert_minimal_json_defaults<T: ModelFixture>() {
    let required_json = build_required_json::<T>();
    let instance: T =
        serde_json::from_value(required_json).expect("required-only JSON should deserialize");

    let reserialized =
        serde_json::to_value(&instance).expect("re-serialize of minimal instance failed");
    let obj = reserialized
        .as_object()
        .expect("minimal instance should serialize to a JSON object");

    for req in T::required_fields() {
        let actual = obj.get(req.key).unwrap_or_else(|| {
            panic!(
                "required field '{}' missing from serialized output",
                req.key
            )
        });
        assert_eq!(
            *actual, req.value,
            "required field '{}' value mismatch after minimal deserialize",
            req.key
        );
    }

    for opt in T::optional_fields() {
        let actual = obj.get(opt.key).unwrap_or_else(|| {
            panic!(
                "optional field '{}' missing from serialized output",
                opt.key
            )
        });
        assert_eq!(
            *actual, opt.default_value,
            "optional field '{}' should have its default value when absent from input",
            opt.key
        );
    }
}

// ─── macro that generates the standard test suite ────────────────────────────

/// Generates the standard serde tests for a model that implements `ModelFixture`.
///
/// Usage:
/// ```ignore
/// serde_tests!(ConfigInstance);
/// ```
macro_rules! serde_tests {
    ($type:ty) => {
        mod harness {
            use super::*;
            use crate::models::harnesses::*;

            #[test]
            fn roundtrip() {
                assert_roundtrip::<$type>();
            }

            #[test]
            fn rejects_invalid_json() {
                assert_rejects_invalid_json::<$type>();
            }

            #[test]
            fn rejects_empty_object() {
                assert_rejects_empty_object::<$type>();
            }

            #[test]
            fn rejects_missing_required_fields() {
                assert_rejects_missing_required::<$type>();
            }

            #[test]
            fn minimal_json_applies_defaults() {
                assert_minimal_json_defaults::<$type>();
            }
        }
    };
}

pub(crate) use serde_tests;

// ─── status enum harness ─────────────────────────────────────────────────────

/// A single serde test case for a status enum variant.
pub struct StatusCase<S> {
    /// The JSON input string (e.g. `"\"staged\""`).
    pub input: &'static str,
    /// The expected deserialized variant.
    pub expected: S,
    /// Whether this case represents a valid (known) variant. When `true`,
    /// re-serialization is checked against `input`.
    pub valid: bool,
}

/// Implement for each status enum to get a standard serde test suite via
/// `status_serde_tests!`.
pub trait StatusFixture:
    Serialize + DeserializeOwned + PartialEq + Eq + Hash + Debug + Default
{
    /// All known variants of the enum.
    fn variants() -> Vec<Self>;
    /// Test cases covering every variant plus at least one unknown input.
    fn cases() -> Vec<StatusCase<Self>>;
}

// ─── status assertion helpers ────────────────────────────────────────────────

/// For each case in `StatusFixture::cases()`:
///   - deserialize `input` and assert it matches `expected`
///   - if `valid`, re-serialize and assert it matches `input`
///
/// After all cases, verify that every variant was covered.
pub fn assert_status_serde<S: StatusFixture>() {
    let mut uncovered = S::variants().into_iter().collect::<HashSet<_>>();

    for case in S::cases() {
        uncovered.remove(&case.expected);
        let deserialized: S = serde_json::from_str(case.input)
            .unwrap_or_else(|e| panic!("failed to deserialize {}: {e}", case.input));
        assert_eq!(deserialized, case.expected, "input: {}", case.input);
        if case.valid {
            let serialized = serde_json::to_string(&case.expected).unwrap();
            assert_eq!(
                serialized, case.input,
                "roundtrip mismatch for {:?}",
                case.expected
            );
        }
    }

    assert!(uncovered.is_empty(), "untested variants: {uncovered:?}");
}

/// Verify that an unrecognised string deserializes to `S::default()`.
pub fn assert_status_unknown_defaults<S: StatusFixture>() {
    let deserialized: S =
        serde_json::from_str("\"unknown\"").expect("deserializing \"unknown\" should not fail");
    assert_eq!(
        deserialized,
        S::default(),
        "unknown input should default to {:?}",
        S::default()
    );
}

pub fn assert_status_rejects_invalid_string<S: StatusFixture>() {
    for input in ["not-json", "42", "null", "[]"] {
        assert!(
            serde_json::from_str::<S>(input).is_err(),
            "should reject: {input}"
        );
    }
}

// ─── macro that generates the standard status serde test suite ───────────────

/// Generates the standard serde tests for a status enum that implements
/// `StatusFixture`.
///
/// Usage:
/// ```ignore
/// status_serde_tests!(DeploymentTargetStatus);
/// ```
macro_rules! status_serde_tests {
    ($type:ty) => {
        mod harness {
            use super::*;
            use crate::models::harnesses::*;

            #[test]
            fn serde_roundtrip_all_variants() {
                assert_status_serde::<$type>();
            }

            #[test]
            fn unknown_falls_back_to_default() {
                assert_status_unknown_defaults::<$type>();
            }

            #[test]
            fn rejects_invalid_string() {
                assert_status_rejects_invalid_string::<$type>();
            }
        }
    };
}

pub(crate) use status_serde_tests;
