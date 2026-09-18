// standard crates
use std::cell::{Cell, RefCell};
use std::time::Duration;

// internal crates
use miru_agent::shutdown::{Latch, RunOutcome};
use miru_agent::windows::errors::ScmErr;
use miru_agent::windows::scm::{self, StatusSink};

// external crates
use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
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
                trace: miru_agent::trace!(),
            });
        }
        self.reported.borrow_mut().push(status);
        Ok(())
    }
}

fn stop_pending() -> ServiceStatus {
    scm::status(ServiceState::StopPending, ServiceExitCode::NO_ERROR)
}

pub mod handle_control {
    use super::*;

    #[test]
    fn stop_reports_stop_pending_and_triggers() {
        let latch = Latch::new();
        let sink = RecordingSink::default();

        let result = scm::handle_control(ServiceControl::Stop, &latch, Some(&sink));

        assert!(matches!(result, ServiceControlHandlerResult::NoError));
        assert!(latch.is_triggered());
        assert_eq!(sink.reported(), vec![stop_pending()]);
    }

    #[test]
    fn shutdown_reports_stop_pending_and_triggers() {
        let latch = Latch::new();
        let sink = RecordingSink::default();

        let result = scm::handle_control(ServiceControl::Shutdown, &latch, Some(&sink));

        assert!(matches!(result, ServiceControlHandlerResult::NoError));
        assert!(latch.is_triggered());
        assert_eq!(sink.reported(), vec![stop_pending()]);
    }

    #[test]
    fn interrogate_is_acknowledged_without_side_effects() {
        let latch = Latch::new();
        let sink = RecordingSink::default();

        let result = scm::handle_control(ServiceControl::Interrogate, &latch, Some(&sink));

        assert!(matches!(result, ServiceControlHandlerResult::NoError));
        assert!(!latch.is_triggered());
        assert!(sink.reported().is_empty());
    }

    #[test]
    fn pause_is_not_implemented() {
        let latch = Latch::new();
        let sink = RecordingSink::default();

        let result = scm::handle_control(ServiceControl::Pause, &latch, Some(&sink));

        assert!(matches!(
            result,
            ServiceControlHandlerResult::NotImplemented
        ));
        assert!(!latch.is_triggered());
        assert!(sink.reported().is_empty());
    }

    #[test]
    fn stop_without_sink_still_triggers() {
        let latch = Latch::new();

        let result = scm::handle_control(ServiceControl::Stop, &latch, None::<&RecordingSink>);

        assert!(matches!(result, ServiceControlHandlerResult::NoError));
        assert!(latch.is_triggered());
    }

    #[test]
    fn stop_with_failing_sink_still_triggers() {
        let latch = Latch::new();
        let sink = RecordingSink::failing_on(ServiceState::StopPending);

        let result = scm::handle_control(ServiceControl::Stop, &latch, Some(&sink));

        assert!(matches!(result, ServiceControlHandlerResult::NoError));
        assert!(latch.is_triggered());
        assert!(sink.reported().is_empty());
    }
}

pub mod status {
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
            scm::status(ServiceState::Running, ServiceExitCode::NO_ERROR),
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
            scm::status(ServiceState::StartPending, ServiceExitCode::NO_ERROR),
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
            scm::status(ServiceState::Stopped, ServiceExitCode::ServiceSpecific(1)),
            expected(
                ServiceState::Stopped,
                ServiceControlAccept::empty(),
                ServiceExitCode::ServiceSpecific(1),
                Duration::ZERO,
            ),
        );
    }
}

pub mod exit_code {
    use super::*;

    #[test]
    fn completed_is_no_error() {
        assert_eq!(
            scm::exit_code(RunOutcome::Completed),
            ServiceExitCode::NO_ERROR
        );
    }

    #[test]
    fn failed_is_service_specific_one() {
        assert_eq!(
            scm::exit_code(RunOutcome::Failed),
            ServiceExitCode::ServiceSpecific(1)
        );
    }
}

pub mod run_lifecycle {
    use super::*;

    #[test]
    fn reports_the_full_state_sequence_around_the_body() {
        let sink = RecordingSink::default();
        let ran = Cell::new(false);

        let result = scm::run_lifecycle(&sink, || {
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

            let result = scm::run_lifecycle(&sink, || outcome);

            assert!(result.is_ok(), "{outcome:?}");
            let last = sink.reported().pop().expect("Stopped is reported");
            assert_eq!(
                last,
                scm::status(ServiceState::Stopped, scm::exit_code(outcome)),
                "{outcome:?}",
            );
        }
    }

    #[test]
    fn start_pending_failure_skips_the_body() {
        let sink = RecordingSink::failing_on(ServiceState::StartPending);
        let ran = Cell::new(false);

        let result = scm::run_lifecycle(&sink, || {
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

        let result = scm::run_lifecycle(&sink, || {
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

        let result = scm::run_lifecycle(&sink, || RunOutcome::Failed);

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
