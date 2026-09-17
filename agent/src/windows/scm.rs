//! Windows Service Control Manager (SCM) integration.
//!
//! [`dispatch`] hands the process to the SCM, which calls `service_main` back
//! on its own thread. That thread registers the control handler, reports the
//! `StartPending → Running → StopPending → Stopped` lifecycle, and runs the
//! agent body. Only [`dispatch`], `service_main`, and the [`StatusSink`] impl
//! for [`ServiceStatusHandle`] touch Win32; [`handle_control`], [`status`],
//! [`exit_code`], and [`run_lifecycle`] are pure and testable without a
//! registered service.

// standard crates
use std::ffi::OsString;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

// internal crates
use crate::shutdown::{RunOutcome, StopSignal};
use crate::trace;
use crate::windows::errors::ScmErr;

// external crates
use windows_service::define_windows_service;
use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{
    self, ServiceControlHandlerResult, ServiceStatusHandle,
};
use windows_service::service_dispatcher;

/// Name the service is registered under; must match the installer's
/// `ServiceInstall Name`.
pub const SERVICE_NAME: &str = "miru-agent";

/// `wait_hint` reported with `StartPending` and `StopPending`.
const PENDING_WAIT_HINT: Duration = Duration::from_secs(30);

/// Win32 `ERROR_FAILED_SERVICE_CONTROLLER_CONNECT`: the process was not
/// started by the SCM.
const ERROR_FAILED_SERVICE_CONTROLLER_CONNECT: i32 = 1063;

/// The agent body run on the SCM's service thread. It receives the stop relay
/// the control handler triggers on STOP / SHUTDOWN and reports how it ended.
pub type ServiceBody = fn(StopSignal) -> RunOutcome;

static BODY: OnceLock<ServiceBody> = OnceLock::new();

/// Registers `body` as the service entry point and blocks the calling thread
/// until the service stops. Fails with [`ScmErr::NotLaunchedByScm`] when
/// the process was not started by the SCM.
pub fn dispatch(body: ServiceBody) -> Result<(), ScmErr> {
    let _ = BODY.set(body);
    match service_dispatcher::start(SERVICE_NAME, ffi_service_main) {
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

define_windows_service!(ffi_service_main, service_main);

/// SCM entry point, called on a thread the SCM owns.
fn service_main(_args: Vec<OsString>) {
    let Some(body) = BODY.get() else { return };
    let stop = StopSignal::new();
    let handler_stop = stop.clone();
    let slot: Arc<OnceLock<ServiceStatusHandle>> = Arc::default();
    let handler_slot = Arc::clone(&slot);
    let handle = match service_control_handler::register(SERVICE_NAME, move |control| {
        handle_control(control, &handler_stop, handler_slot.get())
    }) {
        Ok(handle) => handle,
        // Without a status handle nothing can be reported to the SCM.
        Err(_) => return,
    };
    // The SCM sends no controls before `Running` is reported, so the handler
    // always finds the handle once it can be invoked.
    let _ = slot.set(handle);
    let _ = run_lifecycle(&handle, || body(stop));
}

/// SCM control callback. Runs on the SCM's handler thread: synchronous, no
/// awaits, no locks held. STOP and SHUTDOWN report `StopPending` through
/// `sink` when one is available and trigger `stop`; a failed report never
/// blocks the stop. INTERROGATE is acknowledged; everything else is
/// unimplemented.
pub fn handle_control<S: StatusSink>(
    control: ServiceControl,
    stop: &StopSignal,
    sink: Option<&S>,
) -> ServiceControlHandlerResult {
    match control {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            if let Some(sink) = sink {
                let _ = sink.report(status(ServiceState::StopPending, ServiceExitCode::NO_ERROR));
            }
            stop.trigger();
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    }
}

/// Builds the status report for `state`: an own-process service that accepts
/// STOP and SHUTDOWN only while `Running`, with [`PENDING_WAIT_HINT`] on the
/// pending states and no wait hint otherwise.
pub fn status(state: ServiceState, exit_code: ServiceExitCode) -> ServiceStatus {
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
pub fn exit_code(outcome: RunOutcome) -> ServiceExitCode {
    match outcome {
        RunOutcome::Completed => ServiceExitCode::NO_ERROR,
        RunOutcome::Failed => ServiceExitCode::ServiceSpecific(1),
    }
}

/// Destination for service status reports: the real [`ServiceStatusHandle`]
/// in production, a recording fake in tests.
pub trait StatusSink {
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

/// Reports `StartPending` then `Running`, runs `body`, then always attempts
/// both `StopPending` and `Stopped` (carrying the body's exit code). A report
/// failure before `Running` skips the body; the first error encountered is
/// returned.
pub fn run_lifecycle<S: StatusSink>(
    sink: &S,
    body: impl FnOnce() -> RunOutcome,
) -> Result<(), ScmErr> {
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
