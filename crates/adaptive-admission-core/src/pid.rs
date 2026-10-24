use crate::{PidConfig, Timestamp};

#[derive(Clone, Copy)]
pub(crate) struct PidObservation {
    pub(crate) active: usize,
    pub(crate) capacity: usize,
    pub(crate) queue_entries: usize,
    pub(crate) queue_exits: usize,
}

pub(crate) struct PidController {
    config: PidConfig,
    percentage: f64,
    integral: f64,
    previous_error: f64,
    next_deadline: Option<Timestamp>,
}

impl PidController {
    pub(crate) fn new(config: PidConfig) -> Self {
        Self {
            config,
            percentage: 100.0,
            integral: 0.0,
            previous_error: 0.0,
            next_deadline: None,
        }
    }

    pub(crate) fn next_deadline(&self) -> Option<Timestamp> {
        self.next_deadline
    }

    /// Returns the current admission percentage when an update was due.
    pub(crate) fn advance(&mut self, now: Timestamp, observation: PidObservation) -> Option<f64> {
        let Some(deadline) = self.next_deadline else {
            self.schedule_from(now);
            return None;
        };
        if now < deadline {
            return None;
        }

        // A delayed host wake-up is one observation, not a series of invented empty intervals.
        self.schedule_from(now);
        self.update(observation);
        Some(self.percentage)
    }

    fn schedule_from(&mut self, now: Timestamp) {
        self.next_deadline = Some(Timestamp::from_millis(
            now.as_millis().saturating_add(self.config.interval_ms),
        ));
    }

    fn update(&mut self, observation: PidObservation) {
        let free_capacity = observation.capacity.saturating_sub(observation.active);
        let normalizer = if observation.queue_exits > 0 {
            observation.queue_exits
        } else {
            observation.capacity
        };
        let error = (observation.queue_entries as f64
            - observation.queue_exits as f64
            - free_capacity as f64)
            / normalizer as f64;

        if error > 0.0 && error < 0.01 {
            self.previous_error = error;
            return;
        }

        let dt = self.config.interval_ms as f64 / 1_000.0;
        let next_integral = if error <= 0.0 {
            self.integral * self.config.integral_decay + error * dt
        } else {
            self.integral + error * dt
        };
        let derivative = (error - self.previous_error) / dt;
        let signed_output = self.config.proportional_gain * error
            + self.config.integral_gain * next_integral
            + self.config.derivative_gain * derivative;
        let delta = signed_output.abs().min(self.config.max_delta_percentage);
        let candidate = if error <= 0.0 {
            self.percentage + delta
        } else {
            self.percentage - delta
        };

        if !error.is_finite()
            || !next_integral.is_finite()
            || !derivative.is_finite()
            || !signed_output.is_finite()
            || !candidate.is_finite()
        {
            return;
        }

        self.previous_error = error;
        if (0.0..=100.0).contains(&candidate) {
            self.integral = next_integral;
        }
        self.percentage = candidate.clamp(0.0, 100.0);
    }
}

#[cfg(test)]
mod tests {
    use super::{PidController, PidObservation};
    use crate::{PidConfig, Timestamp};

    fn config() -> PidConfig {
        PidConfig {
            proportional_gain: 10.0,
            integral_gain: 0.0,
            derivative_gain: 0.0,
            interval_ms: 1_000,
            max_delta_percentage: 20.0,
            integral_decay: 0.5,
        }
    }

    #[test]
    fn overload_reduces_percentage_by_at_most_the_configured_delta() {
        let mut pid = PidController::new(config());
        assert_eq!(pid.advance(0.into(), observation(1, 1, 0, 0)), None);

        assert_eq!(
            pid.advance(1_000.into(), observation(1, 1, 100, 0)),
            Some(80.0)
        );
    }

    #[test]
    fn free_capacity_recovers_and_clamps_the_percentage() {
        let mut pid = PidController::new(config());
        pid.advance(0.into(), observation(1, 1, 0, 0));
        pid.advance(1_000.into(), observation(1, 1, 2, 0));

        assert_eq!(
            pid.advance(2_000.into(), observation(0, 1, 0, 0)),
            Some(90.0)
        );
        assert_eq!(
            pid.advance(3_000.into(), observation(0, 1, 0, 0)),
            Some(100.0)
        );
    }

    #[test]
    fn delayed_wakeup_schedules_one_new_interval_from_observation_time() {
        let mut pid = PidController::new(config());
        pid.advance(0.into(), observation(1, 1, 0, 0));

        pid.advance(5_000.into(), observation(1, 1, 2, 0));

        assert_eq!(pid.next_deadline(), Some(Timestamp::from_millis(6_000)));
    }

    #[test]
    fn recovery_decays_the_integral_term() {
        let mut pid = PidController::new(PidConfig {
            proportional_gain: 0.0,
            integral_gain: 10.0,
            ..config()
        });
        pid.advance(0.into(), observation(1, 1, 0, 0));
        pid.advance(1_000.into(), observation(1, 1, 1, 0));
        assert_eq!(pid.integral, 1.0);

        pid.advance(2_000.into(), observation(0, 1, 0, 0));

        assert_eq!(pid.integral, -0.5);
    }

    #[test]
    fn saturation_does_not_accumulate_integral_windup() {
        let mut pid = PidController::new(PidConfig {
            proportional_gain: 100.0,
            integral_gain: 1.0,
            max_delta_percentage: 100.0,
            ..config()
        });
        pid.percentage = 50.0;
        pid.advance(0.into(), observation(1, 1, 0, 0));

        assert_eq!(
            pid.advance(1_000.into(), observation(1, 1, 2, 0)),
            Some(0.0)
        );
        assert_eq!(pid.integral, 0.0);
    }

    #[test]
    fn non_finite_calculation_does_not_corrupt_persistent_state() {
        let mut pid = PidController::new(PidConfig {
            proportional_gain: f64::MAX,
            max_delta_percentage: 100.0,
            ..config()
        });
        pid.advance(0.into(), observation(1, 1, 0, 0));

        assert_eq!(
            pid.advance(1_000.into(), observation(1, 1, 2, 0)),
            Some(100.0)
        );
        assert_eq!(pid.percentage, 100.0);
        assert_eq!(pid.integral, 0.0);
        assert_eq!(pid.previous_error, 0.0);
    }

    fn observation(
        active: usize,
        capacity: usize,
        queue_entries: usize,
        queue_exits: usize,
    ) -> PidObservation {
        PidObservation {
            active,
            capacity,
            queue_entries,
            queue_exits,
        }
    }
}
