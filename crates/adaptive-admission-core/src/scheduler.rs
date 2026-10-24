use std::collections::HashSet;

use crate::RequestId;

pub(crate) struct Scheduler {
    active: HashSet<RequestId>,
    max_concurrent_requests: usize,
}

impl Scheduler {
    pub(crate) fn new(max_concurrent_requests: usize) -> Self {
        Self {
            active: HashSet::with_capacity(max_concurrent_requests),
            max_concurrent_requests,
        }
    }

    pub(crate) fn has_capacity(&self) -> bool {
        self.active.len() < self.max_concurrent_requests
    }

    pub(crate) fn is_active(&self, request_id: RequestId) -> bool {
        self.active.contains(&request_id)
    }

    pub(crate) fn start(&mut self, request_id: RequestId) {
        debug_assert!(self.has_capacity());
        self.active.insert(request_id);
    }

    pub(crate) fn complete(&mut self, request_id: RequestId) {
        let removed = self.active.remove(&request_id);
        debug_assert!(removed);
    }
}
