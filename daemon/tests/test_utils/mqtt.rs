// internal crates
use miru_daemon::mqtt::options::{Credentials, Options};

// external crates
use secrecy::SecretString;

/// Options with placeholder credentials for tests that never reach a broker.
pub fn options() -> Options {
    Options::new(Credentials {
        username: "test-user".to_string(),
        password: SecretString::from("test-password"),
    })
}
