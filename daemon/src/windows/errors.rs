// internal crates
use crate::errors::Trace;

#[derive(Debug, thiserror::Error)]
pub enum ScmErr {
    #[error(
        "miru-agent was not started by the Windows Service Control Manager; \
         run it with --console to run in the foreground"
    )]
    NotLaunchedByScm { trace: Box<Trace> },

    #[cfg(windows)]
    #[error("service control manager call failed: {source}")]
    Scm {
        source: windows_service::Error,
        trace: Box<Trace>,
    },
}

impl crate::errors::Error for ScmErr {}
