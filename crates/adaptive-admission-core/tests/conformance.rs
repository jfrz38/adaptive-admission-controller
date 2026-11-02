use std::collections::HashMap;
use std::fs;
use std::path::Path;

use adaptive_admission_core::{
    Action, CancellationReason, Config, ConfigError, Controller, ControllerDiagnostics,
    ControllerError, EvictionReason, Outcome, PidConfig, Priority, PriorityError, RejectionReason,
    RequestId, StatisticsConfig, StatisticsSnapshot,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Scenario {
    name: String,
    #[serde(default)]
    configuration: ConfigFixture,
    #[serde(default)]
    expect_create_error: Option<ExpectedConfigError>,
    #[serde(default)]
    steps: Vec<Step>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ConfigFixture {
    initial_threshold: Option<u16>,
    max_concurrent_requests: Option<usize>,
    max_queued_requests: Option<usize>,
    queue_timeout_ms: Option<u64>,
    queue_timeout_ratio: Option<f64>,
    statistics: Option<StatisticsConfigFixture>,
    pid: Option<PidConfigFixture>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StatisticsConfigFixture {
    min_window_ms: Option<u64>,
    max_window_ms: Option<u64>,
    min_samples: Option<usize>,
    min_percentile_samples: Option<usize>,
    max_samples: Option<usize>,
    latency_percentile: Option<u8>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PidConfigFixture {
    proportional_gain: Option<f64>,
    integral_gain: Option<f64>,
    derivative_gain: Option<f64>,
    interval_ms: Option<u64>,
    max_delta_percentage: Option<f64>,
    integral_decay: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "kebab-case", deny_unknown_fields)]
enum Step {
    Submit {
        #[serde(rename = "atMs")]
        at_ms: u64,
        priority: u16,
        #[serde(rename = "as")]
        request: String,
        #[serde(default)]
        actions: Option<Vec<ExpectedAction>>,
        #[serde(default)]
        error: Option<ExpectedError>,
    },
    Complete {
        #[serde(rename = "atMs")]
        at_ms: u64,
        request: RequestReference,
        outcome: ExpectedOutcome,
        #[serde(default)]
        actions: Option<Vec<ExpectedAction>>,
        #[serde(default)]
        error: Option<ExpectedError>,
    },
    Cancel {
        #[serde(rename = "atMs")]
        at_ms: u64,
        request: RequestReference,
        #[serde(default)]
        actions: Option<Vec<ExpectedAction>>,
        #[serde(default)]
        error: Option<ExpectedError>,
    },
    AdvanceTime {
        #[serde(rename = "atMs")]
        at_ms: u64,
        #[serde(default)]
        actions: Option<Vec<ExpectedAction>>,
        #[serde(default)]
        error: Option<ExpectedError>,
    },
    Shutdown {
        #[serde(rename = "atMs")]
        at_ms: u64,
        #[serde(default)]
        actions: Option<Vec<ExpectedAction>>,
        #[serde(default)]
        error: Option<ExpectedError>,
    },
    NextDeadline {
        #[serde(rename = "expectMs")]
        expect_ms: Option<u64>,
    },
    LatestStatistics {
        expect: Option<ExpectedStatistics>,
    },
    LatestDiagnostics {
        expect: ExpectedDiagnostics,
    },
    PriorityPercentile {
        percentile: u8,
        expect: Option<u16>,
    },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
enum RequestReference {
    Alias(String),
    Id(u64),
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum ExpectedOutcome {
    Success,
    Failure,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
enum ExpectedAction {
    ThresholdChanged {
        previous: u16,
        current: u16,
    },
    ConcurrencyChanged {
        previous: usize,
        current: usize,
    },
    Start {
        request: String,
    },
    Reject {
        request: String,
        reason: ExpectedRejectionReason,
    },
    Evict {
        request: String,
        reason: ExpectedEvictionReason,
    },
    Cancel {
        request: String,
        reason: ExpectedCancellationReason,
    },
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
enum ExpectedRejectionReason {
    Threshold { priority: u16, threshold: u16 },
    QueueFull,
    Shutdown,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
enum ExpectedEvictionReason {
    QueueTimeout,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
enum ExpectedCancellationReason {
    Requested,
    Shutdown,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
enum ExpectedError {
    PriorityOutOfRange {
        value: u16,
    },
    TimeRegression {
        #[serde(rename = "previousMs")]
        previous_ms: u64,
        #[serde(rename = "receivedMs")]
        received_ms: u64,
    },
    UnknownRequest {
        #[serde(rename = "requestId")]
        request_id: u64,
    },
    InvalidTransition {
        #[serde(rename = "requestId")]
        request_id: u64,
        operation: String,
        state: String,
    },
    RequestIdExhausted,
    SequenceExhausted,
    DeadlineOverflow,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
enum ExpectedConfigError {
    ThresholdOutOfRange {
        value: u16,
    },
    ZeroConcurrency,
    ZeroQueueTimeout,
    QueueTimeoutRatioOutOfRange,
    ZeroStatisticsMinWindow,
    StatisticsWindowOrder {
        #[serde(rename = "minimumMs")]
        minimum_ms: u64,
        #[serde(rename = "maximumMs")]
        maximum_ms: u64,
    },
    ZeroStatisticsMinSamples,
    ZeroStatisticsPercentileSamples,
    StatisticsSampleLimitTooSmall {
        maximum: usize,
        required: usize,
    },
    StatisticsPercentileOutOfRange {
        value: u8,
    },
    InvalidPidGain {
        name: String,
    },
    ZeroPidInterval,
    PidDeltaOutOfRange,
    PidDecayOutOfRange,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExpectedStatistics {
    id: u64,
    started_at_ms: u64,
    ended_at_ms: u64,
    percentile_latency_ms: u64,
    minimum_latency_ms: u64,
    average_latency_ms: f64,
    successful_requests: u64,
    throughput_per_second: f64,
    max_in_flight: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExpectedDiagnostics {
    threshold: u16,
    concurrency_limit: usize,
    queue_timeout_ms: u64,
    target_latency_ms: f64,
    aggregated_latency_ms: Option<f64>,
}

#[test]
fn all_shared_conformance_scenarios_match_the_core() {
    let conformance = conformance_directory();
    let directory = conformance.join("scenarios");
    let mut paths = fs::read_dir(&directory)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", directory.display()))
        .map(|entry| {
            entry
                .expect("scenario directory entry must be readable")
                .path()
        })
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect::<Vec<_>>();
    paths.sort();
    assert!(!paths.is_empty(), "no conformance scenarios were found");

    for path in paths {
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        let scenarios: Vec<Scenario> = serde_json::from_str(&source)
            .unwrap_or_else(|error| panic!("invalid fixture {}: {error}", path.display()));
        assert!(
            !scenarios.is_empty(),
            "{} contains no scenarios",
            path.display()
        );
        for scenario in scenarios {
            run_scenario(scenario, &path);
        }
    }
}

#[test]
fn conformance_schema_is_well_formed_json() {
    let path = conformance_directory().join("schema.json");
    let source = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    serde_json::from_str::<serde_json::Value>(&source)
        .unwrap_or_else(|error| panic!("invalid schema {}: {error}", path.display()));
}

fn conformance_directory() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("conformance")
}

fn run_scenario(scenario: Scenario, path: &Path) {
    let context = format!("{}: {}", path.display(), scenario.name);
    let result = Controller::new(scenario.configuration.into_config());

    if let Some(expected) = scenario.expect_create_error {
        let actual: ExpectedConfigError = match result {
            Ok(_) => panic!("{context}: controller creation should fail"),
            Err(error) => error.into(),
        };
        assert_eq!(actual, expected, "{context}: unexpected creation error");
        assert!(
            scenario.steps.is_empty(),
            "{context}: an invalid configuration cannot have steps"
        );
        return;
    }

    let mut controller = result
        .unwrap_or_else(|error| panic!("{context}: controller creation should succeed: {error}"));
    let mut requests = HashMap::new();
    let mut aliases = HashMap::new();

    for (index, step) in scenario.steps.into_iter().enumerate() {
        let step_context = format!("{context}, step {}", index + 1);
        run_step(
            step,
            &mut controller,
            &mut requests,
            &mut aliases,
            &step_context,
        );
    }
}

fn run_step(
    step: Step,
    controller: &mut Controller,
    requests: &mut HashMap<String, RequestId>,
    aliases: &mut HashMap<RequestId, String>,
    context: &str,
) {
    match step {
        Step::Submit {
            at_ms,
            priority,
            request,
            actions,
            error,
        } => {
            assert!(
                !requests.contains_key(&request),
                "{context}: duplicate request alias {request}"
            );
            let result = Priority::new(priority)
                .map_err(ExpectedError::from)
                .and_then(|priority| {
                    controller
                        .submit(priority, at_ms.into())
                        .map_err(ExpectedError::from)
                });
            match result {
                Ok(submission) => {
                    requests.insert(request.clone(), submission.request_id);
                    aliases.insert(submission.request_id, request);
                    assert_actions(submission.actions, actions, error, aliases, context);
                }
                Err(actual) => assert_error(actual, actions, error, context),
            }
        }
        Step::Complete {
            at_ms,
            request,
            outcome,
            actions,
            error,
        } => {
            let request_id = resolve_request(&request, requests, context);
            let result = controller
                .complete(request_id, outcome.into(), at_ms.into())
                .map_err(ExpectedError::from);
            assert_result(result, actions, error, aliases, context);
        }
        Step::Cancel {
            at_ms,
            request,
            actions,
            error,
        } => {
            let request_id = resolve_request(&request, requests, context);
            let result = controller
                .cancel(request_id, at_ms.into())
                .map_err(ExpectedError::from);
            assert_result(result, actions, error, aliases, context);
        }
        Step::AdvanceTime {
            at_ms,
            actions,
            error,
        } => {
            let result = controller
                .advance_time(at_ms.into())
                .map_err(ExpectedError::from);
            assert_result(result, actions, error, aliases, context);
        }
        Step::Shutdown {
            at_ms,
            actions,
            error,
        } => {
            let result = controller
                .shutdown(at_ms.into())
                .map_err(ExpectedError::from);
            assert_result(result, actions, error, aliases, context);
        }
        Step::NextDeadline { expect_ms } => assert_eq!(
            controller
                .next_deadline()
                .map(|deadline| deadline.as_millis()),
            expect_ms,
            "{context}: unexpected next deadline"
        ),
        Step::LatestStatistics { expect } => {
            assert_statistics(controller.latest_statistics(), expect.as_ref(), context);
        }
        Step::LatestDiagnostics { expect } => {
            assert_diagnostics(controller.diagnostics(), &expect, context);
        }
        Step::PriorityPercentile { percentile, expect } => assert_eq!(
            controller.priority_percentile(percentile),
            expect,
            "{context}: unexpected priority percentile"
        ),
    }
}

fn assert_result(
    result: Result<Vec<Action>, ExpectedError>,
    expected_actions: Option<Vec<ExpectedAction>>,
    expected_error: Option<ExpectedError>,
    aliases: &HashMap<RequestId, String>,
    context: &str,
) {
    match result {
        Ok(actions) => assert_actions(actions, expected_actions, expected_error, aliases, context),
        Err(error) => assert_error(error, expected_actions, expected_error, context),
    }
}

fn assert_actions(
    actions: Vec<Action>,
    expected_actions: Option<Vec<ExpectedAction>>,
    expected_error: Option<ExpectedError>,
    aliases: &HashMap<RequestId, String>,
    context: &str,
) {
    assert!(
        expected_error.is_none(),
        "{context}: operation succeeded but an error was expected"
    );
    let expected = expected_actions
        .unwrap_or_else(|| panic!("{context}: successful fixture step must declare actions"));
    let actual = actions
        .into_iter()
        .map(|action| normalize_action(action, aliases, context))
        .collect::<Vec<_>>();
    assert_eq!(actual, expected, "{context}: unexpected actions");
}

fn assert_error(
    actual: ExpectedError,
    expected_actions: Option<Vec<ExpectedAction>>,
    expected_error: Option<ExpectedError>,
    context: &str,
) {
    assert!(
        expected_actions.is_none(),
        "{context}: operation failed but actions were expected"
    );
    assert_eq!(
        Some(actual),
        expected_error,
        "{context}: unexpected operation error"
    );
}

fn normalize_action(
    action: Action,
    aliases: &HashMap<RequestId, String>,
    context: &str,
) -> ExpectedAction {
    let alias = |request_id: RequestId| {
        aliases
            .get(&request_id)
            .unwrap_or_else(|| panic!("{context}: no alias for request {}", request_id.value()))
            .clone()
    };
    match action {
        Action::ThresholdChanged { previous, current } => {
            ExpectedAction::ThresholdChanged { previous, current }
        }
        Action::ConcurrencyChanged { previous, current } => {
            ExpectedAction::ConcurrencyChanged { previous, current }
        }
        Action::Start { request_id } => ExpectedAction::Start {
            request: alias(request_id),
        },
        Action::Reject { request_id, reason } => ExpectedAction::Reject {
            request: alias(request_id),
            reason: reason.into(),
        },
        Action::Evict { request_id, reason } => ExpectedAction::Evict {
            request: alias(request_id),
            reason: reason.into(),
        },
        Action::Cancel { request_id, reason } => ExpectedAction::Cancel {
            request: alias(request_id),
            reason: reason.into(),
        },
    }
}

fn resolve_request(
    request: &RequestReference,
    requests: &HashMap<String, RequestId>,
    context: &str,
) -> RequestId {
    match request {
        RequestReference::Alias(alias) => *requests
            .get(alias)
            .unwrap_or_else(|| panic!("{context}: unknown request alias {alias}")),
        RequestReference::Id(value) => RequestId::from_value(*value),
    }
}

fn assert_statistics(
    actual: Option<&StatisticsSnapshot>,
    expected: Option<&ExpectedStatistics>,
    context: &str,
) {
    match (actual, expected) {
        (None, None) => {}
        (Some(actual), Some(expected)) => {
            assert_eq!(actual.id, expected.id, "{context}: snapshot id");
            assert_eq!(
                actual.started_at.as_millis(),
                expected.started_at_ms,
                "{context}: snapshot start"
            );
            assert_eq!(
                actual.ended_at.as_millis(),
                expected.ended_at_ms,
                "{context}: snapshot end"
            );
            assert_eq!(
                actual.percentile_latency_ms, expected.percentile_latency_ms,
                "{context}: percentile latency"
            );
            assert_eq!(
                actual.minimum_latency_ms, expected.minimum_latency_ms,
                "{context}: minimum latency"
            );
            assert_float(
                actual.average_latency_ms,
                expected.average_latency_ms,
                context,
                "average latency",
            );
            assert_eq!(
                actual.successful_requests, expected.successful_requests,
                "{context}: successful requests"
            );
            assert_float(
                actual.throughput_per_second,
                expected.throughput_per_second,
                context,
                "throughput",
            );
            assert_eq!(
                actual.max_in_flight, expected.max_in_flight,
                "{context}: maximum in-flight"
            );
        }
        _ => panic!("{context}: unexpected latest statistics presence"),
    }
}

fn assert_diagnostics(
    actual: ControllerDiagnostics,
    expected: &ExpectedDiagnostics,
    context: &str,
) {
    assert_eq!(actual.threshold, expected.threshold, "{context}: threshold");
    assert_eq!(
        actual.concurrency_limit, expected.concurrency_limit,
        "{context}: concurrency limit"
    );
    assert_eq!(
        actual.queue_timeout_ms, expected.queue_timeout_ms,
        "{context}: queue timeout"
    );
    assert_float(
        actual.target_latency_ms,
        expected.target_latency_ms,
        context,
        "target latency",
    );
    match (actual.aggregated_latency_ms, expected.aggregated_latency_ms) {
        (None, None) => {}
        (Some(actual), Some(expected)) => {
            assert_float(actual, expected, context, "aggregated latency");
        }
        (actual, expected) => panic!(
            "{context}: aggregated latency mismatch: actual {actual:?}, expected {expected:?}"
        ),
    }
}

fn assert_float(actual: f64, expected: f64, context: &str, field: &str) {
    let tolerance = f64::EPSILON * actual.abs().max(expected.abs()).max(1.0) * 8.0;
    assert!(
        (actual - expected).abs() <= tolerance,
        "{context}: unexpected {field}: expected {expected}, got {actual}"
    );
}

impl ConfigFixture {
    fn into_config(self) -> Config {
        let mut config = Config::default();
        if let Some(value) = self.initial_threshold {
            config.initial_threshold = value;
        }
        if let Some(value) = self.max_concurrent_requests {
            config.max_concurrent_requests = value;
        }
        if let Some(value) = self.max_queued_requests {
            config.max_queued_requests = value;
        }
        if let Some(value) = self.queue_timeout_ms {
            config.queue_timeout_ms = value;
        }
        if let Some(value) = self.queue_timeout_ratio {
            config.queue_timeout_ratio = value;
        }
        if let Some(value) = self.statistics {
            config.statistics = value.apply(config.statistics);
        }
        if let Some(value) = self.pid {
            config.pid = value.apply(config.pid);
        }
        config
    }
}

impl StatisticsConfigFixture {
    fn apply(self, mut config: StatisticsConfig) -> StatisticsConfig {
        if let Some(value) = self.min_window_ms {
            config.min_window_ms = value;
        }
        if let Some(value) = self.max_window_ms {
            config.max_window_ms = value;
        }
        if let Some(value) = self.min_samples {
            config.min_samples = value;
        }
        if let Some(value) = self.min_percentile_samples {
            config.min_percentile_samples = value;
        }
        if let Some(value) = self.max_samples {
            config.max_samples = value;
        }
        if let Some(value) = self.latency_percentile {
            config.latency_percentile = value;
        }
        config
    }
}

impl PidConfigFixture {
    fn apply(self, mut config: PidConfig) -> PidConfig {
        if let Some(value) = self.proportional_gain {
            config.proportional_gain = value;
        }
        if let Some(value) = self.integral_gain {
            config.integral_gain = value;
        }
        if let Some(value) = self.derivative_gain {
            config.derivative_gain = value;
        }
        if let Some(value) = self.interval_ms {
            config.interval_ms = value;
        }
        if let Some(value) = self.max_delta_percentage {
            config.max_delta_percentage = value;
        }
        if let Some(value) = self.integral_decay {
            config.integral_decay = value;
        }
        config
    }
}

impl From<ExpectedOutcome> for Outcome {
    fn from(value: ExpectedOutcome) -> Self {
        match value {
            ExpectedOutcome::Success => Self::Success,
            ExpectedOutcome::Failure => Self::Failure,
        }
    }
}

impl From<PriorityError> for ExpectedError {
    fn from(error: PriorityError) -> Self {
        match error {
            PriorityError::ValueOutOfRange { value } => Self::PriorityOutOfRange { value },
            PriorityError::TierOutOfRange { .. } | PriorityError::CohortOutOfRange { .. } => {
                unreachable!("conformance submissions construct priorities from raw values")
            }
        }
    }
}

impl From<ControllerError> for ExpectedError {
    fn from(error: ControllerError) -> Self {
        match error {
            ControllerError::TimeRegression { previous, received } => Self::TimeRegression {
                previous_ms: previous.as_millis(),
                received_ms: received.as_millis(),
            },
            ControllerError::UnknownRequest { request_id } => Self::UnknownRequest {
                request_id: request_id.value(),
            },
            ControllerError::InvalidTransition {
                request_id,
                operation,
                state,
            } => Self::InvalidTransition {
                request_id: request_id.value(),
                operation: operation.to_owned(),
                state: state.to_owned(),
            },
            ControllerError::RequestIdExhausted => Self::RequestIdExhausted,
            ControllerError::SequenceExhausted => Self::SequenceExhausted,
            ControllerError::DeadlineOverflow => Self::DeadlineOverflow,
        }
    }
}

impl From<ConfigError> for ExpectedConfigError {
    fn from(error: ConfigError) -> Self {
        match error {
            ConfigError::ThresholdOutOfRange { value } => Self::ThresholdOutOfRange { value },
            ConfigError::ZeroConcurrency => Self::ZeroConcurrency,
            ConfigError::ZeroQueueTimeout => Self::ZeroQueueTimeout,
            ConfigError::QueueTimeoutRatioOutOfRange => Self::QueueTimeoutRatioOutOfRange,
            ConfigError::ZeroStatisticsMinWindow => Self::ZeroStatisticsMinWindow,
            ConfigError::StatisticsWindowOrder {
                minimum_ms,
                maximum_ms,
            } => Self::StatisticsWindowOrder {
                minimum_ms,
                maximum_ms,
            },
            ConfigError::ZeroStatisticsMinSamples => Self::ZeroStatisticsMinSamples,
            ConfigError::ZeroStatisticsPercentileSamples => Self::ZeroStatisticsPercentileSamples,
            ConfigError::StatisticsSampleLimitTooSmall { maximum, required } => {
                Self::StatisticsSampleLimitTooSmall { maximum, required }
            }
            ConfigError::StatisticsPercentileOutOfRange { value } => {
                Self::StatisticsPercentileOutOfRange { value }
            }
            ConfigError::InvalidPidGain { name } => Self::InvalidPidGain {
                name: name.to_owned(),
            },
            ConfigError::ZeroPidInterval => Self::ZeroPidInterval,
            ConfigError::PidDeltaOutOfRange => Self::PidDeltaOutOfRange,
            ConfigError::PidDecayOutOfRange => Self::PidDecayOutOfRange,
        }
    }
}

impl From<RejectionReason> for ExpectedRejectionReason {
    fn from(reason: RejectionReason) -> Self {
        match reason {
            RejectionReason::Threshold {
                priority,
                threshold,
            } => Self::Threshold {
                priority: priority.value(),
                threshold,
            },
            RejectionReason::QueueFull => Self::QueueFull,
            RejectionReason::Shutdown => Self::Shutdown,
        }
    }
}

impl From<EvictionReason> for ExpectedEvictionReason {
    fn from(reason: EvictionReason) -> Self {
        match reason {
            EvictionReason::QueueTimeout => Self::QueueTimeout,
        }
    }
}

impl From<CancellationReason> for ExpectedCancellationReason {
    fn from(reason: CancellationReason) -> Self {
        match reason {
            CancellationReason::Requested => Self::Requested,
            CancellationReason::Shutdown => Self::Shutdown,
        }
    }
}
