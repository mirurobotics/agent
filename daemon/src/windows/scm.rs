//! Windows Service Control Manager (SCM) integration.
//!
//! [`dispatch`] hands the process to the SCM. The binary owns `ServiceMain`
//! (next to the agent body) and calls [`run`] from that callback. [`run`]
//! registers the control handler, reports the
//! `StartPending → Running → StopPending → Stopped` lifecycle, and runs the
//! body. Only [`dispatch`], [`run`], and the [`StatusSink`] impl for
//! [`ServiceStatusHandle`] touch Win32. The control handler only trips the
//! latch; status reports stay in [`run_lifecycle`]. The helpers are private
//! and tested in this file.

// standard crates
use std::time::Duration;

// internal crates
use crate::shutdown::{Latch, RunOutcome};
use crate::trace;
use crate::windows::errors::ScmErr;

// external crates
use tracing::error;
use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{
    self, ServiceControlHandlerResult, ServiceStatusHandle,
};
use windows_service::service_dispatcher;

/// Name the service is registered under; must match the installer's
/// `ServiceInstall Name`.
const SERVICE_NAME: &str = "miru-agent";

/// Blocks the calling thread until the service stops. `service_main` is the
/// `extern "system"` thunk from [`windows_service::define_windows_service`].
/// Fails with [`ScmErr::NotLaunchedByScm`] when the process was not started
/// by the SCM.
pub fn dispatch(service_main: extern "system" fn(u32, *mut *mut u16)) -> Result<(), ScmErr> {
    // Win32 `ERROR_FAILED_SERVICE_CONTROLLER_CONNECT`: this process was not
    // started by the SCM.
    const ERROR_FAILED_SERVICE_CONTROLLER_CONNECT: i32 = 1063;

    match service_dispatcher::start(SERVICE_NAME, service_main) {
        Ok(()) => Ok(()),
        Err(windows_service::Error::Winapi(ref e))
            if e.raw_os_error() == Some(ERROR_FAILED_SERVICE_CONTROLLER_CONNECT) =>
        {
            Err(ScmErr::NotLaunchedByScm { trace: trace!() })
        }
        Err(source) => Err(ScmErr::Scm {
            source,
            trace: trace!(),
        }),
    }
}

/// Registers the control handler and runs `body` on the SCM's service thread.
pub fn run(body: impl FnOnce(Latch) -> RunOutcome) {
    let latch = Latch::new();
    let handler_latch = latch.clone();
    let handle = match service_control_handler::register(SERVICE_NAME, move |control| {
        handle_control(control, &handler_latch)
    }) {
        Ok(handle) => handle,
        // Without a status handle nothing can be reported to the SCM.
        // Tracing is not installed yet (`body` never runs).
        Err(e) => {
            eprintln!("miru-agent: failed to register service control handler: {e}");
            return;
        }
    };
    if let Err(e) = run_lifecycle(&handle, || body(latch)) {
        // Tracing is only installed inside `body`; StartPending/Running
        // failures happen before that, so also write stderr.
        eprintln!("miru-agent: failed to report service status: {e}");
        error!("failed to report service status: {e}");
    }
}

/// SCM control callback. Runs on the SCM's handler thread: synchronous, no
/// awaits, no locks held. STOP and SHUTDOWN trip `latch` only; status stays
/// `Running` until [`run_lifecycle`] reports `StopPending` after `body`
/// returns, so a long in-flight reset is not charged against the wait hint.
/// INTERROGATE is acknowledged; everything else is unimplemented.
fn handle_control(control: ServiceControl, latch: &Latch) -> ServiceControlHandlerResult {
    match control {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            latch.trigger();
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    }
}

/// Reports `StartPending` then `Running`, runs `body`, then always attempts
/// both `StopPending` and `Stopped` (carrying the body's exit code). A report
/// failure before `Running` skips the body; the first error encountered is
/// returned.
fn run_lifecycle<S: StatusSink>(sink: &S, body: impl FnOnce() -> RunOutcome) -> Result<(), ScmErr> {
    sink.report(status(
        ServiceState::StartPending,
        ServiceExitCode::NO_ERROR,
    ))?;
    sink.report(status(ServiceState::Running, ServiceExitCode::NO_ERROR))?;
    let outcome = body();
    let stop_pending = sink.report(status(ServiceState::StopPending, ServiceExitCode::NO_ERROR));
    let stopped = sink.report(status(ServiceState::Stopped, exit_code(outcome)));
    stop_pending.and(stopped)
}

/// Builds the status report for `state`: an own-process service that accepts
/// STOP and SHUTDOWN only while `Running`, with a 30s wait hint on the
/// pending states and no wait hint otherwise.
fn status(state: ServiceState, exit_code: ServiceExitCode) -> ServiceStatus {
    const PENDING_WAIT_HINT: Duration = Duration::from_secs(30);

    let controls_accepted = if state == ServiceState::Running {
        ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN
    } else {
        ServiceControlAccept::empty()
    };
    let wait_hint = match state {
        ServiceState::StartPending | ServiceState::StopPending => PENDING_WAIT_HINT,
        _ => Duration::ZERO,
    };
    ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted,
        exit_code,
        checkpoint: 0,
        wait_hint,
        process_id: None,
    }
}

/// Maps the body's outcome to the exit code reported with `Stopped`.
fn exit_code(outcome: RunOutcome) -> ServiceExitCode {
    match outcome {
        RunOutcome::Completed => ServiceExitCode::NO_ERROR,
        RunOutcome::Failed => ServiceExitCode::ServiceSpecific(1),
    }
}

/// Destination for service status reports: the real [`ServiceStatusHandle`]
/// in production, a recording fake in tests.
trait StatusSink {
    fn report(&self, status: ServiceStatus) -> Result<(), ScmErr>;
}

impl StatusSink for ServiceStatusHandle {
    fn report(&self, status: ServiceStatus) -> Result<(), ScmErr> {
        self.set_service_status(status)
            .map_err(|source| ScmErr::Scm {
                source,
                trace: trace!(),
            })
    }
}

#[cfg(test)]
mod tests {
    // standard crates
    use std::cell::{Cell, RefCell};
    use std::time::Duration;

    // internal crates
    use super::StatusSink;
    use crate::shutdown::{Latch, RunOutcome};
    use crate::windows::errors::ScmErr;

    // external crates
    use windows_service::service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
        ServiceType,
    };
    use windows_service::service_control_handler::ServiceControlHandlerResult;

    /// Records every successful report; reports for `fail_on` fail without being
    /// recorded.
    #[derive(Default)]
    struct RecordingSink {
        reported: RefCell<Vec<ServiceStatus>>,
        fail_on: Option<ServiceState>,
    }

    impl RecordingSink {
        fn failing_on(state: ServiceState) -> Self {
            Self {
                reported: RefCell::default(),
                fail_on: Some(state),
            }
        }

        fn reported(self) -> Vec<ServiceStatus> {
            self.reported.into_inner()
        }

        fn states(&self) -> Vec<ServiceState> {
            self.reported
                .borrow()
                .iter()
                .map(|status| status.current_state)
                .collect()
        }
    }

    impl StatusSink for RecordingSink {
        fn report(&self, status: ServiceStatus) -> Result<(), ScmErr> {
            if self.fail_on == Some(status.current_state) {
                return Err(ScmErr::NotLaunchedByScm {
                    trace: crate::trace!(),
                });
            }
            self.reported.borrow_mut().push(status);
            Ok(())
        }
    }

    fn stop_pending() -> ServiceStatus {
        super::status(ServiceState::StopPending, ServiceExitCode::NO_ERROR)
    }

    mod handle_control {
        use super::super::handle_control;
        use super::*;

        #[test]
        fn stop_triggers_without_reporting() {
            let latch = Latch::new();

            let result = handle_control(ServiceControl::Stop, &latch);

            assert!(matches!(result, ServiceControlHandlerResult::NoError));
            assert!(latch.is_triggered());
        }

        #[test]
        fn shutdown_triggers_without_reporting() {
            let latch = Latch::new();

            let result = handle_control(ServiceControl::Shutdown, &latch);

            assert!(matches!(result, ServiceControlHandlerResult::NoError));
            assert!(latch.is_triggered());
        }

        #[test]
        fn interrogate_is_acknowledged_without_side_effects() {
            let latch = Latch::new();

            let result = handle_control(ServiceControl::Interrogate, &latch);

            assert!(matches!(result, ServiceControlHandlerResult::NoError));
            assert!(!latch.is_triggered());
        }

        #[test]
        fn pause_is_not_implemented() {
            let latch = Latch::new();

            let result = handle_control(ServiceControl::Pause, &latch);

            assert!(matches!(
                result,
                ServiceControlHandlerResult::NotImplemented
            ));
            assert!(!latch.is_triggered());
        }
    }

    mod run_lifecycle {
        use super::super::{exit_code, run_lifecycle, status};
        use super::*;

        #[test]
        fn reports_the_full_state_sequence_around_the_body() {
            let sink = RecordingSink::default();
            let ran = Cell::new(false);

            let result = run_lifecycle(&sink, || {
                ran.set(true);
                RunOutcome::Completed
            });

            assert!(result.is_ok());
            assert!(ran.get());
            assert_eq!(
                sink.states(),
                vec![
                    ServiceState::StartPending,
                    ServiceState::Running,
                    ServiceState::StopPending,
                    ServiceState::Stopped,
                ],
            );
        }

        #[test]
        fn stopped_carries_the_exit_code_for_each_outcome() {
            for outcome in [RunOutcome::Completed, RunOutcome::Failed] {
                let sink = RecordingSink::default();

                let result = run_lifecycle(&sink, || outcome);

                assert!(result.is_ok(), "{outcome:?}");
                let last = sink.reported().pop().expect("Stopped is reported");
                assert_eq!(
                    last,
                    status(ServiceState::Stopped, exit_code(outcome)),
                    "{outcome:?}",
                );
            }
        }

        #[test]
        fn start_pending_failure_skips_the_body() {
            let sink = RecordingSink::failing_on(ServiceState::StartPending);
            let ran = Cell::new(false);

            let result = run_lifecycle(&sink, || {
                ran.set(true);
                RunOutcome::Completed
            });

            assert!(matches!(result, Err(ScmErr::NotLaunchedByScm { .. })));
            assert!(!ran.get());
            assert!(sink.reported().is_empty());
        }

        #[test]
        fn running_failure_skips_the_body() {
            let sink = RecordingSink::failing_on(ServiceState::Running);
            let ran = Cell::new(false);

            let result = run_lifecycle(&sink, || {
                ran.set(true);
                RunOutcome::Completed
            });

            assert!(result.is_err());
            assert!(!ran.get());
            assert_eq!(sink.states(), vec![ServiceState::StartPending]);
        }

        #[test]
        fn stop_pending_failure_still_reports_stopped() {
            let sink = RecordingSink::failing_on(ServiceState::StopPending);

            let result = run_lifecycle(&sink, || RunOutcome::Failed);

            assert!(matches!(result, Err(ScmErr::NotLaunchedByScm { .. })));
            assert_eq!(
                sink.states(),
                vec![
                    ServiceState::StartPending,
                    ServiceState::Running,
                    ServiceState::Stopped,
                ],
            );
        }
    }

    mod status {
        use super::super::status;
        use super::*;

        fn expected(
            state: ServiceState,
            controls_accepted: ServiceControlAccept,
            exit_code: ServiceExitCode,
            wait_hint: Duration,
        ) -> ServiceStatus {
            ServiceStatus {
                service_type: ServiceType::OWN_PROCESS,
                current_state: state,
                controls_accepted,
                exit_code,
                checkpoint: 0,
                wait_hint,
                process_id: None,
            }
        }

        #[test]
        fn running_accepts_stop_and_shutdown_with_no_wait_hint() {
            assert_eq!(
                status(ServiceState::Running, ServiceExitCode::NO_ERROR),
                expected(
                    ServiceState::Running,
                    ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
                    ServiceExitCode::NO_ERROR,
                    Duration::ZERO,
                ),
            );
        }

        #[test]
        fn start_pending_accepts_nothing_with_thirty_second_hint() {
            assert_eq!(
                status(ServiceState::StartPending, ServiceExitCode::NO_ERROR),
                expected(
                    ServiceState::StartPending,
                    ServiceControlAccept::empty(),
                    ServiceExitCode::NO_ERROR,
                    Duration::from_secs(30),
                ),
            );
        }

        #[test]
        fn stop_pending_accepts_nothing_with_thirty_second_hint() {
            assert_eq!(
                stop_pending(),
                expected(
                    ServiceState::StopPending,
                    ServiceControlAccept::empty(),
                    ServiceExitCode::NO_ERROR,
                    Duration::from_secs(30),
                ),
            );
        }

        #[test]
        fn stopped_carries_the_exit_code() {
            assert_eq!(
                status(ServiceState::Stopped, ServiceExitCode::ServiceSpecific(1)),
                expected(
                    ServiceState::Stopped,
                    ServiceControlAccept::empty(),
                    ServiceExitCode::ServiceSpecific(1),
                    Duration::ZERO,
                ),
            );
        }
    }

    mod exit_code {
        use super::super::exit_code;
        use super::*;

        #[test]
        fn completed_is_no_error() {
            assert_eq!(exit_code(RunOutcome::Completed), ServiceExitCode::NO_ERROR);
        }

        #[test]
        fn failed_is_service_specific_one() {
            assert_eq!(
                exit_code(RunOutcome::Failed),
                ServiceExitCode::ServiceSpecific(1)
            );
        }
    }
}
