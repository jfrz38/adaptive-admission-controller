use adaptive_admission_core::{
    Config, ConfigError, Controller, Outcome, Priority, StatisticsConfig, Timestamp,
};

fn priority(value: u16) -> Priority {
    Priority::new(value).expect("test priority must be valid")
}

fn statistics_config() -> StatisticsConfig {
    StatisticsConfig {
        min_window_ms: 100,
        max_window_ms: 1_000,
        min_samples: 2,
        min_percentile_samples: 2,
        max_samples: 10,
        latency_percentile: 90,
    }
}

fn controller(statistics: StatisticsConfig, max_concurrent_requests: usize) -> Controller {
    Controller::new(Config {
        max_concurrent_requests,
        queue_timeout_ms: 10_000,
        statistics,
        ..Config::default()
    })
    .expect("test configuration must be valid")
}

#[test]
fn publishes_one_coherent_snapshot_from_successful_execution_times() {
    let mut controller = controller(statistics_config(), 1);
    let first = controller.submit(priority(0), 0.into()).unwrap();
    let second = controller.submit(priority(1), 10.into()).unwrap();

    controller
        .complete(first.request_id, Outcome::Success, 100.into())
        .unwrap();
    controller
        .complete(second.request_id, Outcome::Success, 300.into())
        .unwrap();

    let snapshot = controller
        .latest_statistics()
        .expect("the populated window should be published");
    assert_eq!(snapshot.id, 0);
    assert_eq!(snapshot.started_at, Timestamp::from_millis(0));
    assert_eq!(snapshot.ended_at, Timestamp::from_millis(300));
    assert_eq!(snapshot.percentile_latency_ms, 190);
    assert_eq!(snapshot.minimum_latency_ms, 100);
    assert_eq!(snapshot.average_latency_ms, 150.0);
    assert_eq!(snapshot.successful_requests, 2);
    assert!((snapshot.throughput_per_second - (2_000.0 / 300.0)).abs() < f64::EPSILON);
    assert_eq!(snapshot.max_in_flight, 1);
}

#[test]
fn failures_release_capacity_without_contributing_samples() {
    let mut controller = controller(
        StatisticsConfig {
            min_samples: 1,
            min_percentile_samples: 1,
            ..statistics_config()
        },
        1,
    );
    let failed = controller.submit(priority(0), 0.into()).unwrap();

    controller
        .complete(failed.request_id, Outcome::Failure, 100.into())
        .unwrap();
    assert!(controller.latest_statistics().is_none());

    let successful = controller.submit(priority(0), 100.into()).unwrap();
    controller
        .complete(successful.request_id, Outcome::Success, 200.into())
        .unwrap();

    let snapshot = controller.latest_statistics().unwrap();
    assert_eq!(snapshot.successful_requests, 1);
    assert_eq!(snapshot.minimum_latency_ms, 100);
}

#[test]
fn maximum_duration_publishes_a_low_throughput_window() {
    let mut controller = controller(
        StatisticsConfig {
            max_window_ms: 500,
            min_samples: 1,
            min_percentile_samples: 3,
            ..statistics_config()
        },
        1,
    );
    let request = controller.submit(priority(0), 0.into()).unwrap();
    controller
        .complete(request.request_id, Outcome::Success, 50.into())
        .unwrap();

    controller.advance_time(500.into()).unwrap();

    let snapshot = controller.latest_statistics().unwrap();
    assert_eq!(snapshot.started_at, Timestamp::from_millis(0));
    assert_eq!(snapshot.ended_at, Timestamp::from_millis(500));
    assert_eq!(snapshot.successful_requests, 1);
    assert_eq!(snapshot.throughput_per_second, 2.0);
}

#[test]
fn bounded_latency_samples_do_not_change_whole_window_minimum_or_average() {
    let mut controller = controller(
        StatisticsConfig {
            min_window_ms: 500,
            min_samples: 2,
            min_percentile_samples: 2,
            max_samples: 2,
            latency_percentile: 50,
            ..statistics_config()
        },
        3,
    );
    let first = controller.submit(priority(0), 0.into()).unwrap();
    let second = controller.submit(priority(0), 0.into()).unwrap();
    let third = controller.submit(priority(0), 0.into()).unwrap();
    controller
        .complete(first.request_id, Outcome::Success, 10.into())
        .unwrap();
    controller
        .complete(second.request_id, Outcome::Success, 20.into())
        .unwrap();
    controller
        .complete(third.request_id, Outcome::Success, 100.into())
        .unwrap();

    assert_eq!(
        controller.next_deadline(),
        Some(Timestamp::from_millis(500))
    );
    controller.advance_time(500.into()).unwrap();

    let snapshot = controller.latest_statistics().unwrap();
    assert_eq!(snapshot.successful_requests, 3);
    assert_eq!(snapshot.minimum_latency_ms, 10);
    assert_eq!(snapshot.average_latency_ms, 130.0 / 3.0);
    assert_eq!(snapshot.percentile_latency_ms, 60);
    assert_eq!(snapshot.max_in_flight, 3);
}

#[test]
fn discarded_window_does_not_replace_the_latest_published_snapshot() {
    let mut controller = controller(
        StatisticsConfig {
            max_window_ms: 200,
            ..statistics_config()
        },
        2,
    );
    let first = controller.submit(priority(0), 0.into()).unwrap();
    let second = controller.submit(priority(0), 0.into()).unwrap();
    controller
        .complete(first.request_id, Outcome::Success, 50.into())
        .unwrap();
    controller
        .complete(second.request_id, Outcome::Success, 100.into())
        .unwrap();
    assert_eq!(controller.latest_statistics().unwrap().id, 0);

    let insufficient = controller.submit(priority(0), 100.into()).unwrap();
    controller
        .complete(insufficient.request_id, Outcome::Success, 150.into())
        .unwrap();
    controller.advance_time(300.into()).unwrap();

    assert_eq!(controller.latest_statistics().unwrap().id, 0);
}

#[test]
fn completion_at_the_maximum_boundary_belongs_to_the_new_window() {
    let mut controller = controller(
        StatisticsConfig {
            max_window_ms: 200,
            min_samples: 1,
            min_percentile_samples: 1,
            ..statistics_config()
        },
        1,
    );
    let request = controller.submit(priority(0), 0.into()).unwrap();

    controller
        .complete(request.request_id, Outcome::Success, 200.into())
        .unwrap();
    assert!(controller.latest_statistics().is_none());
    controller.advance_time(400.into()).unwrap();

    let snapshot = controller.latest_statistics().unwrap();
    assert_eq!(snapshot.id, 1);
    assert_eq!(snapshot.started_at, Timestamp::from_millis(200));
    assert_eq!(snapshot.ended_at, Timestamp::from_millis(400));
    assert_eq!(snapshot.minimum_latency_ms, 200);
}

#[test]
fn priority_distribution_includes_operational_rejections_but_not_shutdown() {
    let mut controller = Controller::new(Config {
        initial_threshold: 100,
        max_concurrent_requests: 1,
        max_queued_requests: 0,
        statistics: StatisticsConfig {
            min_samples: 3,
            min_percentile_samples: 3,
            max_samples: 3,
            ..statistics_config()
        },
        ..Config::default()
    })
    .unwrap();

    let _ = controller.submit(priority(0), 0.into()).unwrap();
    let _ = controller.submit(priority(100), 0.into()).unwrap();
    let _ = controller.submit(priority(50), 0.into()).unwrap();

    assert_eq!(controller.priority_percentile(0), Some(0));
    assert_eq!(controller.priority_percentile(100), Some(100));
    assert_eq!(controller.priority_percentile(101), None);

    controller.shutdown(1.into()).unwrap();
    let _ = controller.submit(priority(700), 1.into()).unwrap();
    assert_eq!(controller.priority_percentile(100), Some(100));
}

#[test]
fn next_deadline_combines_queue_and_statistics_windows() {
    let mut controller = Controller::new(Config {
        max_concurrent_requests: 1,
        queue_timeout_ms: 200,
        statistics: StatisticsConfig {
            max_window_ms: 1_000,
            ..statistics_config()
        },
        ..Config::default()
    })
    .unwrap();
    let _ = controller.submit(priority(0), 0.into()).unwrap();
    let _ = controller.submit(priority(1), 50.into()).unwrap();

    assert_eq!(
        controller.next_deadline(),
        Some(Timestamp::from_millis(250))
    );
}

#[test]
fn validates_statistics_configuration() {
    assert!(matches!(
        Controller::new(Config {
            statistics: StatisticsConfig {
                min_window_ms: 0,
                ..statistics_config()
            },
            ..Config::default()
        }),
        Err(ConfigError::ZeroStatisticsMinWindow)
    ));
    assert!(matches!(
        Controller::new(Config {
            statistics: StatisticsConfig {
                min_window_ms: 200,
                max_window_ms: 100,
                ..statistics_config()
            },
            ..Config::default()
        }),
        Err(ConfigError::StatisticsWindowOrder {
            minimum_ms: 200,
            maximum_ms: 100,
        })
    ));
    assert!(matches!(
        Controller::new(Config {
            statistics: StatisticsConfig {
                max_samples: 1,
                ..statistics_config()
            },
            ..Config::default()
        }),
        Err(ConfigError::StatisticsSampleLimitTooSmall {
            maximum: 1,
            required: 2,
        })
    ));
    assert!(matches!(
        Controller::new(Config {
            statistics: StatisticsConfig {
                latency_percentile: 101,
                ..statistics_config()
            },
            ..Config::default()
        }),
        Err(ConfigError::StatisticsPercentileOutOfRange { value: 101 })
    ));
}
