use std::error::Error;
use std::fmt::{self, Display, Formatter};

use crate::{RequestId, Timestamp};

/// Invalid controller configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigError {
    ThresholdOutOfRange { value: u16 },
    ZeroConcurrency,
    ZeroQueueTimeout,
}

impl Display for ConfigError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::ThresholdOutOfRange { value } => {
                write!(formatter, "admission threshold {value} is outside 0..=768")
            }
            Self::ZeroConcurrency => write!(formatter, "max concurrency must be greater than zero"),
            Self::ZeroQueueTimeout => write!(formatter, "queue timeout must be greater than zero"),
        }
    }
}

impl Error for ConfigError {}

/// Invalid priority construction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PriorityError {
    ValueOutOfRange { value: u16 },
    TierOutOfRange { tier: u8 },
    CohortOutOfRange { cohort: u8 },
}

impl Display for PriorityError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::ValueOutOfRange { value } => {
                write!(formatter, "priority {value} is outside 0..=767")
            }
            Self::TierOutOfRange { tier } => {
                write!(formatter, "priority tier {tier} is outside 0..=5")
            }
            Self::CohortOutOfRange { cohort } => {
                write!(formatter, "priority cohort {cohort} is outside 0..=127")
            }
        }
    }
}

impl Error for PriorityError {}

/// Failure to apply an event to the current controller state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControllerError {
    TimeRegression {
        previous: Timestamp,
        received: Timestamp,
    },
    UnknownRequest {
        request_id: RequestId,
    },
    InvalidTransition {
        request_id: RequestId,
        operation: &'static str,
        state: &'static str,
    },
    RequestIdExhausted,
    SequenceExhausted,
    DeadlineOverflow,
}

impl Display for ControllerError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::TimeRegression { previous, received } => write!(
                formatter,
                "timestamp {} is earlier than the previous timestamp {}",
                received.as_millis(),
                previous.as_millis()
            ),
            Self::UnknownRequest { request_id } => {
                write!(
                    formatter,
                    "request {} is not active or queued",
                    request_id.value()
                )
            }
            Self::InvalidTransition {
                request_id,
                operation,
                state,
            } => write!(
                formatter,
                "cannot {operation} request {} while it is {state}",
                request_id.value()
            ),
            Self::RequestIdExhausted => write!(formatter, "request ID space is exhausted"),
            Self::SequenceExhausted => write!(formatter, "queue sequence space is exhausted"),
            Self::DeadlineOverflow => {
                write!(formatter, "queue deadline exceeds the timestamp range")
            }
        }
    }
}

impl Error for ControllerError {}
