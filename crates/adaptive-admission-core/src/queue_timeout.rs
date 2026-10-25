use crate::StatisticsSnapshot;

pub(crate) struct QueueTimeoutController {
    ratio: f64,
    last_snapshot_id: Option<u64>,
}

impl QueueTimeoutController {
    pub(crate) fn new(ratio: f64) -> Self {
        Self {
            ratio,
            last_snapshot_id: None,
        }
    }

    pub(crate) fn update(&mut self, snapshot: &StatisticsSnapshot) -> Option<u64> {
        if self.last_snapshot_id == Some(snapshot.id) {
            return None;
        }
        self.last_snapshot_id = Some(snapshot.id);

        let timeout = (snapshot.average_latency_ms * self.ratio).round();
        if !timeout.is_finite() || timeout < 1.0 {
            return None;
        }
        if timeout >= u64::MAX as f64 {
            return Some(u64::MAX);
        }
        Some(timeout as u64)
    }
}
