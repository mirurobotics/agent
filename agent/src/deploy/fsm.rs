// standard crates
use std::time::Duration;

// internal crates
use crate::cooldown;
use crate::errors::Error;
use crate::models::{self, deployment::Updates, Patch};

// external crates
use chrono::Utc;

// ================================ NEXT ACTION ==================================== //
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NextAction {
    None,
    Deploy,
    Remove,
    Archive,
    Wait(Duration),
}

/// What to do next with `deployment`. `cooldown` is how much of its retry
/// cooldown is left (see [`crate::deploy::Cooldowns`]), or `None` if it is
/// not cooling down.
pub fn next_action(deployment: &models::Deployment, cooldown: Option<Duration>) -> NextAction {
    // do nothing if the status is failed
    if deployment.error_status == models::DplErrStatus::Failed {
        return NextAction::None;
    }

    // check for cooldown
    if let Some(wait) = cooldown.filter(|wait| !wait.is_zero()) {
        return NextAction::Wait(wait);
    }

    // determine the next action
    match deployment.target_status {
        models::DplTarget::Staged => match deployment.activity_status {
            models::DplActivity::Drifted => NextAction::None,
            models::DplActivity::Staged => NextAction::None,
            models::DplActivity::Queued => NextAction::Archive,
            models::DplActivity::Deployed => NextAction::Remove,
            models::DplActivity::Removing => NextAction::Remove,
            models::DplActivity::Archived => NextAction::None,
        },
        models::DplTarget::Deployed => match deployment.activity_status {
            models::DplActivity::Drifted => NextAction::None,
            models::DplActivity::Staged => NextAction::None,
            models::DplActivity::Queued => NextAction::Deploy,
            models::DplActivity::Deployed => NextAction::None,
            models::DplActivity::Removing => NextAction::Deploy,
            models::DplActivity::Archived => NextAction::Deploy,
        },
        models::DplTarget::Archived => match deployment.activity_status {
            models::DplActivity::Drifted => NextAction::Archive,
            models::DplActivity::Staged => NextAction::Archive,
            models::DplActivity::Queued => NextAction::Archive,
            models::DplActivity::Deployed => NextAction::Remove,
            models::DplActivity::Removing => NextAction::Remove,
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

pub fn removing(mut deployment: models::Deployment) -> models::Deployment {
    let new_activity = models::DplActivity::Removing;
    let patch = get_success_updates(&deployment, new_activity);
    deployment.patch(patch);
    deployment
}

pub fn archive(mut deployment: models::Deployment) -> models::Deployment {
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
        deployed_at: if new_activity == models::DplActivity::Deployed {
            Some(Utc::now())
        } else {
            None
        },
        archived_at: if new_activity == models::DplActivity::Archived {
            Some(Utc::now())
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
                models::DplActivity::Removing => false,
                models::DplActivity::Archived => true,
            }
        }
        models::DplTarget::Deployed => match new_activity {
            models::DplActivity::Drifted => false,
            models::DplActivity::Staged => false,
            models::DplActivity::Queued => false,
            models::DplActivity::Deployed => true,
            models::DplActivity::Removing => false,
            models::DplActivity::Archived => false,
        },
        models::DplTarget::Archived => match new_activity {
            models::DplActivity::Drifted => false,
            models::DplActivity::Staged => false,
            models::DplActivity::Queued => false,
            models::DplActivity::Deployed => false,
            models::DplActivity::Removing => false,
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

/// How long to wait before retrying `deployment` after the failure that
/// [`error`] just recorded: the backoff for its attempt count.
pub fn cooldown(deployment: &models::Deployment, retry_policy: &RetryPolicy) -> Duration {
    let secs = cooldown::calc(&retry_policy.backoff, deployment.attempts);
    Duration::from_secs(u64::try_from(secs).unwrap_or(0))
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

    Updates {
        activity_status: None,
        error_status: new_error_status,
        attempts: Some(attempts),
        deployed_at: None,
        archived_at: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cooldown;
    use chrono::TimeDelta;
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

        struct Expected {
            staged: NextAction,
            deployed: NextAction,
            archived: NextAction,
        }

        fn validate_next_actions(
            mut deployment: Deployment,
            cooldown: Option<Duration>,
            expected: &Expected,
        ) {
            deployment.target_status = DplTarget::Staged;
            assert_eq!(expected.staged, next_action(&deployment, cooldown));
            deployment.target_status = DplTarget::Deployed;
            assert_eq!(expected.deployed, next_action(&deployment, cooldown));
            deployment.target_status = DplTarget::Archived;
            assert_eq!(expected.archived, next_action(&deployment, cooldown));
        }

        // From the FSM table:
        //
        //  target\activity | Drifted | Staged | Queued  | Deployed | Removing | Archived
        //  ----------------+---------+--------+---------+----------+----------+---------
        //  Staged          | None    | None   | Archive | Remove   | Remove | None
        //  Deployed        | None    | None   | Deploy  | None     | Deploy | Deploy
        //  Archived        | Archive | Archive| Archive | Remove   | Remove | None

        fn validate_for_activity(activity: DplActivity, actionable: Expected) {
            let mut deployment = Deployment {
                activity_status: activity,
                ..Default::default()
            };

            // actionable error statuses (None, Retrying) without cooldown; a
            // zero cooldown counts as none
            for error_status in ACTIONABLE_ERROR_STATUSES {
                deployment.error_status = error_status;
                validate_next_actions(deployment.clone(), None, &actionable);
                validate_next_actions(deployment.clone(), Some(Duration::ZERO), &actionable);
            }

            // actionable error statuses with cooldown → always Wait
            let cooldown = Duration::from_secs(60 * 60);
            let wait_all = Expected {
                staged: NextAction::Wait(cooldown),
                deployed: NextAction::Wait(cooldown),
                archived: NextAction::Wait(cooldown),
            };
            for error_status in ACTIONABLE_ERROR_STATUSES {
                deployment.error_status = error_status;
                validate_next_actions(deployment.clone(), Some(cooldown), &wait_all);
            }

            // failed error status → always None, cooldown or not
            let none_all = Expected {
                staged: NextAction::None,
                deployed: NextAction::None,
                archived: NextAction::None,
            };
            deployment.error_status = DplErrStatus::Failed;
            validate_next_actions(deployment.clone(), None, &none_all);
            validate_next_actions(deployment.clone(), Some(cooldown), &none_all);
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
        fn removing_activity() {
            validate_for_activity(
                DplActivity::Removing,
                Expected {
                    staged: NextAction::Remove,
                    deployed: NextAction::Deploy,
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

        // -- helpers

        fn with_attempts(deployments: Vec<Deployment>, attempts: u32) -> Vec<Deployment> {
            deployments
                .into_iter()
                .map(|mut d| {
                    d.attempts = attempts;
                    d
                })
                .collect()
        }

        // --- deploy transition ---

        fn validate_deploy_transition(deployment: Deployment, expected_error_status: DplErrStatus) {
            let before = Utc::now();
            let actual = deploy(deployment.clone());
            let after = Utc::now();

            let recovered = deployment.error_status == DplErrStatus::Retrying
                && expected_error_status == DplErrStatus::None;

            // verify timestamps: deployed_at is always freshly set; archived_at is
            // an independent watermark preserved from the input (never cleared).
            let deployed_at = actual.deployed_at.expect("deployed_at should be set");
            assert!(
                deployed_at >= before && deployed_at <= after,
                "deployed_at should be approximately now",
            );
            assert_eq!(
                actual.archived_at, deployment.archived_at,
                "archived_at should be preserved from input",
            );

            let expected = Deployment {
                activity_status: DplActivity::Deployed,
                error_status: expected_error_status,
                attempts: if recovered { 0 } else { deployment.attempts },
                deployed_at: actual.deployed_at,
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
        fn deploy_preserves_pre_existing_archived_at() {
            let past = Utc::now() - TimeDelta::hours(1);
            for mut deployment in with_error_status(DplErrStatus::None) {
                deployment.archived_at = Some(past);
                validate_deploy_transition(deployment, DplErrStatus::None);
            }
        }

        // --- remove transition ---

        fn validate_remove_transition(deployment: Deployment, expected_error_status: DplErrStatus) {
            let before = Utc::now();
            let actual = archive(deployment.clone());
            let after = Utc::now();

            let recovered = deployment.error_status == DplErrStatus::Retrying
                && expected_error_status == DplErrStatus::None;

            // verify timestamps: archived_at is always freshly set; deployed_at is
            // an independent watermark preserved from the input (never cleared).
            let archived_at = actual.archived_at.expect("archived_at should be set");
            assert!(
                archived_at >= before && archived_at <= after,
                "archived_at should be approximately now",
            );
            assert_eq!(
                actual.deployed_at, deployment.deployed_at,
                "deployed_at should be preserved from input",
            );

            let expected = Deployment {
                activity_status: DplActivity::Archived,
                error_status: expected_error_status,
                attempts: if recovered { 0 } else { deployment.attempts },
                archived_at: actual.archived_at,
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
        fn remove_preserves_pre_existing_deployed_at() {
            let past = Utc::now() - TimeDelta::hours(1);
            for mut deployment in with_error_status(DplErrStatus::None) {
                deployment.deployed_at = Some(past);
                validate_remove_transition(deployment, DplErrStatus::None);
            }
        }

        // --- removing transition (intermediate state) ---

        fn validate_removing_transition(deployment: Deployment) {
            let actual = removing(deployment.clone());

            // removing is intermediate — no timestamps should change
            assert_eq!(
                actual.deployed_at, deployment.deployed_at,
                "deployed_at should be preserved",
            );
            assert_eq!(
                actual.archived_at, deployment.archived_at,
                "archived_at should be preserved",
            );

            // removing never counts as recovery, so error_status/attempts are preserved

            let expected = Deployment {
                activity_status: DplActivity::Removing,
                ..deployment.clone()
            };
            assert!(
                expected == actual,
                "expected:\n{expected:?}\n actual:\n{actual:?}\n",
            );
        }

        #[test]
        fn removing_error_status_none() {
            for deployment in with_error_status(DplErrStatus::None) {
                validate_removing_transition(deployment);
            }
        }

        #[test]
        fn removing_error_status_retrying() {
            for deployment in with_error_status(DplErrStatus::Retrying) {
                validate_removing_transition(deployment);
            }
        }

        #[test]
        fn removing_error_status_failed() {
            for deployment in with_error_status(DplErrStatus::Failed) {
                validate_removing_transition(deployment);
            }
        }

        #[test]
        fn removing_preserves_attempts() {
            for deployment in with_attempts(with_error_status(DplErrStatus::None), 5) {
                validate_removing_transition(deployment);
            }
            for deployment in with_attempts(with_error_status(DplErrStatus::Retrying), 5) {
                validate_removing_transition(deployment);
            }
        }

        #[test]
        fn removing_preserves_pre_existing_timestamps() {
            let past = Utc::now() - TimeDelta::hours(1);
            for mut deployment in with_error_status(DplErrStatus::None) {
                deployment.deployed_at = Some(past);
                deployment.archived_at = Some(past);
                validate_removing_transition(deployment);
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

            // the cooldown is the backoff for the new attempt count
            let cd = cooldown::calc(&retry_policy.backoff, attempts);
            assert_eq!(
                cooldown(&actual, retry_policy),
                Duration::from_secs(cd as u64)
            );

            let expected = Deployment {
                error_status: expected_err_status,
                attempts,
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
        //  target \ activity | Drifted | Staged | Queued | Deployed | Removing | Archived
        //  ------------------+---------+--------+--------+----------+----------+---------
        //  Staged            |  true   |  true  |  true  |  false   |  false   |  true
        //  Deployed          |  false  |  false |  false |  true    |  false   |  false
        //  Archived          |  false  |  false |  false |  false   |  false   |  true

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
                    new_activity: DplActivity::Removing,
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
                    new_activity: DplActivity::Removing,
                    recovered: false,
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
                    new_activity: DplActivity::Removing,
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
