use crate::admission::AdmissionController;
use crate::auto_tuning::AutoTuner;
use crate::pid::{PidController, PidObservation};
use crate::priority_queue::PriorityQueue;
use crate::queue_timeout::QueueTimeoutController;
use crate::scheduler::Scheduler;
use crate::statistics::Statistics;
use crate::{
    Action, CancellationReason, Config, ConfigError, ControllerError, EvictionReason, Outcome,
    Priority, RejectionReason, RequestId, StatisticsSnapshot, Submission, Timestamp,
};

/// Deterministic admission-control state machine.
pub struct Controller {
    admission: AdmissionController,
    scheduler: Scheduler,
    queue: PriorityQueue,
    queue_timeout: QueueTimeoutController,
    statistics: Statistics,
    pid: PidController,
    auto_tuner: AutoTuner,
    maximum_threshold: u16,
    next_request_id: u64,
    current_time: Option<Timestamp>,
    shutdown: bool,
}

impl Controller {
    /// Builds a controller after validating all configuration values.
    pub fn new(config: Config) -> Result<Self, ConfigError> {
        let config = config.validate()?;
        Ok(Self {
            admission: AdmissionController::new(config.initial_threshold),
            scheduler: Scheduler::new(config.max_concurrent_requests),
            queue: PriorityQueue::new(config.max_queued_requests, config.queue_timeout_ms),
            queue_timeout: QueueTimeoutController::new(config.queue_timeout_ratio),
            statistics: Statistics::new(config.statistics),
            pid: PidController::new(config.pid),
            auto_tuner: AutoTuner::new(),
            maximum_threshold: config.initial_threshold,
            next_request_id: 0,
            current_time: None,
            shutdown: false,
        })
    }

    /// Submits request metadata and returns its ID plus any immediate actions.
    pub fn submit(
        &mut self,
        priority: Priority,
        now: Timestamp,
    ) -> Result<Submission, ControllerError> {
        self.validate_time(now)?;
        let request_id = self.reserve_request_id()?;
        let mut actions = self.advance_to(now);

        if self.shutdown {
            actions.push(Action::Reject {
                request_id,
                reason: RejectionReason::Shutdown,
            });
        } else {
            self.statistics.record_priority(priority);
            if let Some(reason) = self.admission.rejection_reason(priority) {
                actions.push(Action::Reject { request_id, reason });
            } else if self.scheduler.has_capacity() {
                self.scheduler.start(request_id, now);
                self.statistics
                    .observe_in_flight(self.scheduler.active_count());
                actions.push(Action::Start { request_id });
            } else if self.queue.is_full() {
                actions.push(Action::Reject {
                    request_id,
                    reason: RejectionReason::QueueFull,
                });
            } else {
                self.queue.push(request_id, priority, now)?;
            }
        }

        Ok(Submission {
            request_id,
            actions,
        })
    }

    /// Reports the outcome of started host work and schedules waiting work when possible.
    pub fn complete(
        &mut self,
        request_id: RequestId,
        outcome: Outcome,
        now: Timestamp,
    ) -> Result<Vec<Action>, ControllerError> {
        self.validate_time(now)?;
        if !self.scheduler.is_active(request_id) {
            return if self.queue.contains(request_id) {
                Err(ControllerError::InvalidTransition {
                    request_id,
                    operation: "complete",
                    state: "queued",
                })
            } else {
                Err(ControllerError::UnknownRequest { request_id })
            };
        }

        let mut actions = self.advance_to(now);
        let started_at = self.scheduler.complete(request_id);
        if outcome == Outcome::Success {
            let latency_ms = now.as_millis() - started_at.as_millis();
            self.statistics
                .record_success(latency_ms, now, self.scheduler.active_count());
            self.update_queue_timeout();
        }
        self.fill_capacity(&mut actions, now);
        Ok(actions)
    }

    /// Cancels queued work. Work that has already started remains owned by the host.
    pub fn cancel(
        &mut self,
        request_id: RequestId,
        now: Timestamp,
    ) -> Result<Vec<Action>, ControllerError> {
        self.validate_time(now)?;
        if self.scheduler.is_active(request_id) {
            return Err(ControllerError::InvalidTransition {
                request_id,
                operation: "cancel",
                state: "started",
            });
        }
        if !self.queue.contains(request_id) {
            return Err(ControllerError::UnknownRequest { request_id });
        }

        let mut actions = self.advance_to(now);
        if self.queue.cancel(request_id) {
            actions.push(Action::Cancel {
                request_id,
                reason: CancellationReason::Requested,
            });
        }
        Ok(actions)
    }

    /// Advances the monotonic clock and evicts requests whose deadlines have elapsed.
    pub fn advance_time(&mut self, now: Timestamp) -> Result<Vec<Action>, ControllerError> {
        self.validate_time(now)?;
        Ok(self.advance_to(now))
    }

    /// Rejects future submissions and cancels queued work without interrupting active work.
    pub fn shutdown(&mut self, now: Timestamp) -> Result<Vec<Action>, ControllerError> {
        self.validate_time(now)?;
        let mut actions = self.advance_to(now);
        if !self.shutdown {
            self.shutdown = true;
            actions.extend(
                self.queue
                    .drain_in_scheduling_order()
                    .into_iter()
                    .map(|request_id| Action::Cancel {
                        request_id,
                        reason: CancellationReason::Shutdown,
                    }),
            );
        }
        Ok(actions)
    }

    /// Returns the next instant at which the host should call [`Self::advance_time`].
    pub fn next_deadline(&mut self) -> Option<Timestamp> {
        let queue_deadline = self.queue.next_deadline();
        let statistics_deadline = (!self.shutdown || self.scheduler.active_count() > 0)
            .then(|| self.statistics.next_deadline())
            .flatten();
        let pid_deadline = (!self.shutdown).then(|| self.pid.next_deadline()).flatten();
        let auto_tuning_deadline = (!self.shutdown)
            .then(|| self.auto_tuner.next_deadline())
            .flatten();
        [
            queue_deadline,
            statistics_deadline,
            pid_deadline,
            auto_tuning_deadline,
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// Returns the most recently published execution statistics, if any.
    pub fn latest_statistics(&self) -> Option<&StatisticsSnapshot> {
        self.statistics.latest_snapshot()
    }

    /// Calculates a cumulative percentile over recent submitted priorities.
    ///
    /// Returns `None` when the percentile is outside `0..=100` or too few requests have been
    /// observed.
    pub fn priority_percentile(&self, percentile: u8) -> Option<u16> {
        self.statistics.priority_percentile(percentile)
    }

    fn validate_time(&self, now: Timestamp) -> Result<(), ControllerError> {
        if let Some(previous) = self.current_time
            && now < previous
        {
            return Err(ControllerError::TimeRegression {
                previous,
                received: now,
            });
        }
        Ok(())
    }

    fn advance_to(&mut self, now: Timestamp) -> Vec<Action> {
        self.current_time = Some(now);
        self.statistics
            .advance_to(now, self.scheduler.active_count());
        self.update_queue_timeout();
        let mut actions = Vec::new();
        if !self.shutdown {
            let observation = PidObservation {
                active: self.scheduler.active_count(),
                capacity: self.scheduler.capacity(),
                queue_entries: 0,
                queue_exits: 0,
            };
            if self
                .pid
                .next_deadline()
                .is_some_and(|deadline| now >= deadline)
            {
                let (queue_entries, queue_exits) = self.queue.take_activity();
                let observation = PidObservation {
                    queue_entries,
                    queue_exits,
                    ..observation
                };
                if let Some(percentage) = self.pid.advance(now, observation) {
                    let threshold = self.threshold_for_percentage(percentage);
                    if let Some((previous, current)) = self.admission.update_threshold(threshold) {
                        actions.push(Action::ThresholdChanged { previous, current });
                    }
                }
            } else {
                self.pid.advance(now, observation);
            }
        }
        actions.extend(
            self.queue
                .expire(now)
                .into_iter()
                .map(|request_id| Action::Evict {
                    request_id,
                    reason: EvictionReason::QueueTimeout,
                }),
        );
        if !self.shutdown
            && let Some(current) = self.auto_tuner.advance(
                now,
                self.statistics.latest_snapshot(),
                self.scheduler.capacity(),
            )
        {
            let previous = self.scheduler.capacity();
            self.scheduler.update_capacity(current);
            actions.push(Action::ConcurrencyChanged { previous, current });
            self.fill_capacity(&mut actions, now);
        }
        actions
    }

    fn reserve_request_id(&mut self) -> Result<RequestId, ControllerError> {
        let value = self.next_request_id;
        self.next_request_id = value
            .checked_add(1)
            .ok_or(ControllerError::RequestIdExhausted)?;
        Ok(RequestId::new(value))
    }

    fn update_queue_timeout(&mut self) {
        let Some(snapshot) = self.statistics.latest_snapshot() else {
            return;
        };
        if let Some(timeout_ms) = self.queue_timeout.update(snapshot) {
            self.queue.update_timeout(timeout_ms);
        }
    }

    fn fill_capacity(&mut self, actions: &mut Vec<Action>, now: Timestamp) {
        if self.shutdown {
            return;
        }
        while self.scheduler.has_capacity() {
            let Some(request_id) = self.queue.pop_next() else {
                break;
            };
            self.scheduler.start(request_id, now);
            self.statistics
                .observe_in_flight(self.scheduler.active_count());
            actions.push(Action::Start { request_id });
        }
    }

    fn threshold_for_percentage(&self, percentage: f64) -> u16 {
        self.statistics
            .priority_threshold(percentage, self.maximum_threshold)
            .unwrap_or_else(|| {
                ((percentage * f64::from(self.maximum_threshold) / 100.0).round() as u16)
                    .min(self.maximum_threshold)
            })
    }
}
