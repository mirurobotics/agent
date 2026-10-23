// internal crates
use backend_api::models as backend_client;
use device_api::models as agent_server;
use miru_agent::models::Os;

// harness
use crate::models::harnesses::{status_enum_tests, StatusCase, StatusEnumFixture};

impl StatusEnumFixture for Os {
    type Backend = backend_client::Os;
    type Agent = agent_server::Os;

    fn cases() -> Vec<StatusCase<Self>> {
        vec![
            StatusCase {
                variant: Os::Linux,
                wire: "linux",
                backend: backend_client::Os::OS_LINUX,
                agent: agent_server::Os::OS_LINUX,
            },
            StatusCase {
                variant: Os::Windows,
                wire: "windows",
                backend: backend_client::Os::OS_WINDOWS,
                agent: agent_server::Os::OS_WINDOWS,
            },
        ]
    }
}

status_enum_tests!(Os, backend, agent);
