use adaptive_admission_core::{
    Action, Config, ConfigError, Controller, EvictionReason, Outcome, PidConfig, Priority,
    StatisticsConfig,
};

fn priority(value: u16) -> Priority {
    Priority::new(value).expect("test priority must be valid")
}

fn pid_without_feedback() -> PidConfig {
    PidConfig {
        proportional_gain: 0.0,
        integral_gain: 0.0,
        derivative_gain: 0.0,
        ..PidConfig::default()
    }
}

fn statistics(min_samples: usize) -> StatisticsConfig {
    StatisticsConfig {
        min_window_ms: 100,
        max_window_ms: 200,
        min_samples,
        min_percentile_samples: min_samples,
        max_samples: 10,
        latency_percentile: 90,
    }
}

fn controller(initial_timeout_ms: u64, ratio: f64, min_samples: usize) -> Controller {
    Controller::new(Config {
        max_concurrent_requests: 1,
        queue_timeout_ms: initial_timeout_ms,
        queue_timeout_ratio: ratio,
        statistics: statistics(min_samples),
        pid: pid_without_feedback(),
        ..Config::default()
    })
    .expect("test configuration must be valid")
}

#[test]
fn uses_the_initial_timeout_before_statistics_are_published() {
    let mut controller = controller(500, 0.5, 1);
    let _active = controller.submit(priority(0), 0.into()).unwrap();
    let waiting = controller.submit(priority(1), 0.into()).unwrap();

    assert!(controller.advance_time(499.into()).unwrap().is_empty());
    assert_eq!(
        controller.advance_time(500.into()).unwrap(),
        vec![Action::Evict {
            request_id: waiting.request_id,
            reason: EvictionReason::QueueTimeout,
        }]
    );
}

#[test]
fn published_average_latency_is_rounded_for_new_queue_entries() {
    let mut controller = controller(500, 0.5, 1);
    let measured = controller.submit(priority(0), 0.into()).unwrap();
    controller
        .complete(measured.request_id, Outcome::Success, 101.into())
        .unwrap();
    let _active = controller.submit(priority(0), 101.into()).unwrap();
    let waiting = controller.submit(priority(1), 101.into()).unwrap();

    assert!(controller.advance_time(151.into()).unwrap().is_empty());
    assert_eq!(
        controller.advance_time(152.into()).unwrap(),
        vec![Action::Evict {
            request_id: waiting.request_id,
            reason: EvictionReason::QueueTimeout,
        }]
    );
}

#[test]
fn changing_the_timeout_does_not_reschedule_existing_queue_entries() {
    let mut controller = Controller::new(Config {
        max_concurrent_requests: 2,
        queue_timeout_ms: 500,
        queue_timeout_ratio: 0.5,
        statistics: statistics(1),
        pid: pid_without_feedback(),
        ..Config::default()
    })
    .unwrap();
    let measured = controller.submit(priority(0), 0.into()).unwrap();
    controller
        .complete(measured.request_id, Outcome::Success, 10.into())
        .unwrap();
    let _first_active = controller.submit(priority(0), 10.into()).unwrap();
    let _second_active = controller.submit(priority(0), 10.into()).unwrap();
    let existing = controller.submit(priority(1), 20.into()).unwrap();

    controller.advance_time(100.into()).unwrap();
    let new = controller.submit(priority(2), 100.into()).unwrap();

    assert_eq!(
        controller.advance_time(105.into()).unwrap(),
        vec![Action::Evict {
            request_id: new.request_id,
            reason: EvictionReason::QueueTimeout,
        }]
    );
    assert!(controller.advance_time(519.into()).unwrap().is_empty());
    assert_eq!(
        controller.advance_time(520.into()).unwrap(),
        vec![Action::Evict {
            request_id: existing.request_id,
            reason: EvictionReason::QueueTimeout,
        }]
    );
}

#[test]
fn discarded_statistics_do_not_replace_the_initial_timeout() {
    let mut controller = controller(500, 0.5, 2);
    let measured = controller.submit(priority(0), 0.into()).unwrap();
    controller
        .complete(measured.request_id, Outcome::Success, 10.into())
        .unwrap();
    controller.advance_time(200.into()).unwrap();
    let _active = controller.submit(priority(0), 200.into()).unwrap();
    let waiting = controller.submit(priority(1), 200.into()).unwrap();

    assert!(controller.advance_time(699.into()).unwrap().is_empty());
    assert_eq!(
        controller.advance_time(700.into()).unwrap(),
        vec![Action::Evict {
            request_id: waiting.request_id,
            reason: EvictionReason::QueueTimeout,
        }]
    );
}

#[test]
fn a_timeout_that_rounds_to_zero_is_ignored() {
    let mut controller = controller(500, 0.1, 1);
    let measured = controller.submit(priority(0), 0.into()).unwrap();
    controller
        .complete(measured.request_id, Outcome::Success, 1.into())
        .unwrap();
    controller.advance_time(100.into()).unwrap();
    let _active = controller.submit(priority(0), 100.into()).unwrap();
    let waiting = controller.submit(priority(1), 100.into()).unwrap();

    assert!(controller.advance_time(599.into()).unwrap().is_empty());
    assert_eq!(
        controller.advance_time(600.into()).unwrap(),
        vec![Action::Evict {
            request_id: waiting.request_id,
            reason: EvictionReason::QueueTimeout,
        }]
    );
}

#[test]
fn validates_the_adaptive_timeout_ratio() {
    for ratio in [0.0, -0.1, 1.1, f64::INFINITY, f64::NAN] {
        assert!(matches!(
            Controller::new(Config {
                queue_timeout_ratio: ratio,
                ..Config::default()
            }),
            Err(ConfigError::QueueTimeoutRatioOutOfRange)
        ));
    }
}
