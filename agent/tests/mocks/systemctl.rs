// standard crates
use std::sync::{Arc, Mutex};

// internal crates
use miru_agent::installer::provision::{SystemctlI, SystemdErr};

type SystemctlFn = Mutex<Box<dyn Fn(&str) -> Result<(), SystemdErr> + Send + Sync>>;

/// Records a single systemctl invocation (verb + unit name) made by
/// [`MockSystemctl`]. Tests inspect the call log to assert ordering and the
/// total number of invocations.
#[derive(Clone, Debug, PartialEq)]
pub struct SystemctlCall {
    pub verb: String,
    pub unit: String,
}

/// Test double for [`SystemctlI`]. Both `stop` and `restart` default to
/// returning `Ok(())`. Use [`MockSystemctl::set_stop`] / `set_restart` to
/// override per-test behavior. The captured call log is available via
/// [`MockSystemctl::calls`].
pub struct MockSystemctl {
    stop_fn: SystemctlFn,
    restart_fn: SystemctlFn,
    calls: Arc<Mutex<Vec<SystemctlCall>>>,
}

impl Default for MockSystemctl {
    fn default() -> Self {
        Self {
            stop_fn: Mutex::new(Box::new(|_unit| Ok(()))),
            restart_fn: Mutex::new(Box::new(|_unit| Ok(()))),
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl MockSystemctl {
    pub fn set_stop<F>(&self, f: F)
    where
        F: Fn(&str) -> Result<(), SystemdErr> + Send + Sync + 'static,
    {
        *self.stop_fn.lock().unwrap() = Box::new(f);
    }

    pub fn set_restart<F>(&self, f: F)
    where
        F: Fn(&str) -> Result<(), SystemdErr> + Send + Sync + 'static,
    {
        *self.restart_fn.lock().unwrap() = Box::new(f);
    }

    pub fn calls(&self) -> Vec<SystemctlCall> {
        self.calls.lock().unwrap().clone()
    }
}

impl SystemctlI for MockSystemctl {
    fn stop(&self, unit: &str) -> Result<(), SystemdErr> {
        self.calls.lock().unwrap().push(SystemctlCall {
            verb: "stop".to_string(),
            unit: unit.to_string(),
        });
        (self.stop_fn.lock().unwrap())(unit)
    }

    fn restart(&self, unit: &str) -> Result<(), SystemdErr> {
        self.calls.lock().unwrap().push(SystemctlCall {
            verb: "restart".to_string(),
            unit: unit.to_string(),
        });
        (self.restart_fn.lock().unwrap())(unit)
    }
}
