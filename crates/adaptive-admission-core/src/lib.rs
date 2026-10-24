#![forbid(unsafe_code)]
//! Host-independent adaptive admission-control state machine.
//!
//! The crate owns controller state and returns [`Action`] values for the embedding host to
//! perform. It does not execute tasks, read clocks, or create timers.
//!
//! ```
//! use adaptive_admission_core::{Action, Config, Controller, Priority};
//!
//! let mut controller = Controller::new(Config::default())?;
//! let submission = controller.submit(Priority::new(100)?, 1_000.into())?;
//! assert_eq!(
//!     submission.actions,
//!     vec![Action::Start { request_id: submission.request_id }],
//! );
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod admission;
mod controller;
mod error;
mod pid;
mod priority_queue;
mod scheduler;
mod statistics;
mod types;

pub use controller::Controller;
pub use error::{ConfigError, ControllerError, PriorityError};
pub use types::{
    Action, CancellationReason, Config, EvictionReason, Outcome, PidConfig, Priority,
    RejectionReason, RequestId, StatisticsConfig, StatisticsSnapshot, Submission, Timestamp,
};
