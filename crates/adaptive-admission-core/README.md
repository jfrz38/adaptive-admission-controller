# Adaptive Admission Core

`adaptive-admission-core` is the deterministic, host-independent state machine behind
[Adaptive Admission Controller](https://github.com/jfrz38/adaptive-admission-controller).

It combines priority-aware admission, bounded concurrency and queueing, runtime statistics, PID
feedback, adaptive queue timeouts, and automatic concurrency tuning. The crate does not execute
tasks, read clocks, create timers, or depend on an asynchronous runtime. Embedding applications own
those responsibilities and drive the controller with explicit timestamps and outcomes.

## Usage

```toml
[dependencies]
adaptive-admission-core = "=0.1.0"
```

```rust
use adaptive_admission_core::{Action, Config, Controller, Priority};

let mut controller = Controller::new(Config::default())?;
let submission = controller.submit(Priority::new(100)?, 1_000.into())?;

assert_eq!(
    submission.actions,
    vec![Action::Start {
        request_id: submission.request_id,
    }],
);

# Ok::<(), Box<dyn std::error::Error>>(())
```

The host must serialize calls to each controller, provide monotonic millisecond timestamps, execute
the returned actions in order, report outcomes, and schedule the next deadline. See the repository's
[host adapter contract](https://github.com/jfrz38/adaptive-admission-controller/blob/main/docs/host-adapters.md)
for the complete integration requirements.

## WebAssembly hosts

Hosts in other languages should consume the separately distributed WebAssembly component rather
than this native Rust crate. The component, WIT contract, and host conformance scenarios use the
same release version as this crate.

## License

Licensed under the MIT License.
