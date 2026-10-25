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

    pub(crate) fn update_threshold(&mut self, threshold: u16) -> Option<(u16, u16)> {
        if threshold == self.threshold {
            return None;
        }
        let previous = self.threshold;
        self.threshold = threshold;
        Some((previous, threshold))
    }
}
