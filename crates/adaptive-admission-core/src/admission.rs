use crate::{Priority, RejectionReason};

pub(crate) struct AdmissionController {
    threshold: u16,
}

impl AdmissionController {
    pub(crate) const fn new(threshold: u16) -> Self {
        Self { threshold }
    }

    pub(crate) fn rejection_reason(&self, priority: Priority) -> Option<RejectionReason> {
        (priority.value() >= self.threshold).then_some(RejectionReason::Threshold {
            priority,
            threshold: self.threshold,
        })
    }
}
