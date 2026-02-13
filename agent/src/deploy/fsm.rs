// internal crates
use crate::errors::MiruError;
use crate::models::deployment::{
    Deployment, DeploymentActivityStatus, DeploymentErrorStatus, DeploymentTargetStatus,
};
use crate::utils::calc_exp_backoff;

// external crates
use chrono::{TimeDelta, Utc};

// ================================ NEXT ACTION ==================================== //
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NextAction {
    None,
    Deploy,
    Remove,
    Archive,
    Wait(TimeDelta),
}

pub fn next_action(deployment: &Deployment, use_cooldown: bool) -> NextAction {
    // do nothing if the status is failed
    if deployment.error_status == DeploymentErrorStatus::Failed {
        return NextAction::None;
    }

    // check for cooldown
    if use_cooldown && deployment.is_in_cooldown() {
        if let Some(cooldown_ends_at) = deployment.cooldown_ends_at {
            return NextAction::Wait(cooldown_ends_at.signed_duration_since(Utc::now()));
        }
    }

    // determine the next action
    match deployment.target_status {
        DeploymentTargetStatus::Staged => match deployment.activity_status {
            DeploymentActivityStatus::Drifted => NextAction::None,
            DeploymentActivityStatus::Staged => NextAction::None,
            DeploymentActivityStatus::Queued => NextAction::Archive,
            DeploymentActivityStatus::Deployed => NextAction::Remove,
            DeploymentActivityStatus::Archived => NextAction::None,
        },
        DeploymentTargetStatus::Deployed => match deployment.activity_status {
            DeploymentActivityStatus::Drifted => NextAction::None,
            DeploymentActivityStatus::Staged => NextAction::None,
            DeploymentActivityStatus::Queued => NextAction::Deploy,
            DeploymentActivityStatus::Deployed => NextAction::None,
            DeploymentActivityStatus::Archived => NextAction::Deploy,
        },
        DeploymentTargetStatus::Archived => match deployment.activity_status {
            DeploymentActivityStatus::Drifted => NextAction::Archive,
            DeploymentActivityStatus::Staged => NextAction::Archive,
            DeploymentActivityStatus::Queued => NextAction::Archive,
            DeploymentActivityStatus::Deployed => NextAction::Remove,
            DeploymentActivityStatus::Archived => NextAction::None,
        },
    }
}

pub fn is_action_required(action: NextAction) -> bool {
    match action {
        NextAction::None => false,
        NextAction::Deploy => true,
        NextAction::Remove => true,
        NextAction::Archive => true,
        NextAction::Wait(_) => false,
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Settings {
    pub max_attempts: u32,
    pub exp_backoff_base_secs: i64,
    pub max_cooldown_secs: i64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            max_attempts: 2147483647, // a VERY large number
            exp_backoff_base_secs: 15,
            max_cooldown_secs: 86400, // 24 hours
        }
    }
}

// ================================== TRANSITIONS ================================== //
#[derive(Debug)]
struct TransitionOptions {
    activity_status: Option<DeploymentActivityStatus>,
    error_status: Option<DeploymentErrorStatus>,
    attempts: Option<u32>,
    cooldown: Option<TimeDelta>,
}

fn transition(mut deployment: Deployment, options: TransitionOptions) -> Deployment {
    if let Some(activity_status) = options.activity_status {
        deployment.activity_status = activity_status;
    }

    if let Some(error_status) = options.error_status {
        deployment.error_status = error_status;
    }

    if let Some(attempts) = options.attempts {
        deployment.attempts = attempts;
    }

    if let Some(cooldown) = options.cooldown {
        deployment.set_cooldown(cooldown);
    }

    deployment
}

// ---------------------------- successful transitions ----------------------------= //
pub fn deploy(deployment: Deployment) -> Deployment {
    let new_activity_status = DeploymentActivityStatus::Deployed;
    let options = get_success_options(&deployment, new_activity_status);
    transition(deployment, options)
}

pub fn remove(deployment: Deployment) -> Deployment {
    let new_activity_status = DeploymentActivityStatus::Archived;
    let options = get_success_options(&deployment, new_activity_status);
    transition(deployment, options)
}

fn get_success_options(
    deployment: &Deployment,
    new_activity_status: DeploymentActivityStatus,
) -> TransitionOptions {
    TransitionOptions {
        activity_status: Some(new_activity_status),
        error_status: if has_recovered(deployment, new_activity_status) {
            Some(DeploymentErrorStatus::None)
        } else {
            None
        },
        // reset attempts and cooldown
        attempts: if has_recovered(deployment, new_activity_status) {
            Some(0)
        } else {
            None
        },
        cooldown: None,
    }
}

fn has_recovered(
    deployment: &Deployment,
    new_activity_status: DeploymentActivityStatus,
) -> bool {
    // the error status only needs to be updated if it is currently retrying. If is
    // failed then it can never exit failed and if it is None then it is already correct
    if deployment.error_status != DeploymentErrorStatus::Retrying {
        return false;
    }

    // check if the new activity status matches the deployment's target status
    match deployment.target_status {
        DeploymentTargetStatus::Staged => {
            // for staged, we're satisfied with the deployment being in other states as long as
            // it is not deployed.
            match new_activity_status {
                DeploymentActivityStatus::Drifted => true,
                DeploymentActivityStatus::Staged => true,
                DeploymentActivityStatus::Queued => true,
                DeploymentActivityStatus::Deployed => false,
                DeploymentActivityStatus::Archived => true,
            }
        }
        DeploymentTargetStatus::Deployed => match new_activity_status {
            DeploymentActivityStatus::Drifted => false,
            DeploymentActivityStatus::Staged => false,
            DeploymentActivityStatus::Queued => false,
            DeploymentActivityStatus::Deployed => true,
            DeploymentActivityStatus::Archived => false,
        },
        DeploymentTargetStatus::Archived => match new_activity_status {
            DeploymentActivityStatus::Drifted => false,
            DeploymentActivityStatus::Staged => false,
            DeploymentActivityStatus::Queued => false,
            DeploymentActivityStatus::Deployed => false,
            DeploymentActivityStatus::Archived => true,
        },
    }
}

// ----------------------------- error transitions --------------------------------- //
pub fn error(
    deployment: Deployment,
    settings: &Settings,
    e: &impl MiruError,
    increment_attempts: bool,
) -> Deployment {
    let options = get_error_options(
        &deployment,
        increment_attempts && should_increment_attempts(e),
        settings,
    );
    transition(deployment, options)
}

fn should_increment_attempts(e: &impl MiruError) -> bool {
    !e.is_network_connection_error()
}

fn get_error_options(
    deployment: &Deployment,
    increment_attempts: bool,
    settings: &Settings,
) -> TransitionOptions {
    // determine the number of attempts
    let attempts = if increment_attempts {
        deployment.attempts().saturating_add(1)
    } else {
        deployment.attempts()
    };

    // determine the new status
    let mut new_error_status = Some(DeploymentErrorStatus::Retrying);
    if attempts >= settings.max_attempts
        || deployment.error_status == DeploymentErrorStatus::Failed
    {
        new_error_status = Some(DeploymentErrorStatus::Failed);
    }

    // determine the cooldown
    let cooldown = calc_exp_backoff(
        settings.exp_backoff_base_secs,
        2,
        attempts,
        settings.max_cooldown_secs,
    );

    TransitionOptions {
        activity_status: None,
        error_status: new_error_status,
        attempts: Some(attempts),
        cooldown: Some(TimeDelta::seconds(cooldown)),
    }
}
