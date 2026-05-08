// internal crates
use miru_agent::errors::Trace;
use miru_agent::privilege;
use miru_agent::privilege::{PrivilegeErr, UserInfo};

fn dummy_trace() -> Box<Trace> {
    Box::new(Trace {
        file: file!(),
        line: line!(),
        backtrace: std::backtrace::Backtrace::disabled(),
    })
}

#[test]
fn target_user_and_target_group_are_miru() {
    assert_eq!(privilege::TARGET_USER, "miru");
    assert_eq!(privilege::TARGET_GROUP, "miru");
}

#[cfg(target_os = "linux")]
#[test]
fn lookup_user_returns_root_for_root() {
    let info = privilege::lookup_user("root").expect("root should always be present");
    assert_eq!(info.uid, 0);
    assert_eq!(info.gid, 0);
    assert_eq!(info.name, "root");
}

#[test]
fn lookup_user_returns_user_not_found_for_nonexistent() {
    let err = privilege::lookup_user("nonexistent_user_xyz_123_miru_test")
        .expect_err("a clearly bogus user must not resolve");
    match err {
        PrivilegeErr::UserNotFound { name, .. } => {
            assert_eq!(name, "nonexistent_user_xyz_123_miru_test");
        }
        other => panic!("expected UserNotFound, got {other:?}"),
    }
}

#[cfg(target_os = "linux")]
#[test]
fn ensure_dropped_or_already_unprivileged_when_running_as_self_is_ok() {
    // The CI runner / dev machine is not root and is not the `miru` user,
    // so the entry point must reject the run with WrongUser. We exercise
    // only the wrong-user branch here; the root and already-miru branches
    // are covered by the smoke-test steps in the plan.
    let err = privilege::ensure_dropped_or_already_unprivileged()
        .expect_err("non-root, non-miru user must be rejected");
    match err {
        PrivilegeErr::WrongUser {
            expected,
            actual_uid,
            argv0,
            ..
        } => {
            assert_eq!(expected, privilege::TARGET_USER);
            // SAFETY: getuid is always safe.
            let real_uid = unsafe { libc::getuid() };
            assert_eq!(actual_uid, real_uid);
            assert!(!argv0.is_empty());
        }
        // `UserNotFound` is also acceptable on a Linux host that lacks a
        // `miru` passwd entry (e.g. a vanilla CI runner). Both branches
        // demonstrate the entry point refuses to silently proceed.
        PrivilegeErr::UserNotFound { name, .. } => {
            assert_eq!(name, privilege::TARGET_USER);
        }
        other => panic!("expected WrongUser or UserNotFound, got {other:?}"),
    }
}

#[cfg(target_os = "linux")]
#[test]
fn lookup_user_returns_user_not_found_when_name_contains_null_byte() {
    // Embedded NULs make CString::new fail; the helper should map that to
    // UserNotFound rather than panic, since the underlying intent is "no such
    // user in /etc/passwd".
    let err = privilege::lookup_user("contains\0null").expect_err("NUL bytes must not pass");
    match err {
        PrivilegeErr::UserNotFound { name, .. } => {
            assert_eq!(name, "contains\0null");
        }
        other => panic!("expected UserNotFound, got {other:?}"),
    }
}

#[test]
fn user_info_struct_round_trips_fields() {
    let info = UserInfo {
        uid: 1234,
        gid: 5678,
        name: "miru".to_string(),
    };
    assert_eq!(info.uid, 1234);
    assert_eq!(info.gid, 5678);
    assert_eq!(info.name, "miru");
    // Confirms Clone + PartialEq are wired up so callers can use them.
    assert_eq!(info.clone(), info);
}

#[test]
fn privilege_err_display_messages_are_human_readable() {
    let user_not_found = PrivilegeErr::UserNotFound {
        name: "miru".to_string(),
        trace: dummy_trace(),
    };
    let s = format!("{user_not_found}");
    assert!(s.contains("miru"));
    assert!(s.contains("not found"));

    let wrong_user = PrivilegeErr::WrongUser {
        expected: "miru".to_string(),
        actual_uid: 1000,
        argv0: "/usr/sbin/miru-agent".to_string(),
        trace: dummy_trace(),
    };
    let s = format!("{wrong_user}");
    assert!(s.contains("miru"));
    assert!(s.contains("1000"));
    assert!(s.contains("/usr/sbin/miru-agent"));
    assert!(s.contains("sudo"));

    let syscall = PrivilegeErr::Syscall {
        call: "setuid",
        errno: 1,
        trace: dummy_trace(),
    };
    let s = format!("{syscall}");
    assert!(s.contains("setuid"));
    assert!(s.contains("errno=1"));

    let post_drop = PrivilegeErr::PostDropMismatch {
        expected_uid: 100,
        expected_gid: 100,
        actual_uid: 0,
        actual_gid: 0,
        trace: dummy_trace(),
    };
    let s = format!("{post_drop}");
    assert!(s.contains("expected uid=100"));
    assert!(s.contains("got uid=0"));
}
