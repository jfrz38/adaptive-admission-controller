use std::collections::VecDeque;

use crate::{StatisticsSnapshot, Timestamp};

const CONCURRENCY_INTERVAL_MS: u64 = 2_000;
const LATENCY_INTERVAL_MS: u64 = 10_000;
const HISTORY_LIMIT: usize = 50;
const HISTORY_WARMUP: usize = 10;
const LATENCY_SAMPLE_LIMIT: usize = 5;

pub(crate) struct AutoTuner {
    concurrency: ConcurrencyController,
    latency: LatencyController,
    history: ControllerHistory,
    next_concurrency_deadline: Option<Timestamp>,
    next_latency_deadline: Option<Timestamp>,
    last_concurrency_snapshot_id: Option<u64>,
    last_latency_snapshot_id: Option<u64>,
}

impl AutoTuner {
    pub(crate) fn new() -> Self {
        Self {
            concurrency: ConcurrencyController::new(),
            latency: LatencyController::new(),
            history: ControllerHistory::new(),
            next_concurrency_deadline: None,
            next_latency_deadline: None,
            last_concurrency_snapshot_id: None,
            last_latency_snapshot_id: None,
        }
    }

    pub(crate) fn next_deadline(&self) -> Option<Timestamp> {
        [self.next_concurrency_deadline, self.next_latency_deadline]
            .into_iter()
            .flatten()
            .min()
    }

    pub(crate) fn target_latency_ms(&self) -> f64 {
        self.latency.target()
    }

    pub(crate) fn aggregated_latency_ms(&self) -> Option<f64> {
        self.latency.filtered_latency
    }

    pub(crate) fn advance(
        &mut self,
        now: Timestamp,
        snapshot: Option<&StatisticsSnapshot>,
        current_limit: usize,
    ) -> Option<usize> {
        if self.next_concurrency_deadline.is_none() {
            self.schedule_concurrency_from(now);
        }
        if self.next_latency_deadline.is_none() {
            self.schedule_latency_from(now);
        }

        let mut new_limit = None;
        if self
            .next_concurrency_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            self.schedule_concurrency_from(now);
            if let Some(snapshot) = snapshot
                && self.last_concurrency_snapshot_id != Some(snapshot.id)
            {
                self.last_concurrency_snapshot_id = Some(snapshot.id);
                self.history
                    .push(snapshot.max_in_flight, snapshot.throughput_per_second);
                let aggregated_latency = self.latency.aggregate(snapshot.percentile_latency_ms);
                let update = self.concurrency.update(
                    current_limit,
                    aggregated_latency,
                    self.latency.target(),
                    snapshot.max_in_flight,
                );
                if update.reset_latency {
                    self.latency.reset(aggregated_latency);
                    self.last_latency_snapshot_id = Some(snapshot.id);
                }
                if update.limit != current_limit {
                    new_limit = Some(update.limit);
                }
            }
        }

        if self
            .next_latency_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            self.schedule_latency_from(now);
            if let Some(snapshot) = snapshot
                && self.last_latency_snapshot_id != Some(snapshot.id)
            {
                self.last_latency_snapshot_id = Some(snapshot.id);
                self.latency.update(snapshot, &self.history);
            }
        }

        new_limit
    }

    fn schedule_concurrency_from(&mut self, now: Timestamp) {
        self.next_concurrency_deadline = Some(Timestamp::from_millis(
            now.as_millis().saturating_add(CONCURRENCY_INTERVAL_MS),
        ));
    }

    fn schedule_latency_from(&mut self, now: Timestamp) {
        self.next_latency_deadline = Some(Timestamp::from_millis(
            now.as_millis().saturating_add(LATENCY_INTERVAL_MS),
        ));
    }
}

struct ConcurrencyController {
    lower_bound_hits: u8,
}

impl ConcurrencyController {
    fn new() -> Self {
        Self {
            lower_bound_hits: 0,
        }
    }

    fn update(
        &mut self,
        current_limit: usize,
        aggregated_latency: f64,
        target_latency: f64,
        observed_in_flight: usize,
    ) -> ConcurrencyUpdate {
        let mut new_limit = self.calculate_limit(
            current_limit,
            aggregated_latency,
            target_latency,
            observed_in_flight,
        );
        let mut reset_latency = false;

        if new_limit == 1 {
            self.lower_bound_hits = self.lower_bound_hits.saturating_add(1);
            if self.lower_bound_hits >= 3 {
                self.lower_bound_hits = 0;
                new_limit = 2;
                reset_latency = true;
            }
        } else {
            self.lower_bound_hits = 0;
        }

        ConcurrencyUpdate {
            limit: new_limit,
            reset_latency,
        }
    }

    fn calculate_limit(
        &self,
        current_limit: usize,
        aggregated_latency: f64,
        target_latency: f64,
        observed_in_flight: usize,
    ) -> usize {
        if !aggregated_latency.is_finite()
            || aggregated_latency <= 0.0
            || !target_latency.is_finite()
            || target_latency <= 0.0
        {
            return current_limit;
        }

        let limit = current_limit as f64;
        let queue = limit * (1.0 - target_latency / aggregated_latency);
        let logarithm = limit.log10();
        if !queue.is_finite() || !logarithm.is_finite() {
            return current_limit;
        }

        let alpha = 3.0 * logarithm;
        let beta = 5.0 * logarithm;
        let step = (logarithm.floor() as usize).max(1);
        let candidate = if queue <= alpha {
            current_limit.saturating_add(step)
        } else if queue > beta {
            current_limit.saturating_sub(step)
        } else {
            current_limit
        };
        let upper_bound = observed_in_flight.max(1).saturating_mul(10);

        candidate.clamp(1, upper_bound)
    }
}

struct ConcurrencyUpdate {
    limit: usize,
    reset_latency: bool,
}

struct LatencyController {
    samples: VecDeque<u64>,
    filtered_latency: Option<f64>,
    target_latency: f64,
}

impl LatencyController {
    fn new() -> Self {
        Self {
            samples: VecDeque::with_capacity(LATENCY_SAMPLE_LIMIT),
            filtered_latency: None,
            target_latency: 100.0,
        }
    }

    fn target(&self) -> f64 {
        self.target_latency
    }

    fn aggregate(&mut self, sample: u64) -> f64 {
        if sample == 0 {
            return self.filtered_latency.unwrap_or(self.target_latency);
        }
        if self.samples.len() == LATENCY_SAMPLE_LIMIT {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);

        let mut sorted: Vec<_> = self.samples.iter().copied().collect();
        sorted.sort_unstable();
        let median = sorted[(sorted.len() - 1) / 2] as f64;
        let filtered = self
            .filtered_latency
            .map_or(median, |previous| previous * 0.8 + median * 0.2);
        if filtered.is_finite() && filtered > 0.0 {
            self.filtered_latency = Some(filtered);
        }
        self.filtered_latency.unwrap_or(self.target_latency)
    }

    fn update(&mut self, snapshot: &StatisticsSnapshot, history: &ControllerHistory) {
        let minimum_latency = snapshot.minimum_latency_ms.max(1) as f64;
        let candidate = if history.len() < HISTORY_WARMUP {
            minimum_latency
        } else {
            match history.covariance() {
                Some(covariance) if covariance > 0.0 => {
                    (self.target_latency * 0.9 + minimum_latency * 0.1).round()
                }
                Some(covariance) if covariance < 0.0 => (self.target_latency * 0.8).round(),
                _ => self.target_latency,
            }
        };

        if candidate.is_finite() && candidate > 0.0 {
            self.target_latency = candidate.max(1.0);
        }
    }

    fn reset(&mut self, target_latency: f64) {
        self.target_latency = if target_latency.is_finite() && target_latency > 0.0 {
            target_latency
        } else {
            100.0
        };
        self.samples.clear();
        self.filtered_latency = None;
    }
}

struct ControllerHistory {
    entries: VecDeque<(usize, f64)>,
}

impl ControllerHistory {
    fn new() -> Self {
        Self {
            entries: VecDeque::with_capacity(HISTORY_LIMIT),
        }
    }

    fn push(&mut self, in_flight: usize, throughput: f64) {
        if !throughput.is_finite() {
            return;
        }
        if self.entries.len() == HISTORY_LIMIT {
            self.entries.pop_front();
        }
        self.entries.push_back((in_flight, throughput));
    }

    fn len(&self) -> usize {
        self.entries.len()
    }

    fn covariance(&self) -> Option<f64> {
        if self.entries.len() <= 1 {
            return None;
        }
        let count = self.entries.len() as f64;
        let mean_in_flight = self
            .entries
            .iter()
            .map(|(in_flight, _)| *in_flight as f64)
            .sum::<f64>()
            / count;
        let mean_throughput = self
            .entries
            .iter()
            .map(|(_, throughput)| throughput)
            .sum::<f64>()
            / count;
        let covariance = self
            .entries
            .iter()
            .map(|(in_flight, throughput)| {
                (*in_flight as f64 - mean_in_flight) * (throughput - mean_throughput)
            })
            .sum::<f64>()
            / count;

        covariance.is_finite().then_some(covariance)
    }
}

#[cfg(test)]
mod tests {
    use super::{ConcurrencyController, ControllerHistory, LatencyController};
    use crate::{StatisticsSnapshot, Timestamp};

    #[test]
    fn latency_filter_uses_a_five_sample_median_and_exponential_average() {
        let mut controller = LatencyController::new();

        assert_eq!(controller.aggregate(100), 100.0);
        assert_eq!(controller.aggregate(1_000), 100.0);
        assert_eq!(controller.aggregate(110), 102.0);
        assert!((controller.aggregate(120) - 103.6).abs() < 1e-10);
        assert!((controller.aggregate(130) - 106.88).abs() < 1e-10);
        assert!((controller.aggregate(140) - 111.504).abs() < 1e-10);
    }

    #[test]
    fn latency_target_uses_covariance_after_warmup() {
        let mut positive = ControllerHistory::new();
        let mut negative = ControllerHistory::new();
        for value in 1..=10 {
            positive.push(value, (value * 10) as f64);
            negative.push(value, (110 - value * 10) as f64);
        }
        let snapshot = snapshot(50);
        let mut controller = LatencyController::new();

        controller.update(&snapshot, &positive);
        assert_eq!(controller.target(), 95.0);
        controller.reset(100.0);
        controller.update(&snapshot, &negative);
        assert_eq!(controller.target(), 80.0);
    }

    #[test]
    fn history_keeps_the_latest_fifty_coherent_pairs() {
        let mut history = ControllerHistory::new();
        for value in 0..60 {
            history.push(value, (value * 2) as f64);
        }

        assert_eq!(history.len(), 50);
        assert_eq!(history.entries.front(), Some(&(10, 20.0)));
        assert_eq!(history.entries.back(), Some(&(59, 118.0)));
    }

    #[test]
    fn concurrency_is_bounded_by_observed_work() {
        let controller = ConcurrencyController::new();

        assert_eq!(controller.calculate_limit(100, 1.0, 100.0, 1), 10);
        assert_eq!(controller.calculate_limit(1, 10_000.0, 100.0, 1), 1);
    }

    #[test]
    fn three_lower_bound_hits_recalibrate_and_probe_two() {
        let mut controller = ConcurrencyController::new();

        assert_eq!(controller.update(1, 10_000.0, 100.0, 1).limit, 1);
        assert_eq!(controller.update(1, 10_000.0, 100.0, 1).limit, 1);
        let update = controller.update(1, 10_000.0, 100.0, 1);
        assert_eq!(update.limit, 2);
        assert!(update.reset_latency);
    }

    fn snapshot(minimum_latency_ms: u64) -> StatisticsSnapshot {
        StatisticsSnapshot {
            id: 1,
            started_at: Timestamp::from_millis(0),
            ended_at: Timestamp::from_millis(1_000),
            percentile_latency_ms: minimum_latency_ms,
            minimum_latency_ms,
            average_latency_ms: minimum_latency_ms as f64,
            successful_requests: 10,
            throughput_per_second: 10.0,
            max_in_flight: 1,
        }
    }
}
