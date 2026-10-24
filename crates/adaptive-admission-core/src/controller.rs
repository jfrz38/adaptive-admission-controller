use crate::admission::AdmissionController;
use crate::priority_queue::PriorityQueue;
use crate::scheduler::Scheduler;
use crate::{
    Action, CancellationReason, Config, ConfigError, ControllerError, EvictionReason, Outcome,
    Priority, RejectionReason, RequestId, Submission, Timestamp,
};

/// Deterministic admission-control state machine.
pub struct Controller {
    admission: AdmissionController,
    scheduler: Scheduler,
    queue: PriorityQueue,
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
        } else if let Some(reason) = self.admission.rejection_reason(priority) {
            actions.push(Action::Reject { request_id, reason });
        } else if self.scheduler.has_capacity() {
            self.scheduler.start(request_id);
            actions.push(Action::Start { request_id });
        } else if self.queue.is_full() {
            actions.push(Action::Reject {
                request_id,
                reason: RejectionReason::QueueFull,
            });
        } else {
            self.queue.push(request_id, priority, now)?;
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
        _outcome: Outcome,
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
        self.scheduler.complete(request_id);
        self.fill_capacity(&mut actions);
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
        self.queue.next_deadline()
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
        self.queue
            .expire(now)
            .into_iter()
            .map(|request_id| Action::Evict {
                request_id,
                reason: EvictionReason::QueueTimeout,
            })
            .collect()
    }

    fn reserve_request_id(&mut self) -> Result<RequestId, ControllerError> {
        let value = self.next_request_id;
        self.next_request_id = value
            .checked_add(1)
            .ok_or(ControllerError::RequestIdExhausted)?;
        Ok(RequestId::new(value))
    }

    fn fill_capacity(&mut self, actions: &mut Vec<Action>) {
        if self.shutdown {
            return;
        }
        while self.scheduler.has_capacity() {
            let Some(request_id) = self.queue.pop_next() else {
                break;
            };
            self.scheduler.start(request_id);
            actions.push(Action::Start { request_id });
        }
    }
}
