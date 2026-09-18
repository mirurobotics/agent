// internal crates
use miru_agent::windows::errors::ScmErr;

#[test]
fn not_launched_by_scm_message_points_at_console_flag() {
    let err = ScmErr::NotLaunchedByScm {
        trace: miru_agent::trace!(),
    };
    let message = err.to_string();
    assert!(message.contains("--console"), "message: {message}");
    assert!(
        message.contains("Service Control Manager"),
        "message: {message}"
    );
}
