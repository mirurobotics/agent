// Shared boilerplate for status enums.
//
// `impl_status_enum!` is the single source of truth for a status enum's
// variant↔wire mapping. From one mapping table it generates a custom
// `Deserialize` (unknown→default with a log), `variants()`, `as_str()`, the
// agent-server and/or backend `From` conversions, and inline serde
// forward-compat tests. Optional clauses (`display`, `case_insensitive`,
// `on_non_string`, `aliases`) default to off so existing call sites are
// unchanged.
//
// The macro has four public forms, distinguished by the conversions a call site
// needs:
//   - local        : neither `agent_type` nor `backend_type` (e.g. `LogLevel`)
//   - agent         : `agent_type` only (e.g. `DeviceStatus`)
//   - backend+agent : `agent_type` + `backend_type` (e.g. the `Dpl*` enums)
//   - backend-only  : `backend_type` only (e.g. `DeletePolicy`)
//
// See the "Enum conventions" section of `AGENTS.md` for the canonical decision
// tree describing which facility to reach for.
macro_rules! impl_status_enum {
    // ───────────────────────────── public: backend + agent ──────────────────
    (
        enum $name:ident,
        default: $default:ident,
        label: $label:expr,
        log: $log:ident,
        $(display: $d:tt,)?
        $(case_insensitive: $ci:tt,)?
        $(on_non_string: $ons:tt,)?
        $(aliases: [ $($alias:literal => $avariant:ident),* $(,)? ],)?
        agent_type: $at:ty,
        backend_type: $bt:ty,
        unknown_backend: $ub:path,
        mappings: [
            $( $variant:ident => $wire:literal => $av:expr => $bv:path ),+ $(,)?
        ]
    ) => {
        impl_status_enum!(@base_impls enum $name, $(display: $d,)? wires [ $($variant => $wire),+ ]);
        impl_status_enum!(@deser enum $name, default $default, label $label, log $log,
            $(case_insensitive: $ci,)? aliases [ $($($alias => $avariant),*)? ],
            wires [ $($variant => $wire),+ ] $(, on_non_string: $ons)?);
        impl_status_enum!(@tests enum $name, default $default,
            $(display: $d,)? $(case_insensitive: $ci,)? $(on_non_string: $ons,)?
            aliases [ $($($alias => $avariant),*)? ], wires [ $($variant => $wire),+ ]);
        impl_status_enum!(@agent enum $name, agent_type: $at, [ $($variant => $av),+ ]);
        impl_status_enum!(@backend enum $name, default $default, label $label, log $log,
            backend_type: $bt, unknown_backend: $ub, [ $($variant => $bv),+ ]);
    };

    // ───────────────────────────── public: agent only ───────────────────────
    (
        enum $name:ident,
        default: $default:ident,
        label: $label:expr,
        log: $log:ident,
        $(display: $d:tt,)?
        $(case_insensitive: $ci:tt,)?
        $(on_non_string: $ons:tt,)?
        $(aliases: [ $($alias:literal => $avariant:ident),* $(,)? ],)?
        agent_type: $at:ty,
        mappings: [
            $( $variant:ident => $wire:literal => $av:expr ),+ $(,)?
        ]
    ) => {
        impl_status_enum!(@base_impls enum $name, $(display: $d,)? wires [ $($variant => $wire),+ ]);
        impl_status_enum!(@deser enum $name, default $default, label $label, log $log,
            $(case_insensitive: $ci,)? aliases [ $($($alias => $avariant),*)? ],
            wires [ $($variant => $wire),+ ] $(, on_non_string: $ons)?);
        impl_status_enum!(@tests enum $name, default $default,
            $(display: $d,)? $(case_insensitive: $ci,)? $(on_non_string: $ons,)?
            aliases [ $($($alias => $avariant),*)? ], wires [ $($variant => $wire),+ ]);
        impl_status_enum!(@agent enum $name, agent_type: $at, [ $($variant => $av),+ ]);
    };

    // ───────────────────────────── public: backend only ─────────────────────
    (
        enum $name:ident,
        default: $default:ident,
        label: $label:expr,
        log: $log:ident,
        $(display: $d:tt,)?
        $(case_insensitive: $ci:tt,)?
        $(on_non_string: $ons:tt,)?
        $(aliases: [ $($alias:literal => $avariant:ident),* $(,)? ],)?
        backend_type: $bt:ty,
        unknown_backend: $ub:path,
        mappings: [
            $( $variant:ident => $wire:literal => $bv:path ),+ $(,)?
        ]
    ) => {
        impl_status_enum!(@base_impls enum $name, $(display: $d,)? wires [ $($variant => $wire),+ ]);
        impl_status_enum!(@deser enum $name, default $default, label $label, log $log,
            $(case_insensitive: $ci,)? aliases [ $($($alias => $avariant),*)? ],
            wires [ $($variant => $wire),+ ] $(, on_non_string: $ons)?);
        impl_status_enum!(@tests enum $name, default $default,
            $(display: $d,)? $(case_insensitive: $ci,)? $(on_non_string: $ons,)?
            aliases [ $($($alias => $avariant),*)? ], wires [ $($variant => $wire),+ ]);
        impl_status_enum!(@backend enum $name, default $default, label $label, log $log,
            backend_type: $bt, unknown_backend: $ub, [ $($variant => $bv),+ ]);
    };

    // ───────────────────────────── public: local ────────────────────────────
    (
        enum $name:ident,
        default: $default:ident,
        label: $label:expr,
        log: $log:ident,
        $(display: $d:tt,)?
        $(case_insensitive: $ci:tt,)?
        $(on_non_string: $ons:tt,)?
        $(aliases: [ $($alias:literal => $avariant:ident),* $(,)? ],)?
        mappings: [
            $( $variant:ident => $wire:literal ),+ $(,)?
        ]
    ) => {
        impl_status_enum!(@base_impls enum $name, $(display: $d,)? wires [ $($variant => $wire),+ ]);
        impl_status_enum!(@deser enum $name, default $default, label $label, log $log,
            $(case_insensitive: $ci,)? aliases [ $($($alias => $avariant),*)? ],
            wires [ $($variant => $wire),+ ] $(, on_non_string: $ons)?);
        impl_status_enum!(@tests enum $name, default $default,
            $(display: $d,)? $(case_insensitive: $ci,)? $(on_non_string: $ons,)?
            aliases [ $($($alias => $avariant),*)? ], wires [ $($variant => $wire),+ ]);
    };

    // ───────────────────────────── @base_impls ──────────────────────────────
    // `variants()`, `as_str()`, and (opt-in) `Display`.
    (@base_impls enum $name:ident, $(display: $d:tt,)? wires [ $($variant:ident => $wire:literal),+ $(,)? ]) => {
        impl $name {
            pub fn variants() -> Vec<$name> {
                vec![$($name::$variant),+]
            }

            pub fn as_str(&self) -> &'static str {
                match self {
                    $($name::$variant => $wire,)+
                }
            }
        }

        $( impl_status_enum!(@display $d, enum $name); )?
    };

    (@display $d:tt, enum $name:ident) => {
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };

    // ───────────────────────────── @key ─────────────────────────────────────
    // The string the deserializer matches against: lowercased when
    // case-insensitive. Case-insensitive wires/aliases must be lowercase.
    (@key $s:ident) => { $s.as_str() };
    (@key $s:ident, case_insensitive: $ci:tt) => { $s.to_lowercase().as_str() };

    // ───────────────────────────── @deser ───────────────────────────────────
    // Lenient form: a non-string logs and falls back to the default.
    (@deser enum $name:ident, default $default:ident, label $label:expr, log $log:ident,
        $(case_insensitive: $ci:tt,)?
        aliases [ $($alias:literal => $avariant:ident),* $(,)? ],
        wires [ $($variant:ident => $wire:literal),+ $(,)? ],
        on_non_string: default
    ) => {
        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let s = match <String as serde::Deserialize>::deserialize(deserializer) {
                    Ok(s) => s,
                    Err(e) => {
                        $log!(
                            "{} could not be read as a string ({:?}), defaulting to {:?}",
                            $label, e, $name::$default
                        );
                        return Ok($name::$default);
                    }
                };
                let default = $name::$default;
                match impl_status_enum!(@key s $(, case_insensitive: $ci)?) {
                    $( $wire => Ok($name::$variant), )+
                    $( $alias => Ok($name::$avariant), )*
                    other => {
                        $log!("{} '{}' is not valid, defaulting to {:?}", $label, other, default);
                        Ok(default)
                    }
                }
            }
        }
    };

    // Strict form (default): a non-string propagates the deserialize error.
    (@deser enum $name:ident, default $default:ident, label $label:expr, log $log:ident,
        $(case_insensitive: $ci:tt,)?
        aliases [ $($alias:literal => $avariant:ident),* $(,)? ],
        wires [ $($variant:ident => $wire:literal),+ $(,)? ]
    ) => {
        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let s = <String as serde::Deserialize>::deserialize(deserializer)?;
                let default = $name::$default;
                match impl_status_enum!(@key s $(, case_insensitive: $ci)?) {
                    $( $wire => Ok($name::$variant), )+
                    $( $alias => Ok($name::$avariant), )*
                    other => {
                        $log!("{} '{}' is not valid, defaulting to {:?}", $label, other, default);
                        Ok(default)
                    }
                }
            }
        }
    };

    // ───────────────────────────── @agent ───────────────────────────────────
    (@agent enum $name:ident, agent_type: $at:ty, [ $($variant:ident => $av:expr),+ $(,)? ]) => {
        impl From<&$name> for $at {
            fn from(status: &$name) -> Self {
                match status {
                    $($name::$variant => $av,)+
                }
            }
        }
    };

    // ───────────────────────────── @backend ─────────────────────────────────
    (@backend enum $name:ident, default $default:ident, label $label:expr, log $log:ident,
        backend_type: $bt:ty, unknown_backend: $ub:path, [ $($variant:ident => $bv:path),+ $(,)? ]
    ) => {
        impl From<&$name> for $bt {
            fn from(status: &$name) -> Self {
                match status {
                    $($name::$variant => $bv,)+
                }
            }
        }

        impl From<&$bt> for $name {
            fn from(status: &$bt) -> $name {
                match status {
                    $( $bv => $name::$variant, )+
                    other => {
                        let default = $name::$default;
                        $log!(
                            "{} backend value {:?} is not recognized, defaulting to {:?}",
                            $label, other, default
                        );
                        default
                    }
                }
            }
        }

        #[cfg(test)]
        paste::paste! {
            mod [<$name:snake _backend_tests>] {
                use super::*;

                #[test]
                fn unknown_backend_maps_to_default() {
                    let d: $name = (&$ub).into();
                    assert_eq!(d, $name::$default);
                }

                #[test]
                fn known_backend_values_map_exactly() {
                    $(
                        let d: $name = (&$bv).into();
                        assert_eq!(d, $name::$variant);
                    )+
                }
            }
        }
    };

    // ───────────────────────────── @tests ───────────────────────────────────
    // Inline serde forward-compat tests generated from the mapping table.
    (@tests enum $name:ident, default $default:ident,
        $(display: $d:tt,)? $(case_insensitive: $ci:tt,)? $(on_non_string: $ons:tt,)?
        aliases [ $($alias:literal => $avariant:ident),* $(,)? ],
        wires [ $($variant:ident => $wire:literal),+ $(,)? ]
    ) => {
        #[cfg(test)]
        paste::paste! {
            mod [<$name:snake _serde_tests>] {
                use super::*;
                use std::collections::HashSet;

                #[test]
                fn serialize_roundtrips_every_variant() {
                    $(
                        let v = $name::$variant;
                        let s = serde_json::to_string(&v).unwrap();
                        assert_eq!(s, concat!("\"", $wire, "\""));
                        let back: $name = serde_json::from_str(&s).unwrap();
                        assert_eq!(back, v);
                    )+
                }

                #[test]
                fn as_str_matches_serde() {
                    $( assert_eq!($name::$variant.as_str(), $wire); )+
                }

                #[test]
                fn unknown_wire_deserializes_to_default() {
                    let d: $name =
                        serde_json::from_str("\"__impl_status_enum_unknown_sentinel__\"").unwrap();
                    assert_eq!(d, $name::$default);
                }

                #[test]
                fn exhaustiveness() {
                    let mut set: HashSet<$name> = $name::variants().into_iter().collect();
                    $( set.remove(&$name::$variant); )+
                    assert!(set.is_empty(), "untested variants: {set:?}");
                }

                #[test]
                fn aliases_deserialize_to_variant() {
                    $(
                        let d: $name =
                            serde_json::from_str(concat!("\"", $alias, "\"")).unwrap();
                        assert_eq!(d, $name::$avariant);
                    )*
                }

                impl_status_enum!(@nonstring_test enum $name, default $default $(, on_non_string: $ons)?);
                impl_status_enum!(@case_test enum $name, default $default,
                    wires [ $($variant => $wire),+ ] $(, case_insensitive: $ci)?);
                $( impl_status_enum!(@display_test $d, enum $name); )?
            }
        }
    };

    // Non-string handling: lenient (falls back) vs strict (rejects, the default).
    // Both deserializer instantiations (`from_value` over `serde_json::Value` and
    // `from_str` over the streaming reader) are exercised so the generic
    // `Deserialize` error path is covered for each monomorphization.
    (@nonstring_test enum $name:ident, default $default:ident, on_non_string: default) => {
        #[test]
        fn non_string_falls_back_to_default() {
            let cases = [
                serde_json::json!(42),
                serde_json::json!(true),
                serde_json::json!(null),
                serde_json::json!([1, 2]),
                serde_json::json!({"k": "v"}),
            ];
            for input in cases {
                let d: $name = serde_json::from_value(input.clone()).unwrap();
                assert_eq!(d, $name::$default, "non-string {input} should default");
            }
            // Exercise the streaming-reader instantiation's error arm. The reader
            // may report trailing characters after the lenient fallback, so the
            // overall result is ignored — only the error arm needs to execute.
            for input in ["42", "null", "true", "not-json"] {
                let _ = serde_json::from_str::<$name>(input);
            }
        }
    };
    (@nonstring_test enum $name:ident, default $default:ident) => {
        #[test]
        fn rejects_non_string() {
            for input in ["not-json", "42", "null", "[]"] {
                assert!(
                    serde_json::from_str::<$name>(input).is_err(),
                    "should reject: {input}"
                );
            }
            let values = [
                serde_json::json!(42),
                serde_json::json!(true),
                serde_json::json!(null),
                serde_json::json!([1, 2]),
                serde_json::json!({"k": "v"}),
            ];
            for input in values {
                assert!(
                    serde_json::from_value::<$name>(input.clone()).is_err(),
                    "should reject: {input}"
                );
            }
        }
    };

    // Case handling: insensitive accepts uppercase; sensitive rejects it (→ default).
    (@case_test enum $name:ident, default $default:ident,
        wires [ $($variant:ident => $wire:literal),+ $(,)? ], case_insensitive: $ci:tt) => {
        #[test]
        fn case_insensitive_deserialize() {
            $(
                let json = format!("\"{}\"", $wire.to_uppercase());
                let d: $name = serde_json::from_str(&json).unwrap();
                assert_eq!(d, $name::$variant);
            )+
        }
    };
    (@case_test enum $name:ident, default $default:ident,
        wires [ $($variant:ident => $wire:literal),+ $(,)? ]) => {
        #[test]
        fn case_sensitive_rejects_uppercase() {
            $(
                let json = format!("\"{}\"", $wire.to_uppercase());
                let d: $name = serde_json::from_str(&json).unwrap();
                assert_eq!(d, $name::$default);
            )+
        }
    };

    (@display_test $d:tt, enum $name:ident) => {
        #[test]
        fn display_matches_as_str() {
            for v in $name::variants() {
                assert_eq!(v.to_string(), v.as_str());
            }
        }
    };
}

// Re-export the macro so model and logs modules can
// `use crate::models::status::impl_status_enum;`.
pub(crate) use impl_status_enum;
