# Architecture and project scope

## Purpose

Traditional rate limiters usually accept or reject traffic according to a fixed quota. Adaptive
Admission Controller instead regulates whether work may start according to the observed condition of
a service. It combines:

- Priority-aware admission and rejection.
- Bounded priority queueing.
- A concurrency limit for active work.
- Execution statistics over coherent time windows.
- PID feedback for the admission threshold.
- Automatic queue-timeout, concurrency, and target-latency tuning.

The design evolves the approach explored in
[`rate-limit-pid-controller`](https://github.com/jfrz38/rate-limit-pid-controller), inspired by
Uber's [Cinnamon load-shedding architecture](https://www.uber.com/en-ES/blog/cinnamon-using-century-old-tech-to-build-a-mean-load-shedder/).
The core boundary is independent of Node.js, HTTP frameworks, gateway SDKs, and asynchronous
runtimes so the same control algorithm can be embedded in different hosts.

## System boundary

The controller is a synchronous state machine. It owns decisions and control state, while its host
owns the work and all platform resources.

### Core responsibilities

- Validate controller configuration.
- Track request state and enforce legal transitions.
- Apply the current admission threshold.
- Select queued work by priority and FIFO order.
- Enforce the current concurrency limit.
- Expire queued requests.
- Aggregate execution statistics.
- Update PID and automatic-tuning state.
- Return explicit actions for the host to perform.

### Host responsibilities

- Associate controller request IDs with real requests, tasks, or continuations.
- Serialize calls into each controller instance.
- Execute work selected by the controller.
- Report completion, failure, and cancellation.
- Supply monotonic timestamps.
- Schedule the next controller deadline.
- Translate decisions into framework- or gateway-specific behavior.
- Own logging, metrics, shutdown, and lifecycle integration.

The controller never owns an HTTP request, invokes application work, reads a clock, or creates a
timer. When work waits, the host retains the real request and the controller retains only its
metadata.

## Execution model

One controller exposes the following state-machine operations:

```text
Controller::new(config)

submit(priority, now_ms)                  -> request ID + actions
complete(request_id, outcome, now_ms)     -> actions
cancel(request_id, now_ms)                -> actions
advance_time(now_ms)                      -> actions
shutdown(now_ms)                          -> actions
next_deadline()                           -> optional timestamp
```

Actions are decisions, not host operations:

```text
Start(request_id)
Reject(request_id, reason)
Evict(request_id, reason)
Cancel(request_id, reason)
ThresholdChanged(previous, current)
ConcurrencyChanged(previous, current)
```

For example, an adapter stores `request_id -> host task`, executes the task after receiving `Start`,
and calls `complete` when it settles. A queued submission may return no immediate action; the host
must retain it until a later operation returns `Start`, `Evict`, or `Cancel`.

The WebAssembly component mirrors these operations through the WIT contract in
[`crates/adaptive-admission-component/wit/world.wit`](../crates/adaptive-admission-component/wit/world.wit).
Language-specific promises, futures, request objects, and errors remain outside that contract.

## Request lifecycle

Each request follows an explicit lifecycle:

```text
Created  -> Queued | Started | Rejected
Queued   -> Started | Evicted | Cancelled
Started  -> Completed | Failed
```

Invalid transitions return errors. Shutdown cancels queued work and rejects future submissions;
work already started remains owned by the host and may complete normally.

## Admission, priority, and queueing

Priorities are values in `0..=767`, with lower values representing more important work. Hosts may
construct them as six tiers with 128 cohorts per tier. Cohorts let a host distribute admission
consistently within a tier without requiring a random source in the core.

The effective threshold is exclusive: a request is rejected when its priority is at or above that
threshold. A threshold of `768` admits every valid priority. The configured initial threshold also
acts as the maximum threshold the adaptive controller may use.

If a request passes admission and capacity is available, it starts immediately. Otherwise it enters
a bounded queue. The queue selects lower priority values first and preserves FIFO order for equal
priorities. When the queue is full, a new submission is rejected rather than displacing existing
work.

Queue deadlines are inclusive: a queued request expires when `deadline <= now`. Every timestamped
operation processes elapsed time before applying its requested transition. Expiration therefore wins
when cancellation or newly available capacity occurs at exactly the deadline.

## Time and concurrency

The core does not assume that WebAssembly or another host provides clocks, timers, threads, or an
event loop:

- Every timestamp is supplied by the host as monotonic `u64` milliseconds.
- A timestamp that moves backwards is rejected.
- `next_deadline` reports the earliest queue, statistics, PID, or tuning deadline.
- The host calls `advance_time` when that deadline is reached.
- A late wake-up performs the due update from current observations; it does not synthesize missed
  intervals.

Calls into one controller must be serialized. The controller tracks active work but does not execute
it concurrently. A native binding may add a mutex, and an event-loop host may use single ownership,
without adding synchronization to the portable algorithm.

Controller state belongs to one resource, process, or gateway worker. Separate instances have
independent queues, statistics, thresholds, and concurrency limits. Process-wide or cluster-wide
limits require external coordination.

## Statistics and adaptive control

The controller records bounded samples from successful executions and publishes coherent statistics
windows containing latency, throughput, and observed in-flight work. Execution latency is measured
from `Start` until a successful `complete`; queue waiting time is excluded. Failed work releases
capacity but does not contribute a latency or throughput sample.

Statistics windows close when their duration and sample conditions are met, or when their maximum
duration expires. Undersized windows are discarded rather than replacing the latest usable
snapshot. Sample collections and histories have explicit bounds.

Three mechanisms consume these observations:

1. **Admission PID:** queue activity, active work, and free capacity adjust an admission percentage.
   Recent priority percentiles convert that percentage into the effective threshold.
2. **Adaptive queue timeout:** newly published average latency updates the timeout assigned to later
   queue entries. Existing entries keep their original deadline.
3. **Automatic tuning:** filtered percentile latency, minimum latency, throughput, and observed
   in-flight work gradually adjust concurrency and target latency. Reducing concurrency never
   interrupts active work, so active work may temporarily exceed a newly reduced limit; increasing
   concurrency can immediately start queued work.

The current operational threshold, concurrency limit, queue timeout, target latency, and filtered
latency are available as diagnostics. Diagnostics are observational and are not required for correct
admission behavior.

## Determinism and numerical safety

Time, priorities, outcomes, and cancellations are explicit inputs. The same valid input sequence
therefore produces the same actions and state changes.

The implementation uses bounded collections and deterministic percentile interpolation. Invalid
configuration, integer overflow at public boundaries, and non-finite values that could corrupt
persistent control state are rejected. The Rust core uses no external production dependencies;
binding generation and component tooling remain isolated in the WebAssembly crate and build process.

## Persistence and observability

Controller state is in memory, including queued metadata, statistics, PID state, tuning history, the
admission threshold, and the concurrency limit. Restarting an instance discards that state.
Persistence or restoration, if required by an embedding, is a host responsibility and must never
attempt to persist live requests or continuations.

The core does not depend on a logger or telemetry SDK. A host can translate returned actions,
statistics, and diagnostics into logs, metrics, or traces. Observability callbacks must not re-enter
the controller during a state mutation.

## Portability

The repository separates two layers:

```text
adaptive-admission-core        Host-independent Rust state machine
adaptive-admission-component   WIT / WebAssembly Component Model interface
```

The component is built for `wasm32-unknown-unknown` and imports no ambient WASI clock, randomness,
I/O, callbacks, or logging. Each exported `admission-controller` resource owns one independent core
controller.

No WebAssembly ABI works in every gateway. A gateway may support the Component Model, Proxy-Wasm, a
proprietary SDK, or only a synchronous admission hook. ABI-specific code belongs in an adapter, not
in the core. A full adapter must verify that its platform can pause and resume requests, schedule
timers, report cancellation, retain state at a suitable scope, and handle reload and shutdown.

See the [host adapter contract](host-adapters.md) for the complete embedding requirements.

## Non-goals

The portable core does not:

- Execute callbacks, promises, futures, or application work.
- Include an asynchronous runtime or HTTP integration.
- Manage threads, sockets, process signals, clocks, timers, or random sources.
- Provide persistence or distributed coordination.
- Enforce a global limit across independent instances.
- Guarantee that one WebAssembly artifact can be installed unchanged in every gateway ABI.

## Behavioral guarantees

The implementation and shared conformance scenarios preserve these properties:

- Lower priority values receive precedence.
- Equal priorities retain FIFO order.
- Requests only perform legal state transitions.
- The controller does not start additional work while active work is at or above the current
  concurrency limit.
- Queues and sample collections are bounded.
- Queued work can expire without executing host work.
- Shutdown cancels pending work without interrupting active work.
- Statistics describe coherent execution periods.
- Non-finite numerical values do not enter persistent control state.
- The same input event sequence produces the same output actions.

Language and gateway hosts validate the observable contract with the JSON fixtures documented in
[`conformance/README.md`](../conformance/README.md). Target-specific representation, resource,
timer, and packaging behavior remains the responsibility of each host implementation.
