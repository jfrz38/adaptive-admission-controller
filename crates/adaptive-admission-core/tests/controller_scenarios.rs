use adaptive_admission_core::{
    Action, CancellationReason, Config, ConfigError, Controller, ControllerError, EvictionReason,
    Outcome, Priority, PriorityError, RejectionReason, Timestamp,
};

fn priority(value: u16) -> Priority {
    Priority::new(value).expect("test priority must be valid")
}

fn controller(config: Config) -> Controller {
    Controller::new(config).expect("test configuration must be valid")
}

fn constrained_config() -> Config {
    Config {
        max_concurrent_requests: 1,
        max_queued_requests: 10,
        queue_timeout_ms: 100,
        ..Config::default()
    }
}

#[test]
fn starts_immediately_when_capacity_is_available() {
    let mut controller = controller(constrained_config());

    let submission = controller.submit(priority(500), 0.into()).unwrap();

    assert_eq!(
        submission.actions,
        vec![Action::Start {
            request_id: submission.request_id,
        }]
    );
    assert_eq!(
        controller.next_deadline(),
        Some(Timestamp::from_millis(500))
    );
}

#[test]
fn never_starts_more_work_than_the_configured_capacity() {
    let mut controller = controller(Config {
        max_concurrent_requests: 2,
        ..constrained_config()
    });

    let first = controller.submit(priority(0), 0.into()).unwrap();
    let second = controller.submit(priority(1), 0.into()).unwrap();
    let third = controller.submit(priority(2), 0.into()).unwrap();

    assert_eq!(first.actions.len(), 1);
    assert_eq!(second.actions.len(), 1);
    assert!(third.actions.is_empty());

    let actions = controller
        .complete(first.request_id, Outcome::Success, 1.into())
        .unwrap();
    assert_eq!(
        actions,
        vec![Action::Start {
            request_id: third.request_id,
        }]
    );
}

#[test]
fn schedules_lower_numeric_priorities_first() {
    let mut controller = controller(constrained_config());
    let active = controller.submit(priority(0), 0.into()).unwrap();
    let less_important = controller.submit(priority(500), 0.into()).unwrap();
    let more_important = controller.submit(priority(10), 0.into()).unwrap();

    let actions = controller
        .complete(active.request_id, Outcome::Success, 1.into())
        .unwrap();

    assert_eq!(
        actions,
        vec![Action::Start {
            request_id: more_important.request_id,
        }]
    );
    assert!(less_important.actions.is_empty());
}

#[test]
fn preserves_fifo_order_for_equal_priorities() {
    let mut controller = controller(constrained_config());
    let active = controller.submit(priority(0), 0.into()).unwrap();
    let first_waiting = controller.submit(priority(100), 1.into()).unwrap();
    let second_waiting = controller.submit(priority(100), 2.into()).unwrap();

    let actions = controller
        .complete(active.request_id, Outcome::Success, 3.into())
        .unwrap();

    assert_eq!(
        actions,
        vec![Action::Start {
            request_id: first_waiting.request_id,
        }]
    );
    assert!(second_waiting.actions.is_empty());
}

#[test]
fn rejects_a_priority_at_or_above_the_exclusive_threshold() {
    let mut controller = controller(Config {
        initial_threshold: 100,
        ..constrained_config()
    });

    let accepted = controller.submit(priority(99), 0.into()).unwrap();
    let rejected = controller.submit(priority(100), 0.into()).unwrap();

    assert!(matches!(
        accepted.actions.as_slice(),
        [Action::Start { .. }]
    ));
    assert_eq!(
        rejected.actions,
        vec![Action::Reject {
            request_id: rejected.request_id,
            reason: RejectionReason::Threshold {
                priority: priority(100),
                threshold: 100,
            },
        }]
    );
}

#[test]
fn rejects_the_new_request_when_the_queue_is_full() {
    let mut controller = controller(Config {
        max_queued_requests: 1,
        ..constrained_config()
    });
    let _ = controller.submit(priority(0), 0.into()).unwrap();
    let _ = controller.submit(priority(1), 0.into()).unwrap();

    let rejected = controller.submit(priority(2), 0.into()).unwrap();

    assert_eq!(
        rejected.actions,
        vec![Action::Reject {
            request_id: rejected.request_id,
            reason: RejectionReason::QueueFull,
        }]
    );
}

#[test]
fn expires_waiting_work_at_its_deadline_before_reusing_capacity() {
    let mut controller = controller(constrained_config());
    let active = controller.submit(priority(0), 0.into()).unwrap();
    let expiring = controller.submit(priority(1), 0.into()).unwrap();
    let next = controller.submit(priority(2), 50.into()).unwrap();

    assert_eq!(
        controller.next_deadline(),
        Some(Timestamp::from_millis(100))
    );
    let actions = controller
        .complete(active.request_id, Outcome::Success, 100.into())
        .unwrap();

    assert_eq!(
        actions,
        vec![
            Action::Evict {
                request_id: expiring.request_id,
                reason: EvictionReason::QueueTimeout,
            },
            Action::Start {
                request_id: next.request_id,
            },
        ]
    );
    assert_eq!(
        controller.next_deadline(),
        Some(Timestamp::from_millis(500))
    );
}

#[test]
fn advance_time_evicts_without_executing_host_work() {
    let mut controller = controller(constrained_config());
    let _ = controller.submit(priority(0), 0.into()).unwrap();
    let waiting = controller.submit(priority(1), 0.into()).unwrap();

    let actions = controller.advance_time(100.into()).unwrap();

    assert_eq!(
        actions,
        vec![Action::Evict {
            request_id: waiting.request_id,
            reason: EvictionReason::QueueTimeout,
        }]
    );
}

#[test]
fn cancels_waiting_work_but_not_started_work() {
    let mut controller = controller(constrained_config());
    let active = controller.submit(priority(0), 0.into()).unwrap();
    let waiting = controller.submit(priority(1), 0.into()).unwrap();

    let actions = controller.cancel(waiting.request_id, 1.into()).unwrap();

    assert_eq!(
        actions,
        vec![Action::Cancel {
            request_id: waiting.request_id,
            reason: CancellationReason::Requested,
        }]
    );
    assert!(matches!(
        controller.cancel(active.request_id, 1.into()),
        Err(ControllerError::InvalidTransition {
            operation: "cancel",
            state: "started",
            ..
        })
    ));
}

#[test]
fn expiration_wins_over_cancellation_at_the_same_timestamp() {
    let mut controller = controller(constrained_config());
    let _ = controller.submit(priority(0), 0.into()).unwrap();
    let waiting = controller.submit(priority(1), 0.into()).unwrap();

    let actions = controller.cancel(waiting.request_id, 100.into()).unwrap();

    assert_eq!(
        actions,
        vec![Action::Evict {
            request_id: waiting.request_id,
            reason: EvictionReason::QueueTimeout,
        }]
    );
}

#[test]
fn shutdown_cancels_waiting_work_and_allows_started_work_to_complete() {
    let mut controller = controller(constrained_config());
    let active = controller.submit(priority(0), 0.into()).unwrap();
    let waiting = controller.submit(priority(1), 0.into()).unwrap();

    let actions = controller.shutdown(1.into()).unwrap();
    assert_eq!(
        actions,
        vec![Action::Cancel {
            request_id: waiting.request_id,
            reason: CancellationReason::Shutdown,
        }]
    );
    assert!(
        controller
            .complete(active.request_id, Outcome::Success, 2.into())
            .unwrap()
            .is_empty()
    );

    let rejected = controller.submit(priority(0), 2.into()).unwrap();
    assert_eq!(
        rejected.actions,
        vec![Action::Reject {
            request_id: rejected.request_id,
            reason: RejectionReason::Shutdown,
        }]
    );
}

#[test]
fn failed_work_also_releases_capacity() {
    let mut controller = controller(constrained_config());
    let active = controller.submit(priority(0), 0.into()).unwrap();
    let waiting = controller.submit(priority(1), 0.into()).unwrap();

    let actions = controller
        .complete(active.request_id, Outcome::Failure, 1.into())
        .unwrap();

    assert_eq!(
        actions,
        vec![Action::Start {
            request_id: waiting.request_id,
        }]
    );
}

#[test]
fn rejects_regressing_time_without_changing_controller_state() {
    let mut controller = controller(constrained_config());
    let active = controller.submit(priority(0), 10.into()).unwrap();

    assert_eq!(
        controller.advance_time(9.into()),
        Err(ControllerError::TimeRegression {
            previous: 10.into(),
            received: 9.into(),
        })
    );
    assert!(
        controller
            .complete(active.request_id, Outcome::Success, 10.into())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn reports_invalid_and_unknown_request_transitions() {
    let mut controller = controller(constrained_config());
    let active = controller.submit(priority(0), 0.into()).unwrap();
    let waiting = controller.submit(priority(1), 0.into()).unwrap();

    assert!(matches!(
        controller.complete(waiting.request_id, Outcome::Success, 0.into()),
        Err(ControllerError::InvalidTransition {
            operation: "complete",
            state: "queued",
            ..
        })
    ));
    controller
        .complete(active.request_id, Outcome::Success, 1.into())
        .unwrap();
    assert_eq!(
        controller.complete(active.request_id, Outcome::Success, 1.into()),
        Err(ControllerError::UnknownRequest {
            request_id: active.request_id,
        })
    );
}

#[test]
fn validates_configuration_and_priority_inputs() {
    assert!(matches!(
        Controller::new(Config {
            initial_threshold: 769,
            ..Config::default()
        }),
        Err(ConfigError::ThresholdOutOfRange { value: 769 })
    ));
    assert!(matches!(
        Controller::new(Config {
            max_concurrent_requests: 0,
            ..Config::default()
        }),
        Err(ConfigError::ZeroConcurrency)
    ));
    assert!(matches!(
        Controller::new(Config {
            queue_timeout_ms: 0,
            ..Config::default()
        }),
        Err(ConfigError::ZeroQueueTimeout)
    ));
    assert_eq!(Priority::from_tier(2, 42).unwrap().value(), 298);
    assert!(matches!(
        Priority::new(768),
        Err(PriorityError::ValueOutOfRange { value: 768 })
    ));
}

#[test]
fn identical_input_sequences_produce_identical_actions() {
    fn run_trace() -> Vec<Action> {
        let mut controller = controller(constrained_config());
        let active = controller.submit(priority(0), 0.into()).unwrap();
        let first = controller.submit(priority(300), 1.into()).unwrap();
        let second = controller.submit(priority(100), 2.into()).unwrap();

        let mut actions = active.actions;
        actions.extend(first.actions);
        actions.extend(second.actions);
        actions.extend(
            controller
                .complete(active.request_id, Outcome::Success, 3.into())
                .unwrap(),
        );
        actions.extend(controller.shutdown(4.into()).unwrap());
        actions
    }

    assert_eq!(run_trace(), run_trace());
}
