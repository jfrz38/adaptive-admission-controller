use std::collections::HashMap;

use crate::{RequestId, Timestamp};

pub(crate) struct Scheduler {
    active: HashMap<RequestId, Timestamp>,
    max_concurrent_requests: usize,
}

impl Scheduler {
    pub(crate) fn new(max_concurrent_requests: usize) -> Self {
        Self {
            active: HashMap::with_capacity(max_concurrent_requests),
            max_concurrent_requests,
        }
    }

    pub(crate) fn has_capacity(&self) -> bool {
        self.active.len() < self.max_concurrent_requests
    }

    pub(crate) fn is_active(&self, request_id: RequestId) -> bool {
        self.active.contains_key(&request_id)
    }

    pub(crate) fn start(&mut self, request_id: RequestId, started_at: Timestamp) {
        debug_assert!(self.has_capacity());
        self.active.insert(request_id, started_at);
    }

    pub(crate) fn complete(&mut self, request_id: RequestId) -> Timestamp {
        self.active
            .remove(&request_id)
            .expect("active request must have a start timestamp")
    }

    pub(crate) fn active_count(&self) -> usize {
        self.active.len()
    }
}
