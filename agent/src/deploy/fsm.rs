// internal crates
use crate::cooldown;
use crate::errors::Error;
use crate::models;
use crate::models::deployment::Updates;
use crate::models::Patch;

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

pub fn next_action(deployment: &models::Deployment) -> NextAction {
    // do nothing if the status is failed
    if deployment.error_status == models::DplErrStatus::Failed {
        return NextAction::None;
    }

    // check for cooldown
    if deployment.is_in_cooldown() {
        return NextAction::Wait(
            deployment
                .cooldown_ends_at
                .signed_duration_since(Utc::now()),
        );
    }

    // determine the next action
    match deployment.target_status {
        models::DplTarget::Staged => match deployment.activity_status {
            models::DplActivity::Drifted => NextAction::None,
            models::DplActivity::Staged => NextAction::None,
            models::DplActivity::Queued => NextAction::Archive,
            models::DplActivity::Deployed => NextAction::Remove,
            models::DplActivity::Archived => NextAction::None,
        },
        models::DplTarget::Deployed => match deployment.activity_status {
            models::DplActivity::Drifted => NextAction::None,
            models::DplActivity::Staged => NextAction::None,
            models::DplActivity::Queued => NextAction::Deploy,
            models::DplActivity::Deployed => NextAction::None,
            models::DplActivity::Archived => NextAction::Deploy,
        },
        models::DplTarget::Archived => match deployment.activity_status {
            models::DplActivity::Drifted => NextAction::Archive,
            models::DplActivity::Staged => NextAction::Archive,
            models::DplActivity::Queued => NextAction::Archive,
            models::DplActivity::Deployed => NextAction::Remove,
            models::DplActivity::Archived => NextAction::None,
        },
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub backoff: cooldown::Backoff,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 2147483647, // a VERY large number
            backoff: cooldown::Backoff {
                base_secs: 15,
                growth_factor: 2,
                max_secs: 86400, // 24 hours
            },
        }
    }
}

// ================================== TRANSITIONS ================================== //

// ---------------------------- successful transitions ----------------------------= //
pub fn deploy(mut deployment: models::Deployment) -> models::Deployment {
    let new_activity = models::DplActivity::Deployed;
    let patch = get_success_updates(&deployment, new_activity);
    deployment.patch(patch);
    deployment
}

pub fn remove(mut deployment: models::Deployment) -> models::Deployment {
    let new_activity = models::DplActivity::Archived;
    let patch = get_success_updates(&deployment, new_activity);
    deployment.patch(patch);
    deployment
}

fn get_success_updates(
    deployment: &models::Deployment,
    new_activity: models::DplActivity,
) -> Updates {
    Updates {
        activity_status: Some(new_activity),
        error_status: if has_recovered(deployment, new_activity) {
            Some(models::DplErrStatus::None)
        } else {
            None
        },
        attempts: if has_recovered(deployment, new_activity) {
            Some(0)
        } else {
            None
        },
        cooldown: if has_recovered(deployment, new_activity) {
            Some(TimeDelta::zero())
        } else {
            None
        },
    }
}

fn has_recovered(deployment: &models::Deployment, new_activity: models::DplActivity) -> bool {
    // the error status only needs to be updated if it is currently retrying. If is
    // failed then it can never exit failed and if it is None then it is already correct
    if deployment.error_status != models::DplErrStatus::Retrying {
        return false;
    }

    // check if the new activity status matches the deployment's target status
    match deployment.target_status {
        models::DplTarget::Staged => {
            // for staged, we're satisfied with the deployment being in other states as
            // long as it is not deployed.
            match new_activity {
                models::DplActivity::Drifted => true,
                models::DplActivity::Staged => true,
                models::DplActivity::Queued => true,
                models::DplActivity::Deployed => false,
                models::DplActivity::Archived => true,
            }
        }
        models::DplTarget::Deployed => match new_activity {
            models::DplActivity::Drifted => false,
            models::DplActivity::Staged => false,
            models::DplActivity::Queued => false,
            models::DplActivity::Deployed => true,
            models::DplActivity::Archived => false,
        },
        models::DplTarget::Archived => match new_activity {
            models::DplActivity::Drifted => false,
            models::DplActivity::Staged => false,
            models::DplActivity::Queued => false,
            models::DplActivity::Deployed => false,
            models::DplActivity::Archived => true,
        },
    }
}

// ----------------------------- error transitions --------------------------------- //
pub fn error(
    mut deployment: models::Deployment,
    retry_policy: &RetryPolicy,
    e: &impl Error,
    bump_attempts: bool,
) -> models::Deployment {
    let patch = get_error_updates(
        &deployment,
        bump_attempts && should_bump_attempts(e),
        retry_policy,
    );
    deployment.patch(patch);
    deployment
}

fn should_bump_attempts(e: &impl Error) -> bool {
    !e.is_network_conn_err()
}

fn get_error_updates(
    deployment: &models::Deployment,
    bump_attempts: bool,
    retry_policy: &RetryPolicy,
) -> Updates {
    let attempts = if bump_attempts {
        deployment.attempts.saturating_add(1)
    } else {
        deployment.attempts
    };

    let mut new_error_status = Some(models::DplErrStatus::Retrying);
    if attempts >= retry_policy.max_attempts
        || deployment.error_status == models::DplErrStatus::Failed
    {
        new_error_status = Some(models::DplErrStatus::Failed);
    }

    let cooldown = cooldown::calc(&retry_policy.backoff, attempts);

    Updates {
        activity_status: None,
        error_status: new_error_status,
        attempts: Some(attempts),
        cooldown: Some(TimeDelta::seconds(cooldown)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cooldown;
    use models::{Deployment, DplActivity, DplErrStatus, DplTarget};

    // ================================ MOCK ERROR ================================= //

    #[derive(Debug, thiserror::Error)]
    #[error("MockError")]
    struct MockError {
        network_err: bool,
    }

    impl MockError {
        fn new(network_err: bool) -> Self {
            Self { network_err }
        }
    }

    impl Error for MockError {
        fn is_network_conn_err(&self) -> bool {
            self.network_err
        }
    }

    // =============================== NEXT ACTION ================================= //

    mod next_action_fn {
        use super::*;

        const ACTIONABLE_ERROR_STATUSES: [DplErrStatus; 2] =
            [DplErrStatus::None, DplErrStatus::Retrying];

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

        fn validate_next_action(expected: NextAction, actual: NextAction) {
            // if the expected action is not a wait, then the actual action should be
            // the same
            let expected_wait_time = match expected {
                NextAction::Wait(expected_wait_time) => expected_wait_time,
                _ => {
                    assert_eq!(expected, actual);
                    return;
                }
            };

            // if the actual action is not a wait, then the expected action should be
            // the same
            let actual_wait_time = match actual {
                NextAction::Wait(actual_wait_time) => actual_wait_time,
                _ => {
                    assert_eq!(expected, actual);
                    return;
                }
            };

            // both actions are waits, so validate the wait times
            validate_eq_wait_time(
                expected_wait_time,
                actual_wait_time,
                TimeDelta::milliseconds(1),
            );
        }

        struct Expected {
            staged: NextAction,
            deployed: NextAction,
            archived: NextAction,
        }

        fn validate_next_actions(mut deployment: Deployment, expected: &Expected) {
            deployment.target_status = DplTarget::Staged;
            validate_next_action(expected.staged, next_action(&deployment));
            deployment.target_status = DplTarget::Deployed;
            validate_next_action(expected.deployed, next_action(&deployment));
            deployment.target_status = DplTarget::Archived;
            validate_next_action(expected.archived, next_action(&deployment));
        }

        // From the FSM table:
        //
        //  target\activity | Drifted | Staged | Queued  | Deployed | Archived
        //  ----------------+---------+--------+---------+----------+---------
        //  Staged          | None    | None   | Archive | Remove   | None
        //  Deployed        | None    | None   | Deploy  | None     | Deploy
        //  Archived        | Archive | Archive| Archive | Remove   | None

        fn validate_for_activity(activity: DplActivity, actionable: Expected) {
            let mut deployment = Deployment {
                activity_status: activity,
                ..Default::default()
            };

            // actionable error statuses (None, Retrying) without cooldown
            for error_status in ACTIONABLE_ERROR_STATUSES {
                deployment.error_status = error_status;
                validate_next_actions(deployment.clone(), &actionable);
            }

            // actionable error statuses with cooldown → always Wait
            let cooldown = TimeDelta::minutes(60);
            let wait_all = Expected {
                staged: NextAction::Wait(cooldown),
                deployed: NextAction::Wait(cooldown),
                archived: NextAction::Wait(cooldown),
            };
            for error_status in ACTIONABLE_ERROR_STATUSES {
                deployment.error_status = error_status;
                deployment.set_cooldown(cooldown);
                validate_next_actions(deployment.clone(), &wait_all);
            }

            // failed error status → always None
            let none_all = Expected {
                staged: NextAction::None,
                deployed: NextAction::None,
                archived: NextAction::None,
            };
            deployment.error_status = DplErrStatus::Failed;
            validate_next_actions(deployment.clone(), &none_all);
        }

        #[test]
        fn drifted_activity() {
            validate_for_activity(
                DplActivity::Drifted,
                Expected {
                    staged: NextAction::None,
                    deployed: NextAction::None,
                    archived: NextAction::Archive,
                },
            );
        }

        #[test]
        fn staged_activity() {
            validate_for_activity(
                DplActivity::Staged,
                Expected {
                    staged: NextAction::None,
                    deployed: NextAction::None,
                    archived: NextAction::Archive,
                },
            );
        }

        #[test]
        fn queued_activity() {
            validate_for_activity(
                DplActivity::Queued,
                Expected {
                    staged: NextAction::Archive,
                    deployed: NextAction::Deploy,
                    archived: NextAction::Archive,
                },
            );
        }

        #[test]
        fn deployed_activity() {
            validate_for_activity(
                DplActivity::Deployed,
                Expected {
                    staged: NextAction::Remove,
                    deployed: NextAction::None,
                    archived: NextAction::Remove,
                },
            );
        }

        #[test]
        fn archived_activity() {
            validate_for_activity(
                DplActivity::Archived,
                Expected {
                    staged: NextAction::None,
                    deployed: NextAction::Deploy,
                    archived: NextAction::None,
                },
            );
        }
    }

    // =============================== TRANSITIONS ================================= //

    fn all_status_combos() -> Vec<Deployment> {
        let mut deployments = Vec::new();
        for activity in DplActivity::variants() {
            for target in DplTarget::variants() {
                for err in DplErrStatus::variants() {
                    deployments.push(Deployment {
                        activity_status: activity,
                        target_status: target,
                        error_status: err,
                        ..Default::default()
                    });
                }
            }
        }
        deployments
    }

    fn with_error_status(error_status: DplErrStatus) -> Vec<Deployment> {
        let mut deployments = Vec::new();
        for activity in DplActivity::variants() {
            for target in DplTarget::variants() {
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

    mod successful_transitions {
        use super::*;

        fn validate_deploy_transition(deployment: Deployment, expected_error_status: DplErrStatus) {
            let actual = deploy(deployment.clone());

            let recovered = deployment.error_status == DplErrStatus::Retrying
                && expected_error_status == DplErrStatus::None;

            // verify cooldown behavior
            if recovered {
                assert!(
                    !actual.is_in_cooldown(),
                    "cooldown should be cleared on recovery"
                );
            } else {
                assert_eq!(
                    actual.cooldown_ends_at, deployment.cooldown_ends_at,
                    "cooldown should be unchanged when not recovering"
                );
            }

            let expected = Deployment {
                activity_status: DplActivity::Deployed,
                error_status: expected_error_status,
                attempts: if recovered { 0 } else { deployment.attempts },
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
            for deployment in with_error_status(DplErrStatus::None) {
                validate_deploy_transition(deployment, DplErrStatus::None);
            }
        }

        #[test]
        fn deploy_error_status_retrying() {
            for deployment in with_error_status(DplErrStatus::Retrying) {
                match deployment.target_status {
                    DplTarget::Deployed => {
                        validate_deploy_transition(deployment, DplErrStatus::None)
                    }
                    _ => validate_deploy_transition(deployment, DplErrStatus::Retrying),
                }
            }
        }

        #[test]
        fn deploy_error_status_failed() {
            for deployment in with_error_status(DplErrStatus::Failed) {
                validate_deploy_transition(deployment, DplErrStatus::Failed);
            }
        }

        fn validate_remove_transition(deployment: Deployment, expected_error_status: DplErrStatus) {
            let actual = remove(deployment.clone());

            let recovered = deployment.error_status == DplErrStatus::Retrying
                && expected_error_status == DplErrStatus::None;

            // verify cooldown behavior
            if recovered {
                assert!(
                    !actual.is_in_cooldown(),
                    "cooldown should be cleared on recovery"
                );
            } else {
                assert_eq!(
                    actual.cooldown_ends_at, deployment.cooldown_ends_at,
                    "cooldown should be unchanged when not recovering"
                );
            }

            let expected = Deployment {
                activity_status: DplActivity::Archived,
                error_status: expected_error_status,
                attempts: if recovered { 0 } else { deployment.attempts },
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
            for deployment in with_error_status(DplErrStatus::None) {
                validate_remove_transition(deployment, DplErrStatus::None);
            }
        }

        #[test]
        fn remove_error_status_retrying() {
            for deployment in with_error_status(DplErrStatus::Retrying) {
                match deployment.target_status {
                    DplTarget::Staged | DplTarget::Archived => {
                        validate_remove_transition(deployment, DplErrStatus::None)
                    }
                    DplTarget::Deployed => {
                        validate_remove_transition(deployment, DplErrStatus::Retrying)
                    }
                }
            }
        }

        #[test]
        fn remove_error_status_failed() {
            for deployment in with_error_status(DplErrStatus::Failed) {
                validate_remove_transition(deployment, DplErrStatus::Failed);
            }
        }

        // --- non-zero attempts: verify attempts are preserved/reset correctly ---

        fn with_attempts(deployments: Vec<Deployment>, attempts: u32) -> Vec<Deployment> {
            deployments
                .into_iter()
                .map(|mut d| {
                    d.attempts = attempts;
                    d
                })
                .collect()
        }

        fn with_cooldown(deployments: Vec<Deployment>, cooldown: TimeDelta) -> Vec<Deployment> {
            deployments
                .into_iter()
                .map(|mut d| {
                    d.set_cooldown(cooldown);
                    d
                })
                .collect()
        }

        #[test]
        fn deploy_preserves_attempts_when_not_recovering() {
            for deployment in with_attempts(with_error_status(DplErrStatus::None), 5) {
                validate_deploy_transition(deployment, DplErrStatus::None);
            }
            for deployment in with_attempts(with_error_status(DplErrStatus::Failed), 5) {
                validate_deploy_transition(deployment, DplErrStatus::Failed);
            }
        }

        #[test]
        fn deploy_resets_attempts_on_recovery() {
            for deployment in with_attempts(with_error_status(DplErrStatus::Retrying), 5) {
                match deployment.target_status {
                    DplTarget::Deployed => {
                        validate_deploy_transition(deployment, DplErrStatus::None)
                    }
                    _ => validate_deploy_transition(deployment, DplErrStatus::Retrying),
                }
            }
        }

        #[test]
        fn deploy_clears_cooldown_on_recovery() {
            let deployments = with_cooldown(
                with_attempts(with_error_status(DplErrStatus::Retrying), 3),
                TimeDelta::minutes(5),
            );
            for deployment in deployments {
                match deployment.target_status {
                    DplTarget::Deployed => {
                        validate_deploy_transition(deployment, DplErrStatus::None)
                    }
                    _ => validate_deploy_transition(deployment, DplErrStatus::Retrying),
                }
            }
        }

        #[test]
        fn remove_preserves_attempts_when_not_recovering() {
            for deployment in with_attempts(with_error_status(DplErrStatus::None), 5) {
                validate_remove_transition(deployment, DplErrStatus::None);
            }
            for deployment in with_attempts(with_error_status(DplErrStatus::Failed), 5) {
                validate_remove_transition(deployment, DplErrStatus::Failed);
            }
        }

        #[test]
        fn remove_resets_attempts_on_recovery() {
            for deployment in with_attempts(with_error_status(DplErrStatus::Retrying), 5) {
                match deployment.target_status {
                    DplTarget::Staged | DplTarget::Archived => {
                        validate_remove_transition(deployment, DplErrStatus::None)
                    }
                    DplTarget::Deployed => {
                        validate_remove_transition(deployment, DplErrStatus::Retrying)
                    }
                }
            }
        }

        #[test]
        fn remove_clears_cooldown_on_recovery() {
            let deployments = with_cooldown(
                with_attempts(with_error_status(DplErrStatus::Retrying), 3),
                TimeDelta::minutes(5),
            );
            for deployment in deployments {
                match deployment.target_status {
                    DplTarget::Staged | DplTarget::Archived => {
                        validate_remove_transition(deployment, DplErrStatus::None)
                    }
                    DplTarget::Deployed => {
                        validate_remove_transition(deployment, DplErrStatus::Retrying)
                    }
                }
            }
        }
    }

    mod error_transitions {
        use super::*;

        fn validate_error_transition(
            deployment: Deployment,
            retry_policy: &RetryPolicy,
            e: &impl Error,
            bump_attempts: bool,
        ) {
            let attempts = if bump_attempts && !e.is_network_conn_err() {
                deployment.attempts + 1
            } else {
                deployment.attempts
            };
            let expected_err_status = if attempts >= retry_policy.max_attempts
                || deployment.error_status == DplErrStatus::Failed
            {
                DplErrStatus::Failed
            } else {
                DplErrStatus::Retrying
            };
            let actual = error(deployment.clone(), retry_policy, e, bump_attempts);

            // check the cooldown
            let now = Utc::now();
            let cd = cooldown::calc(&retry_policy.backoff, attempts);
            let expected_cooldown_ends_at = now + TimeDelta::seconds(cd);
            assert!(
                actual.cooldown_ends_at <= expected_cooldown_ends_at,
                "actual:\n{:?}\n expected:\n{:?}\n",
                actual.cooldown_ends_at,
                expected_cooldown_ends_at
            );
            assert!(
                actual.cooldown_ends_at >= expected_cooldown_ends_at - TimeDelta::seconds(1),
                "actual:\n{:?}\n expected:\n{:?}\n",
                actual.cooldown_ends_at,
                expected_cooldown_ends_at
            );

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
        }

        #[test]
        fn error_transition() {
            let retry_policy = RetryPolicy {
                max_attempts: 5,
                backoff: cooldown::Backoff {
                    base_secs: 1,
                    growth_factor: 2,
                    max_secs: 60,
                },
            };

            for network_err in [false, true] {
                for bump_attempts in [false, true] {
                    // no failed attempts
                    for mut deployment in all_status_combos() {
                        deployment.attempts = 0;
                        validate_error_transition(
                            deployment,
                            &retry_policy,
                            &MockError::new(network_err),
                            bump_attempts,
                        );
                    }

                    // failed attempts not reached max
                    for mut deployment in all_status_combos() {
                        deployment.attempts = retry_policy.max_attempts - 2;
                        validate_error_transition(
                            deployment,
                            &retry_policy,
                            &MockError::new(network_err),
                            bump_attempts,
                        );
                    }

                    // failed attempts reached max
                    for mut deployment in all_status_combos() {
                        deployment.attempts = retry_policy.max_attempts - 1;
                        validate_error_transition(
                            deployment,
                            &retry_policy,
                            &MockError::new(network_err),
                            bump_attempts,
                        );
                    }

                    // failed attempts exceeding max
                    for mut deployment in all_status_combos() {
                        deployment.attempts = retry_policy.max_attempts + 1;
                        validate_error_transition(
                            deployment,
                            &retry_policy,
                            &MockError::new(network_err),
                            bump_attempts,
                        );
                    }
                }
            }
        }
    }

    // ============================== HAS_RECOVERED ================================ //

    mod has_recovered_fn {
        use super::*;

        // error_status != Retrying → always false (early return)
        #[test]
        fn non_retrying_always_false() {
            for err in [DplErrStatus::None, DplErrStatus::Failed] {
                for target in DplTarget::variants() {
                    for activity in DplActivity::variants() {
                        let d = Deployment {
                            error_status: err,
                            target_status: target,
                            ..Default::default()
                        };
                        assert!(
                            !has_recovered(&d, activity),
                            "expected false for error={err:?}, target={target:?}, activity={activity:?}",
                        );
                    }
                }
            }
        }

        //  When error=Retrying, recovery depends on (target, new_activity):
        //
        //  target \ activity | Drifted | Staged | Queued | Deployed | Archived
        //  ------------------+---------+--------+--------+----------+---------
        //  Staged            |  true   |  true  |  true  |  false   |  true
        //  Deployed          |  false  |  false |  false |  true    |  false
        //  Archived          |  false  |  false |  false |  false   |  true

        struct Case {
            target: DplTarget,
            new_activity: DplActivity,
            recovered: bool,
        }

        #[test]
        fn retrying_recovery_table() {
            let table = vec![
                // target=Staged: recovered unless new_activity is Deployed
                Case {
                    target: DplTarget::Staged,
                    new_activity: DplActivity::Drifted,
                    recovered: true,
                },
                Case {
                    target: DplTarget::Staged,
                    new_activity: DplActivity::Staged,
                    recovered: true,
                },
                Case {
                    target: DplTarget::Staged,
                    new_activity: DplActivity::Queued,
                    recovered: true,
                },
                Case {
                    target: DplTarget::Staged,
                    new_activity: DplActivity::Deployed,
                    recovered: false,
                },
                Case {
                    target: DplTarget::Staged,
                    new_activity: DplActivity::Archived,
                    recovered: true,
                },
                // target=Deployed: recovered only when new_activity is Deployed
                Case {
                    target: DplTarget::Deployed,
                    new_activity: DplActivity::Drifted,
                    recovered: false,
                },
                Case {
                    target: DplTarget::Deployed,
                    new_activity: DplActivity::Staged,
                    recovered: false,
                },
                Case {
                    target: DplTarget::Deployed,
                    new_activity: DplActivity::Queued,
                    recovered: false,
                },
                Case {
                    target: DplTarget::Deployed,
                    new_activity: DplActivity::Deployed,
                    recovered: true,
                },
                Case {
                    target: DplTarget::Deployed,
                    new_activity: DplActivity::Archived,
                    recovered: false,
                },
                // target=Archived: recovered only when new_activity is Archived
                Case {
                    target: DplTarget::Archived,
                    new_activity: DplActivity::Drifted,
                    recovered: false,
                },
                Case {
                    target: DplTarget::Archived,
                    new_activity: DplActivity::Staged,
                    recovered: false,
                },
                Case {
                    target: DplTarget::Archived,
                    new_activity: DplActivity::Queued,
                    recovered: false,
                },
                Case {
                    target: DplTarget::Archived,
                    new_activity: DplActivity::Deployed,
                    recovered: false,
                },
                Case {
                    target: DplTarget::Archived,
                    new_activity: DplActivity::Archived,
                    recovered: true,
                },
            ];

            for case in table {
                let d = Deployment {
                    error_status: DplErrStatus::Retrying,
                    target_status: case.target,
                    ..Default::default()
                };
                assert_eq!(
                    has_recovered(&d, case.new_activity),
                    case.recovered,
                    "target={:?}, new_activity={:?}: expected {}",
                    case.target,
                    case.new_activity,
                    case.recovered,
                );
            }
        }
    }
}
