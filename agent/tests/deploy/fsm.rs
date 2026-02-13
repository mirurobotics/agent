use miru_agent::deploy::fsm;
use miru_agent::errors::MiruError;
use miru_agent::models::deployment::{
    Deployment, DeploymentActivityStatus, DeploymentErrorStatus, DeploymentTargetStatus,
};
use miru_agent::utils::calc_exp_backoff;

use crate::mock::MockMiruError;

// external crates
use chrono::{TimeDelta, Utc};

// ================================= NEXT ACTION =================================== //
pub mod next_action {

    use super::*;

    fn validate_eq_wait_time(expected: TimeDelta, actual: TimeDelta, tol: TimeDelta) {
        assert!(
            expected - actual > -tol,
            "expected wait time {expected} is not equal to actual wait time {actual}",
        );
        assert!(
            expected - actual < tol,
            "expected wait time {expected} is not equal to actual wait time {actual}",
        );
    }

    fn validate_next_action(expected: fsm::NextAction, actual: fsm::NextAction) {
        let expected_wait_time = match expected {
            fsm::NextAction::Wait(expected_wait_time) => expected_wait_time,
            _ => {
                assert_eq!(expected, actual);
                return;
            }
        };

        let actual_wait_time = match actual {
            fsm::NextAction::Wait(actual_wait_time) => actual_wait_time,
            _ => {
                assert_eq!(expected, actual);
                return;
            }
        };

        validate_eq_wait_time(
            expected_wait_time,
            actual_wait_time,
            TimeDelta::milliseconds(1),
        );
    }

    /// Validates next_action for a deployment across all 3 target statuses.
    /// Order: target_staged, target_deployed, target_archived
    fn validate_next_actions(
        mut deployment: Deployment,
        use_cooldown: bool,
        target_staged: fsm::NextAction,
        target_deployed: fsm::NextAction,
        target_archived: fsm::NextAction,
    ) {
        deployment.target_status = DeploymentTargetStatus::Staged;
        validate_next_action(target_staged, fsm::next_action(&deployment, use_cooldown));
        deployment.target_status = DeploymentTargetStatus::Deployed;
        validate_next_action(target_deployed, fsm::next_action(&deployment, use_cooldown));
        deployment.target_status = DeploymentTargetStatus::Archived;
        validate_next_action(target_archived, fsm::next_action(&deployment, use_cooldown));
    }

    // From the FSM table in deploy/fsm.rs:
    //
    //  target\activity | Drifted | Staged | Queued  | Deployed | Archived
    //  ----------------+---------+--------+---------+----------+---------
    //  Staged          | None    | None   | Archive | Remove   | None
    //  Deployed        | None    | None   | Deploy  | None     | Deploy
    //  Archived        | Archive | Archive| Archive | Remove   | None

    #[test]
    fn drifted_activity_status() {
        let mut deployment = Deployment {
            activity_status: DeploymentActivityStatus::Drifted,
            error_status: DeploymentErrorStatus::None,
            ..Default::default()
        };

        // error status 'None' or 'Retrying' && not in cooldown
        for i in 0..2 {
            deployment.error_status = if i == 0 {
                DeploymentErrorStatus::None
            } else {
                DeploymentErrorStatus::Retrying
            };
            // Staged=None, Deployed=None, Archived=Archive
            validate_next_actions(
                deployment.clone(),
                true,
                fsm::NextAction::None,
                fsm::NextAction::None,
                fsm::NextAction::Archive,
            );
        }

        // error status 'None' or 'Retrying' && in cooldown
        for i in 0..2 {
            deployment.error_status = if i == 0 {
                DeploymentErrorStatus::None
            } else {
                DeploymentErrorStatus::Retrying
            };
            let cooldown = TimeDelta::minutes(60);
            deployment.set_cooldown(cooldown);

            // using cooldown
            validate_next_actions(
                deployment.clone(),
                true,
                fsm::NextAction::Wait(cooldown),
                fsm::NextAction::Wait(cooldown),
                fsm::NextAction::Wait(cooldown),
            );

            // ignore cooldown
            validate_next_actions(
                deployment.clone(),
                false,
                fsm::NextAction::None,
                fsm::NextAction::None,
                fsm::NextAction::Archive,
            );
        }

        // error status 'Failed'
        deployment.error_status = DeploymentErrorStatus::Failed;
        validate_next_actions(
            deployment.clone(),
            true,
            fsm::NextAction::None,
            fsm::NextAction::None,
            fsm::NextAction::None,
        );
    }

    #[test]
    fn staged_activity_status() {
        let mut deployment = Deployment {
            activity_status: DeploymentActivityStatus::Staged,
            error_status: DeploymentErrorStatus::None,
            ..Default::default()
        };

        // error status 'None' or 'Retrying' && not in cooldown
        for i in 0..2 {
            deployment.error_status = if i == 0 {
                DeploymentErrorStatus::None
            } else {
                DeploymentErrorStatus::Retrying
            };
            // Staged=None, Deployed=None, Archived=Archive
            validate_next_actions(
                deployment.clone(),
                true,
                fsm::NextAction::None,
                fsm::NextAction::None,
                fsm::NextAction::Archive,
            );
        }

        // error status 'None' or 'Retrying' && in cooldown
        for i in 0..2 {
            deployment.error_status = if i == 0 {
                DeploymentErrorStatus::None
            } else {
                DeploymentErrorStatus::Retrying
            };
            let cooldown = TimeDelta::minutes(60);
            deployment.set_cooldown(cooldown);

            validate_next_actions(
                deployment.clone(),
                true,
                fsm::NextAction::Wait(cooldown),
                fsm::NextAction::Wait(cooldown),
                fsm::NextAction::Wait(cooldown),
            );

            validate_next_actions(
                deployment.clone(),
                false,
                fsm::NextAction::None,
                fsm::NextAction::None,
                fsm::NextAction::Archive,
            );
        }

        // error status 'Failed'
        deployment.error_status = DeploymentErrorStatus::Failed;
        validate_next_actions(
            deployment.clone(),
            true,
            fsm::NextAction::None,
            fsm::NextAction::None,
            fsm::NextAction::None,
        );
    }

    #[test]
    fn queued_activity_status() {
        let mut deployment = Deployment {
            activity_status: DeploymentActivityStatus::Queued,
            error_status: DeploymentErrorStatus::None,
            ..Default::default()
        };

        // error status 'None' or 'Retrying' && not in cooldown
        for i in 0..2 {
            deployment.error_status = if i == 0 {
                DeploymentErrorStatus::None
            } else {
                DeploymentErrorStatus::Retrying
            };
            // Staged=Archive, Deployed=Deploy, Archived=Archive
            validate_next_actions(
                deployment.clone(),
                true,
                fsm::NextAction::Archive,
                fsm::NextAction::Deploy,
                fsm::NextAction::Archive,
            );
        }

        // error status 'None' or 'Retrying' && in cooldown
        for i in 0..2 {
            deployment.error_status = if i == 0 {
                DeploymentErrorStatus::None
            } else {
                DeploymentErrorStatus::Retrying
            };
            let cooldown = TimeDelta::minutes(60);
            deployment.set_cooldown(cooldown);

            validate_next_actions(
                deployment.clone(),
                true,
                fsm::NextAction::Wait(cooldown),
                fsm::NextAction::Wait(cooldown),
                fsm::NextAction::Wait(cooldown),
            );

            validate_next_actions(
                deployment.clone(),
                false,
                fsm::NextAction::Archive,
                fsm::NextAction::Deploy,
                fsm::NextAction::Archive,
            );
        }

        // error status 'Failed'
        deployment.error_status = DeploymentErrorStatus::Failed;
        validate_next_actions(
            deployment.clone(),
            true,
            fsm::NextAction::None,
            fsm::NextAction::None,
            fsm::NextAction::None,
        );
    }

    #[test]
    fn deployed_activity_status() {
        let mut deployment = Deployment {
            activity_status: DeploymentActivityStatus::Deployed,
            error_status: DeploymentErrorStatus::None,
            ..Default::default()
        };

        // error status 'None' or 'Retrying' && not in cooldown
        for i in 0..2 {
            deployment.error_status = if i == 0 {
                DeploymentErrorStatus::None
            } else {
                DeploymentErrorStatus::Retrying
            };
            // Staged=Remove, Deployed=None, Archived=Remove
            validate_next_actions(
                deployment.clone(),
                true,
                fsm::NextAction::Remove,
                fsm::NextAction::None,
                fsm::NextAction::Remove,
            );
        }

        // error status 'None' or 'Retrying' && in cooldown
        for i in 0..2 {
            deployment.error_status = if i == 0 {
                DeploymentErrorStatus::None
            } else {
                DeploymentErrorStatus::Retrying
            };
            let cooldown = TimeDelta::minutes(60);
            deployment.set_cooldown(cooldown);

            validate_next_actions(
                deployment.clone(),
                true,
                fsm::NextAction::Wait(cooldown),
                fsm::NextAction::Wait(cooldown),
                fsm::NextAction::Wait(cooldown),
            );

            validate_next_actions(
                deployment.clone(),
                false,
                fsm::NextAction::Remove,
                fsm::NextAction::None,
                fsm::NextAction::Remove,
            );
        }

        // error status 'Failed'
        deployment.error_status = DeploymentErrorStatus::Failed;
        validate_next_actions(
            deployment.clone(),
            true,
            fsm::NextAction::None,
            fsm::NextAction::None,
            fsm::NextAction::None,
        );
    }

    #[test]
    fn archived_activity_status() {
        let mut deployment = Deployment {
            activity_status: DeploymentActivityStatus::Archived,
            error_status: DeploymentErrorStatus::None,
            ..Default::default()
        };

        // error status 'None' or 'Retrying' && not in cooldown
        for i in 0..2 {
            deployment.error_status = if i == 0 {
                DeploymentErrorStatus::None
            } else {
                DeploymentErrorStatus::Retrying
            };
            // Staged=None, Deployed=Deploy, Archived=None
            validate_next_actions(
                deployment.clone(),
                true,
                fsm::NextAction::None,
                fsm::NextAction::Deploy,
                fsm::NextAction::None,
            );
        }

        // error status 'None' or 'Retrying' && in cooldown
        for i in 0..2 {
            deployment.error_status = if i == 0 {
                DeploymentErrorStatus::None
            } else {
                DeploymentErrorStatus::Retrying
            };
            let cooldown = TimeDelta::minutes(60);
            deployment.set_cooldown(cooldown);

            validate_next_actions(
                deployment.clone(),
                true,
                fsm::NextAction::Wait(cooldown),
                fsm::NextAction::Wait(cooldown),
                fsm::NextAction::Wait(cooldown),
            );

            validate_next_actions(
                deployment.clone(),
                false,
                fsm::NextAction::None,
                fsm::NextAction::Deploy,
                fsm::NextAction::None,
            );
        }

        // error status 'Failed'
        deployment.error_status = DeploymentErrorStatus::Failed;
        validate_next_actions(
            deployment.clone(),
            true,
            fsm::NextAction::None,
            fsm::NextAction::None,
            fsm::NextAction::None,
        );
    }
}

#[test]
fn is_action_required() {
    assert!(!fsm::is_action_required(fsm::NextAction::None));
    assert!(fsm::is_action_required(fsm::NextAction::Deploy));
    assert!(fsm::is_action_required(fsm::NextAction::Remove));
    assert!(fsm::is_action_required(fsm::NextAction::Archive));
    assert!(!fsm::is_action_required(fsm::NextAction::Wait(
        TimeDelta::minutes(1)
    )));
}

// ================================= TRANSITIONS =================================== //
pub mod transitions {
    use super::*;

    fn def_deps_w_all_status_combos() -> Vec<Deployment> {
        let mut deployments = Vec::new();
        for activity in DeploymentActivityStatus::variants() {
            for target in DeploymentTargetStatus::variants() {
                for error in DeploymentErrorStatus::variants() {
                    deployments.push(Deployment {
                        activity_status: activity,
                        target_status: target,
                        error_status: error,
                        ..Default::default()
                    });
                }
            }
        }
        deployments
    }

    fn def_deps_w_error_status(error_status: DeploymentErrorStatus) -> Vec<Deployment> {
        let mut deployments = Vec::new();
        for activity in DeploymentActivityStatus::variants() {
            for target in DeploymentTargetStatus::variants() {
                deployments.push(Deployment {
                    activity_status: activity,
                    target_status: target,
                    error_status,
                    ..Default::default()
                });
            }
        }
        deployments
    }

    fn validate_deploy_transition(
        deployment: Deployment,
        expected_error_status: DeploymentErrorStatus,
    ) {
        let actual = fsm::deploy(deployment.clone());

        let expected = Deployment {
            activity_status: DeploymentActivityStatus::Deployed,
            error_status: expected_error_status,
            attempts: if expected_error_status == DeploymentErrorStatus::None
                && deployment.error_status == DeploymentErrorStatus::Retrying
            {
                0
            } else {
                deployment.attempts
            },
            cooldown_ends_at: actual.cooldown_ends_at,
            ..deployment.clone()
        };
        assert!(
            expected == actual,
            "expected:\n{expected:?}\n actual:\n{actual:?}\n",
        );
    }

    #[test]
    fn deploy_error_status_none() {
        let deployments = def_deps_w_error_status(DeploymentErrorStatus::None);
        for deployment in deployments {
            validate_deploy_transition(deployment, DeploymentErrorStatus::None);
        }
    }

    #[test]
    fn deploy_error_status_retrying() {
        let deployments = def_deps_w_error_status(DeploymentErrorStatus::Retrying);
        for deployment in deployments {
            match deployment.target_status {
                DeploymentTargetStatus::Deployed => {
                    validate_deploy_transition(deployment, DeploymentErrorStatus::None)
                }
                _ => validate_deploy_transition(deployment, DeploymentErrorStatus::Retrying),
            }
        }
    }

    #[test]
    fn deploy_error_status_failed() {
        let deployments = def_deps_w_error_status(DeploymentErrorStatus::Failed);
        for deployment in deployments {
            validate_deploy_transition(deployment, DeploymentErrorStatus::Failed);
        }
    }

    fn validate_remove_transition(
        deployment: Deployment,
        expected_error_status: DeploymentErrorStatus,
    ) {
        let actual = fsm::remove(deployment.clone());

        let expected = Deployment {
            activity_status: DeploymentActivityStatus::Archived,
            error_status: expected_error_status,
            attempts: if expected_error_status == DeploymentErrorStatus::None
                && deployment.error_status == DeploymentErrorStatus::Retrying
            {
                0
            } else {
                deployment.attempts
            },
            cooldown_ends_at: actual.cooldown_ends_at,
            ..deployment.clone()
        };
        assert!(
            expected == actual,
            "expected:\n{expected:?}\n actual:\n{actual:?}\n",
        );
    }

    #[test]
    fn remove_error_status_none() {
        let deployments = def_deps_w_error_status(DeploymentErrorStatus::None);
        for deployment in deployments {
            validate_remove_transition(deployment, DeploymentErrorStatus::None);
        }
    }

    #[test]
    fn remove_error_status_retrying() {
        let deployments = def_deps_w_error_status(DeploymentErrorStatus::Retrying);
        for deployment in deployments {
            match deployment.target_status {
                // Staged and Archived: transitioning to Archived counts as recovery
                DeploymentTargetStatus::Staged | DeploymentTargetStatus::Archived => {
                    validate_remove_transition(deployment, DeploymentErrorStatus::None)
                }
                // Deployed: target not reached, error preserved
                DeploymentTargetStatus::Deployed => {
                    validate_remove_transition(deployment, DeploymentErrorStatus::Retrying)
                }
            }
        }
    }

    #[test]
    fn remove_error_status_failed() {
        let deployments = def_deps_w_error_status(DeploymentErrorStatus::Failed);
        for deployment in deployments {
            validate_remove_transition(deployment, DeploymentErrorStatus::Failed);
        }
    }

    fn validate_error_transition(
        deployment: Deployment,
        settings: &fsm::Settings,
        e: &impl MiruError,
        increment_attempts: bool,
    ) {
        let attempts = if increment_attempts && !e.is_network_connection_error() {
            deployment.attempts + 1
        } else {
            deployment.attempts
        };
        let expected_err_status = if attempts >= settings.max_attempts
            || deployment.error_status == DeploymentErrorStatus::Failed
        {
            DeploymentErrorStatus::Failed
        } else {
            DeploymentErrorStatus::Retrying
        };
        let actual = fsm::error(deployment.clone(), settings, e, increment_attempts);

        let expected = Deployment {
            error_status: expected_err_status,
            attempts,
            cooldown_ends_at: actual.cooldown_ends_at,
            ..deployment.clone()
        };
        assert!(
            expected == actual,
            "expected:\n{expected:?}\n actual:\n{actual:?}\n",
        );

        // check the cooldown
        let now = Utc::now();
        let cooldown = calc_exp_backoff(
            settings.exp_backoff_base_secs,
            2,
            attempts,
            settings.max_cooldown_secs,
        );
        let expected_cooldown_ends_at = now + TimeDelta::seconds(cooldown);
        assert!(
            actual.cooldown_ends_at <= Some(expected_cooldown_ends_at),
            "actual:\n{:?}\n expected:\n{:?}\n",
            actual.cooldown_ends_at,
            expected_cooldown_ends_at
        );
        assert!(
            actual.cooldown_ends_at >= Some(expected_cooldown_ends_at - TimeDelta::seconds(1)),
            "actual:\n{:?}\n expected:\n{:?}\n",
            actual.cooldown_ends_at,
            expected_cooldown_ends_at
        );
    }

    #[test]
    fn error_transition() {
        let settings = fsm::Settings {
            max_attempts: 5,
            exp_backoff_base_secs: 1,
            max_cooldown_secs: 60,
        };

        for i in 0..4 {
            let network_err = i % 2 == 0;
            let increment_attempts = i < 3;

            // no failed attempts
            let deployments = def_deps_w_all_status_combos();
            for mut deployment in deployments {
                deployment.attempts = 0;
                validate_error_transition(
                    deployment,
                    &settings,
                    &MockMiruError::new(network_err),
                    increment_attempts,
                );
            }

            // failed attempts not reached max
            let deployments = def_deps_w_all_status_combos();
            for mut deployment in deployments {
                deployment.attempts = settings.max_attempts - 2;
                validate_error_transition(
                    deployment,
                    &settings,
                    &MockMiruError::new(network_err),
                    increment_attempts,
                );
            }

            // failed attempts reached max
            let deployments = def_deps_w_all_status_combos();
            for mut deployment in deployments {
                deployment.attempts = settings.max_attempts - 1;
                validate_error_transition(
                    deployment,
                    &settings,
                    &MockMiruError::new(network_err),
                    increment_attempts,
                );
            }

            // failed attempts exceeding max
            let deployments = def_deps_w_all_status_combos();
            for mut deployment in deployments {
                deployment.attempts = settings.max_attempts + 1;
                validate_error_transition(
                    deployment,
                    &settings,
                    &MockMiruError::new(network_err),
                    increment_attempts,
                );
            }
        }
    }
}
