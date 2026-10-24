use adaptive_admission_core::{
    Action, Config, ConfigError, Controller, Outcome, PidConfig, Priority, RejectionReason,
    StatisticsConfig, Timestamp,
};

fn priority(value: u16) -> Priority {
    Priority::new(value).expect("test priority must be valid")
}

fn pid_config(proportional_gain: f64) -> PidConfig {
    PidConfig {
        proportional_gain,
        integral_gain: 0.0,
        derivative_gain: 0.0,
        interval_ms: 1_000,
        max_delta_percentage: 100.0,
        integral_decay: 0.5,
    }
}

fn controller(pid: PidConfig, statistics: StatisticsConfig) -> Controller {
    Controller::new(Config {
        max_concurrent_requests: 1,
        queue_timeout_ms: 10_000,
        pid,
        statistics,
        ..Config::default()
    })
    .expect("test configuration must be valid")
}

#[test]
fn queue_pressure_reduces_the_threshold_and_changes_later_admission() {
    let mut controller = controller(pid_config(20.0), StatisticsConfig::default());
    let _active = controller.submit(priority(0), 0.into()).unwrap();
    let _waiting = controller.submit(priority(700), 0.into()).unwrap();

    assert_eq!(
        controller.advance_time(1_000.into()).unwrap(),
        vec![Action::ThresholdChanged {
            previous: 768,
            current: 614,
        }]
    );

    let rejected = controller.submit(priority(614), 1_000.into()).unwrap();
    assert_eq!(
        rejected.actions,
        vec![Action::Reject {
            request_id: rejected.request_id,
            reason: RejectionReason::Threshold {
                priority: priority(614),
                threshold: 614,
            },
        }]
    );
}

#[test]
fn unchanged_effective_threshold_does_not_emit_an_action() {
    let mut controller = controller(pid_config(20.0), StatisticsConfig::default());
    let _ = controller.submit(priority(0), 0.into()).unwrap();

    assert!(controller.advance_time(1_000.into()).unwrap().is_empty());
}

#[test]
fn free_capacity_recovers_the_initial_threshold() {
    let mut controller = controller(pid_config(20.0), StatisticsConfig::default());
    let active = controller.submit(priority(0), 0.into()).unwrap();
    let waiting = controller.submit(priority(700), 0.into()).unwrap();
    controller.advance_time(1_000.into()).unwrap();
    controller
        .complete(active.request_id, Outcome::Success, 1_001.into())
        .unwrap();
    controller
        .complete(waiting.request_id, Outcome::Success, 1_002.into())
        .unwrap();

    assert_eq!(
        controller.advance_time(2_000.into()).unwrap(),
        vec![Action::ThresholdChanged {
            previous: 614,
            current: 768,
        }]
    );
}

#[test]
fn uses_priority_distribution_when_enough_samples_are_available() {
    let statistics = StatisticsConfig {
        min_samples: 3,
        min_percentile_samples: 3,
        max_samples: 10,
        ..StatisticsConfig::default()
    };
    let mut controller = controller(pid_config(25.0), statistics);
    let _ = controller.submit(priority(0), 0.into()).unwrap();
    let _ = controller.submit(priority(100), 0.into()).unwrap();
    let _ = controller.submit(priority(700), 0.into()).unwrap();

    assert_eq!(
        controller.advance_time(1_000.into()).unwrap(),
        vec![Action::ThresholdChanged {
            previous: 768,
            current: 101,
        }]
    );
}

#[test]
fn delayed_wakeup_performs_one_update_and_reschedules_from_observation_time() {
    let mut controller = controller(pid_config(20.0), StatisticsConfig::default());
    let _ = controller.submit(priority(0), 0.into()).unwrap();
    let _ = controller.submit(priority(700), 0.into()).unwrap();

    assert_eq!(
        controller.advance_time(5_000.into()).unwrap(),
        vec![Action::ThresholdChanged {
            previous: 768,
            current: 614,
        }]
    );
    assert_eq!(
        controller.next_deadline(),
        Some(Timestamp::from_millis(6_000))
    );
}

#[test]
fn validates_pid_configuration() {
    assert!(matches!(
        Controller::new(Config {
            pid: PidConfig {
                proportional_gain: f64::NAN,
                ..PidConfig::default()
            },
            ..Config::default()
        }),
        Err(ConfigError::InvalidPidGain {
            name: "proportional"
        })
    ));
    assert!(matches!(
        Controller::new(Config {
            pid: PidConfig {
                interval_ms: 0,
                ..PidConfig::default()
            },
            ..Config::default()
        }),
        Err(ConfigError::ZeroPidInterval)
    ));
    assert!(matches!(
        Controller::new(Config {
            pid: PidConfig {
                max_delta_percentage: 101.0,
                ..PidConfig::default()
            },
            ..Config::default()
        }),
        Err(ConfigError::PidDeltaOutOfRange)
    ));
    assert!(matches!(
        Controller::new(Config {
            pid: PidConfig {
                integral_decay: -0.1,
                ..PidConfig::default()
            },
            ..Config::default()
        }),
        Err(ConfigError::PidDecayOutOfRange)
    ));
}
