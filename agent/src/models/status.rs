// Shared boilerplate for status enums. The public macro has:
// - a base form for agent-facing enums
// - an extended form that also generates backend conversions
macro_rules! impl_status_enum {
    (
        enum $name:ident,
        default: $default:ident,
        label: $label:expr,
        log: $log_macro:ident,
        agent_type: $agent_type:ty,
        mappings: [
            $(
                $variant:ident => $wire:literal => $agent_value:expr
            ),+ $(,)?
        ]
    ) => {
        impl_status_enum!(
            // Route the simple form through the shared implementation arm.
            @base
            enum $name,
            default: $default,
            label: $label,
            log: $log_macro,
            agent_type: $agent_type,
            mappings: [
                $(
                    $variant => $wire => $agent_value
                ),+
            ]
        );
    };
    (
        enum $name:ident,
        default: $default:ident,
        label: $label:expr,
        log: $log_macro:ident,
        agent_type: $agent_type:ty,
        backend_type: $backend_type:ty,
        mappings: [
            $(
                $variant:ident => $wire:literal =>
                    $agent_value:expr =>
                    $backend_value:path
            ),+ $(,)?
        ]
    ) => {
        impl_status_enum!(
            // Reuse the shared enum/string/agent conversion implementation first.
            @base
            enum $name,
            default: $default,
            label: $label,
            log: $log_macro,
            agent_type: $agent_type,
            mappings: [
                $(
                    $variant => $wire => $agent_value
                ),+
            ]
        );

        impl From<&$name> for $backend_type {
            fn from(status: &$name) -> Self {
                match status {
                    $(
                        $name::$variant => $backend_value,
                    )+
                }
            }
        }

        impl From<&$backend_type> for $name {
            fn from(status: &$backend_type) -> $name {
                match status {
                    $(
                        $backend_value => $name::$variant,
                    )+
                }
            }
        }
    };
    (
        // Internal arm that generates the behavior shared by both public forms.
        @base
        enum $name:ident,
        default: $default:ident,
        label: $label:expr,
        log: $log_macro:ident,
        agent_type: $agent_type:ty,
        mappings: [
            $(
                $variant:ident => $wire:literal => $agent_value:expr
            ),+ $(,)?
        ]
    ) => {
        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let s = <String as serde::Deserialize>::deserialize(deserializer)?;
                let default = $name::$default;
                match s.as_str() {
                    $(
                        $wire => Ok($name::$variant),
                    )+
                    status => {
                        $log_macro!(
                            "{} '{}' is not valid, defaulting to {:?}",
                            $label, status, default
                        );
                        Ok(default)
                    }
                }
            }
        }

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

        impl From<&$name> for $agent_type {
            fn from(status: &$name) -> Self {
                match status {
                    $(
                        $name::$variant => $agent_value,
                    )+
                }
            }
        }
    };
}

// Re-export the macro so model modules can `use crate::models::status::impl_status_enum;`.
pub(crate) use impl_status_enum;
