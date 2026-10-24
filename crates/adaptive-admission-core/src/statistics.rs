use std::collections::VecDeque;

use crate::{Priority, StatisticsConfig, StatisticsSnapshot, Timestamp};

pub(crate) struct Statistics {
    config: StatisticsConfig,
    active: Option<ExecutionWindow>,
    priorities: VecDeque<u64>,
    latest_snapshot: Option<StatisticsSnapshot>,
    next_window_id: u64,
}

impl Statistics {
    pub(crate) fn new(config: StatisticsConfig) -> Self {
        Self {
            config,
            active: None,
            priorities: VecDeque::with_capacity(config.max_samples),
            latest_snapshot: None,
            next_window_id: 0,
        }
    }

    pub(crate) fn advance_to(&mut self, now: Timestamp, in_flight: usize) {
        let Some(window) = &self.active else {
            self.active = Some(ExecutionWindow::new(now, in_flight));
            return;
        };

        let elapsed = elapsed_ms(window.started_at, now);
        let ready = elapsed >= self.config.min_window_ms
            && window.successful_requests >= self.config.min_percentile_samples as u64;
        if ready || elapsed >= self.config.max_window_ms {
            self.close_window(now, in_flight);
        } else {
            self.observe_in_flight(in_flight);
        }
    }

    pub(crate) fn record_priority(&mut self, priority: Priority) {
        push_bounded(
            &mut self.priorities,
            u64::from(priority.value()),
            self.config.max_samples,
        );
    }

    pub(crate) fn record_success(&mut self, latency_ms: u64, now: Timestamp, in_flight: usize) {
        let window = self
            .active
            .as_mut()
            .expect("a timestamped operation must open a statistics window");
        window.successful_requests += 1;
        window.latency_total += u128::from(latency_ms);
        window.minimum_latency_ms = Some(
            window
                .minimum_latency_ms
                .map_or(latency_ms, |minimum| minimum.min(latency_ms)),
        );
        push_bounded(&mut window.latencies, latency_ms, self.config.max_samples);
        window.max_in_flight = window.max_in_flight.max(in_flight);

        let ready = elapsed_ms(window.started_at, now) >= self.config.min_window_ms
            && window.successful_requests >= self.config.min_percentile_samples as u64;
        if ready {
            self.close_window(now, in_flight);
        }
    }

    pub(crate) fn observe_in_flight(&mut self, in_flight: usize) {
        if let Some(window) = &mut self.active {
            window.max_in_flight = window.max_in_flight.max(in_flight);
        }
    }

    pub(crate) fn next_deadline(&self) -> Option<Timestamp> {
        self.active.as_ref().map(|window| {
            let duration =
                if window.successful_requests >= self.config.min_percentile_samples as u64 {
                    self.config.min_window_ms
                } else {
                    self.config.max_window_ms
                };
            Timestamp::from_millis(window.started_at.as_millis().saturating_add(duration))
        })
    }

    pub(crate) fn latest_snapshot(&self) -> Option<&StatisticsSnapshot> {
        self.latest_snapshot.as_ref()
    }

    pub(crate) fn priority_percentile(&self, percentile: u8) -> Option<u16> {
        if percentile > 100 || self.priorities.len() < self.config.min_samples {
            return None;
        }
        percentile_type_7(&self.priorities, percentile).map(|value| value as u16)
    }

    pub(crate) fn priority_threshold(&self, percentage: f64, maximum: u16) -> Option<u16> {
        if !percentage.is_finite()
            || !(0.0..=100.0).contains(&percentage)
            || self.priorities.len() < self.config.min_samples
        {
            return None;
        }
        if percentage <= 0.0 {
            return Some(0);
        }
        if percentage >= 100.0 {
            return Some(maximum);
        }

        let percentile = percentage.round() as u8;
        self.priority_percentile(percentile)
            .map(|priority| priority.saturating_add(1).min(maximum))
    }

    fn close_window(&mut self, now: Timestamp, in_flight: usize) {
        let window = self
            .active
            .take()
            .expect("only an active statistics window can be closed");
        let id = self.next_window_id;
        self.next_window_id = self.next_window_id.saturating_add(1);

        if window.successful_requests >= self.config.min_samples as u64 {
            let duration_ms = elapsed_ms(window.started_at, now);
            let percentile_latency_ms =
                percentile_type_7(&window.latencies, self.config.latency_percentile)
                    .expect("a publishable window must contain latency samples");
            let average_latency_ms =
                window.latency_total as f64 / window.successful_requests as f64;
            let throughput_per_second =
                window.successful_requests as f64 * 1_000.0 / duration_ms as f64;

            debug_assert!(average_latency_ms.is_finite());
            debug_assert!(throughput_per_second.is_finite());
            self.latest_snapshot = Some(StatisticsSnapshot {
                id,
                started_at: window.started_at,
                ended_at: now,
                percentile_latency_ms,
                minimum_latency_ms: window
                    .minimum_latency_ms
                    .expect("a publishable window must have a minimum latency"),
                average_latency_ms,
                successful_requests: window.successful_requests,
                throughput_per_second,
                max_in_flight: window.max_in_flight,
            });
        }

        self.active = Some(ExecutionWindow::new(now, in_flight));
    }
}

struct ExecutionWindow {
    started_at: Timestamp,
    latencies: VecDeque<u64>,
    latency_total: u128,
    minimum_latency_ms: Option<u64>,
    successful_requests: u64,
    max_in_flight: usize,
}

impl ExecutionWindow {
    fn new(started_at: Timestamp, in_flight: usize) -> Self {
        Self {
            started_at,
            latencies: VecDeque::new(),
            latency_total: 0,
            minimum_latency_ms: None,
            successful_requests: 0,
            max_in_flight: in_flight,
        }
    }
}

fn elapsed_ms(start: Timestamp, end: Timestamp) -> u64 {
    end.as_millis() - start.as_millis()
}

fn push_bounded(values: &mut VecDeque<u64>, value: u64, maximum: usize) {
    if values.len() == maximum {
        values.pop_front();
    }
    values.push_back(value);
}

fn percentile_type_7(values: &VecDeque<u64>, percentile: u8) -> Option<u64> {
    if values.is_empty() || percentile > 100 {
        return None;
    }

    let mut sorted: Vec<_> = values.iter().copied().collect();
    sorted.sort_unstable();
    let rank_numerator = (sorted.len() as u128 - 1) * u128::from(percentile);
    let lower_index = usize::try_from(rank_numerator / 100).expect("rank must fit in usize");
    let remainder = rank_numerator % 100;
    let lower = sorted[lower_index];
    let upper = sorted.get(lower_index + 1).copied().unwrap_or(lower);
    let interpolated = u128::from(lower) + u128::from(upper - lower) * remainder / 100;
    Some(u64::try_from(interpolated).expect("percentile must fit in u64"))
}

#[cfg(test)]
mod tests {
    use super::percentile_type_7;
    use std::collections::VecDeque;

    #[test]
    fn type_7_percentile_interpolates_and_truncates() {
        let values = VecDeque::from([10, 20, 30, 40, 50]);

        assert_eq!(percentile_type_7(&values, 0), Some(10));
        assert_eq!(percentile_type_7(&values, 50), Some(30));
        assert_eq!(percentile_type_7(&values, 90), Some(46));
        assert_eq!(percentile_type_7(&values, 100), Some(50));
    }

    #[test]
    fn type_7_percentile_does_not_depend_on_input_order() {
        let values = VecDeque::from([384, 256, 128]);

        assert_eq!(percentile_type_7(&values, 90), Some(358));
    }
}
