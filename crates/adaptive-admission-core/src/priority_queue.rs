use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

use crate::types::Priority;
use crate::{ControllerError, RequestId, Timestamp};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct QueuedRequest {
    priority: Priority,
    sequence: u64,
    deadline: Timestamp,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SchedulingEntry {
    request_id: RequestId,
    priority: Priority,
    sequence: u64,
}

impl Ord for SchedulingEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .priority
            .cmp(&self.priority)
            .then_with(|| other.sequence.cmp(&self.sequence))
    }
}

impl PartialOrd for SchedulingEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DeadlineEntry {
    request_id: RequestId,
    deadline: Timestamp,
    sequence: u64,
}

impl Ord for DeadlineEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .deadline
            .cmp(&self.deadline)
            .then_with(|| other.sequence.cmp(&self.sequence))
    }
}

impl PartialOrd for DeadlineEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

pub(crate) struct PriorityQueue {
    requests: HashMap<RequestId, QueuedRequest>,
    scheduling: BinaryHeap<SchedulingEntry>,
    deadlines: BinaryHeap<DeadlineEntry>,
    max_queued_requests: usize,
    queue_timeout_ms: u64,
    next_sequence: u64,
}

impl PriorityQueue {
    pub(crate) fn new(max_queued_requests: usize, queue_timeout_ms: u64) -> Self {
        Self {
            requests: HashMap::with_capacity(max_queued_requests),
            scheduling: BinaryHeap::with_capacity(max_queued_requests),
            deadlines: BinaryHeap::with_capacity(max_queued_requests),
            max_queued_requests,
            queue_timeout_ms,
            next_sequence: 0,
        }
    }

    pub(crate) fn is_full(&self) -> bool {
        self.requests.len() >= self.max_queued_requests
    }

    pub(crate) fn contains(&self, request_id: RequestId) -> bool {
        self.requests.contains_key(&request_id)
    }

    pub(crate) fn push(
        &mut self,
        request_id: RequestId,
        priority: Priority,
        now: Timestamp,
    ) -> Result<(), ControllerError> {
        let sequence = self.next_sequence;
        let next_sequence = sequence
            .checked_add(1)
            .ok_or(ControllerError::SequenceExhausted)?;
        let deadline = now
            .as_millis()
            .checked_add(self.queue_timeout_ms)
            .map(Timestamp::from_millis)
            .ok_or(ControllerError::DeadlineOverflow)?;

        let request = QueuedRequest {
            priority,
            sequence,
            deadline,
        };
        self.requests.insert(request_id, request);
        self.scheduling.push(SchedulingEntry {
            request_id,
            priority,
            sequence,
        });
        self.deadlines.push(DeadlineEntry {
            request_id,
            deadline,
            sequence,
        });
        self.next_sequence = next_sequence;
        Ok(())
    }

    pub(crate) fn pop_next(&mut self) -> Option<RequestId> {
        while let Some(entry) = self.scheduling.pop() {
            if self.matches(entry.request_id, entry.sequence) {
                self.requests.remove(&entry.request_id);
                return Some(entry.request_id);
            }
        }
        None
    }

    pub(crate) fn cancel(&mut self, request_id: RequestId) -> bool {
        self.requests.remove(&request_id).is_some()
    }

    pub(crate) fn expire(&mut self, now: Timestamp) -> Vec<RequestId> {
        let mut expired = Vec::new();
        while self
            .deadlines
            .peek()
            .is_some_and(|entry| entry.deadline <= now)
        {
            let entry = self.deadlines.pop().expect("peeked deadline must exist");
            if self.matches(entry.request_id, entry.sequence) {
                self.requests.remove(&entry.request_id);
                expired.push(entry.request_id);
            }
        }
        expired
    }

    pub(crate) fn next_deadline(&mut self) -> Option<Timestamp> {
        while let Some(entry) = self.deadlines.peek() {
            if self.matches(entry.request_id, entry.sequence) {
                return Some(entry.deadline);
            }
            self.deadlines.pop();
        }
        None
    }

    pub(crate) fn drain_in_scheduling_order(&mut self) -> Vec<RequestId> {
        let mut request_ids = Vec::with_capacity(self.requests.len());
        while let Some(request_id) = self.pop_next() {
            request_ids.push(request_id);
        }
        request_ids
    }

    fn matches(&self, request_id: RequestId, sequence: u64) -> bool {
        self.requests
            .get(&request_id)
            .is_some_and(|request| request.sequence == sequence)
    }
}
