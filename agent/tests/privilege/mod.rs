// internal crates
use miru_agent::errors::Trace;
use miru_agent::privilege;
use miru_agent::privilege::PrivilegeErr;

fn dummy_trace() -> Box<Trace> {
    Box::new(Trace {
        file: file!(),
        line: line!(),
        backtrace: std::backtrace::Backtrace::disabled(),
    })
}

#[test]
fn run_as_rejects_non_target_user_when_not_root() {
    // The CI runner / dev machine is not root and is not the `miru` user, so
    // the entry point must reject the run with WrongUser. The root and
    // already-miru branches are covered by the smoke-test steps in the plan.
    //
    // On a hypothetical host whose euid happens to resolve to a passwd entry
    // named `miru`, this test would otherwise hit the no-op success path and
    // fall through to `expect_err`. Skip in that case rather than panic.
    let euid = nix::unistd::geteuid();
    match nix::unistd::User::from_uid(euid) {
        Ok(Some(u)) if u.name == "miru" => {
            eprintln!(
                "skipping: euid {} resolves to 'miru'; this test asserts the \
                 non-miru rejection path",
                euid.as_raw(),
            );
            return;
        }
        Ok(Some(_)) | Ok(None) => {}
        Err(e) => {
            eprintln!("skipping: User::from_uid({}) failed: {e}", euid.as_raw());
            return;
        }
    }

    let err = privilege::run_as("miru").expect_err("non-root, non-miru user must be rejected");
    match err {
        PrivilegeErr::WrongUser {
            expected,
            actual_uid,
            argv0,
            ..
        } => {
            assert_eq!(expected, "miru");
            let real_uid = nix::unistd::getuid().as_raw();
            assert_eq!(actual_uid, real_uid);
            assert!(!argv0.is_empty());
        }
        // `UserNotFound` is also acceptable on a Linux host that lacks a
        // `miru` passwd entry (e.g. a vanilla CI runner). Both branches
        // demonstrate the entry point refuses to silently proceed.
        PrivilegeErr::UserNotFound { name, .. } => {
            assert_eq!(name, "miru");
        }
        other => panic!("expected WrongUser or UserNotFound, got {other:?}"),
    }
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
        actual_gid: 1001,
        expected_uid: 999,
        expected_gid: 998,
        argv0: "/usr/sbin/miru-agent".to_string(),
        trace: dummy_trace(),
    };
    let s = format!("{wrong_user}");
    assert!(s.contains("miru"));
    assert!(s.contains("uid 1000"));
    assert!(s.contains("gid 1001"));
    assert!(s.contains("expected uid 999"));
    assert!(s.contains("gid 998"));
    assert!(s.contains("/usr/sbin/miru-agent"));
    assert!(s.contains("sudo"));

    let syscall = PrivilegeErr::Syscall {
        call: "setuid",
        errno: nix::errno::Errno::EPERM,
        trace: dummy_trace(),
    };
    let s = format!("{syscall}");
    assert!(s.contains("setuid"));
    assert!(s.contains("EPERM"));

    let post_drop = PrivilegeErr::PostDropMismatch {
        expected_uid: 100,
        expected_gid: 200,
        actual_ruid: 0,
        actual_euid: 1,
        actual_suid: 2,
        actual_rgid: 3,
        actual_egid: 4,
        actual_sgid: 5,
        trace: dummy_trace(),
    };
    let s = format!("{post_drop}");
    assert!(s.contains("expected uid=100"));
    assert!(s.contains("gid=200"));
    assert!(s.contains("ruid=0"));
    assert!(s.contains("euid=1"));
    assert!(s.contains("suid=2"));
    assert!(s.contains("rgid=3"));
    assert!(s.contains("egid=4"));
    assert!(s.contains("sgid=5"));

    let priv_supp = PrivilegeErr::PrivilegedSupplementaryGroup {
        gid: 0,
        trace: dummy_trace(),
    };
    let s = format!("{priv_supp}");
    assert!(s.contains("privileged gid 0"));
    assert!(s.contains("supplementary"));
}

#[test]
fn run_as_is_noop_when_already_target_user() {
    // Resolve the current effective user from the passwd database. If the
    // runner has no passwd entry for its euid (rare — minimal containers,
    // chroots) we skip rather than fail, since we have nothing to verify
    // against.
    let euid = nix::unistd::geteuid();
    let user = match nix::unistd::User::from_uid(euid) {
        Ok(Some(u)) => u,
        Ok(None) => {
            eprintln!("skipping: no passwd entry for euid {}", euid.as_raw());
            return;
        }
        Err(e) => {
            eprintln!("skipping: User::from_uid({}) failed: {e}", euid.as_raw(),);
            return;
        }
    };
    privilege::run_as(&user.name).expect("run_as on current effective user is a no-op");
}
