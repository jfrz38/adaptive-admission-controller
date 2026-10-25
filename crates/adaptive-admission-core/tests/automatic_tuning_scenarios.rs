use adaptive_admission_core::{
    Action, Config, Controller, EvictionReason, Outcome, PidConfig, Priority, StatisticsConfig,
    Timestamp,
};

fn priority(value: u16) -> Priority {
    Priority::new(value).expect("test priority must be valid")
}

fn pid_without_feedback(interval_ms: u64) -> PidConfig {
    PidConfig {
        proportional_gain: 0.0,
        integral_gain: 0.0,
        derivative_gain: 0.0,
        interval_ms,
        ..PidConfig::default()
    }
}

fn statistics(max_window_ms: u64) -> StatisticsConfig {
    StatisticsConfig {
        min_window_ms: 100,
        max_window_ms,
        min_samples: 1,
        min_percentile_samples: 1,
        max_samples: 10,
        latency_percentile: 90,
    }
}

fn controller(initial_concurrency: usize, max_window_ms: u64) -> Controller {
    Controller::new(Config {
        max_concurrent_requests: initial_concurrency,
        max_queued_requests: 20,
        queue_timeout_ms: 50_000,
        statistics: statistics(max_window_ms),
        pid: pid_without_feedback(500),
        ..Config::default()
    })
    .expect("test configuration must be valid")
}

#[test]
fn low_latency_increases_capacity_and_starts_waiting_work() {
    let mut controller = controller(2, 1_000);
    let measured = controller.submit(priority(0), 0.into()).unwrap();
    let _active = controller.submit(priority(1), 0.into()).unwrap();
    let promoted = controller.submit(priority(2), 0.into()).unwrap();
    let waiting = controller.submit(priority(3), 0.into()).unwrap();
    assert_eq!(
        controller
            .complete(measured.request_id, Outcome::Success, 10.into())
            .unwrap(),
        vec![Action::Start {
            request_id: promoted.request_id,
        }]
    );
    controller.advance_time(100.into()).unwrap();

    assert_eq!(
        controller.advance_time(2_000.into()).unwrap(),
        vec![
            Action::ConcurrencyChanged {
                previous: 2,
                current: 3,
            },
            Action::Start {
                request_id: waiting.request_id,
            },
        ]
    );
}

#[test]
fn one_statistics_snapshot_is_not_applied_twice() {
    let mut controller = controller(1, 1_000);
    let measured = controller.submit(priority(0), 0.into()).unwrap();
    controller
        .complete(measured.request_id, Outcome::Success, 10.into())
        .unwrap();
    controller.advance_time(100.into()).unwrap();

    assert_eq!(
        controller.advance_time(2_000.into()).unwrap(),
        vec![Action::ConcurrencyChanged {
            previous: 1,
            current: 2,
        }]
    );
    assert!(controller.advance_time(4_000.into()).unwrap().is_empty());
}

#[test]
fn degraded_latency_reduces_capacity_without_interrupting_active_work() {
    let mut controller = controller(10, 20_000);
    let mut active = Vec::new();
    for value in 0..10 {
        active.push(controller.submit(priority(value), 0.into()).unwrap());
    }
    let first_waiting = controller.submit(priority(10), 0.into()).unwrap();
    let second_waiting = controller.submit(priority(11), 0.into()).unwrap();

    assert_eq!(
        controller
            .complete(active[0].request_id, Outcome::Success, 10_000.into())
            .unwrap(),
        vec![Action::Start {
            request_id: first_waiting.request_id,
        }]
    );
    assert_eq!(
        controller.advance_time(12_000.into()).unwrap(),
        vec![Action::ConcurrencyChanged {
            previous: 10,
            current: 9,
        }]
    );

    assert!(
        controller
            .complete(active[1].request_id, Outcome::Success, 12_001.into())
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        controller
            .complete(active[2].request_id, Outcome::Success, 12_002.into())
            .unwrap(),
        vec![Action::Start {
            request_id: second_waiting.request_id,
        }]
    );
}

#[test]
fn expiration_wins_when_capacity_grows_at_the_same_timestamp() {
    let mut controller = Controller::new(Config {
        max_concurrent_requests: 2,
        max_queued_requests: 10,
        queue_timeout_ms: 2_000,
        statistics: statistics(1_000),
        pid: pid_without_feedback(500),
        ..Config::default()
    })
    .unwrap();
    let first = controller.submit(priority(0), 0.into()).unwrap();
    let _second = controller.submit(priority(1), 0.into()).unwrap();
    let promoted = controller.submit(priority(2), 0.into()).unwrap();
    let expiring = controller.submit(priority(3), 0.into()).unwrap();
    assert_eq!(
        controller
            .complete(first.request_id, Outcome::Success, 100.into())
            .unwrap(),
        vec![Action::Start {
            request_id: promoted.request_id,
        }]
    );

    assert_eq!(
        controller.advance_time(2_000.into()).unwrap(),
        vec![
            Action::Evict {
                request_id: expiring.request_id,
                reason: EvictionReason::QueueTimeout,
            },
            Action::ConcurrencyChanged {
                previous: 2,
                current: 3,
            },
        ]
    );
}

#[test]
fn delayed_wakeup_runs_one_update_and_reschedules_from_observation_time() {
    let mut controller = Controller::new(Config {
        max_concurrent_requests: 1,
        queue_timeout_ms: 50_000,
        statistics: statistics(30_000),
        pid: pid_without_feedback(5_000),
        ..Config::default()
    })
    .unwrap();
    let _active = controller.submit(priority(0), 0.into()).unwrap();
    assert_eq!(
        controller.next_deadline(),
        Some(Timestamp::from_millis(2_000))
    );

    assert!(controller.advance_time(5_000.into()).unwrap().is_empty());
    assert_eq!(
        controller.next_deadline(),
        Some(Timestamp::from_millis(7_000))
    );
}
