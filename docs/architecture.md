# Architecture and project scope

## Purpose

Traditional rate limiters usually accept or reject traffic according to a fixed quota. Adaptive Admission Controller instead adjusts admission to the observed condition of a service. It is intended to:

- Reduce load as a service approaches saturation.
- Protect higher-priority requests before lower-priority work.
- Regulate concurrent executions.
- Adjust admission thresholds through PID feedback.
- Learn from observed latency and throughput.
- Share one control algorithm across languages and gateways.

The project builds on lessons from [`rate-limit-pid-controller`](https://github.com/jfrz38/rate-limit-pid-controller), but it is not a literal translation of its TypeScript implementation. The core boundary is designed so that it does not depend on Node.js, an HTTP framework, or a particular asynchronous runtime.

## Goals

The project provides a Rust implementation of the common control logic:

- Priorities and cohorts.
- Admission and rejection.
- Priority queueing and timeout eviction.
- Concurrency limits.
- Statistics over coherent execution windows.
- PID control.
- Automatic latency and concurrency tuning.

The core is intended to compile to WebAssembly and remain deterministic, synchronous, and independent of infrastructure. Its behavior should be verifiable through shared conformance scenarios.

The algorithm crate should use no external production dependencies unless doing so would materially improve correctness, safety, or maintainability. Platform SDKs and binding generators may be used by integration crates when necessary.

## Non-goals

The portable core does not:

- Execute host-language callbacks, promises, or futures inside WebAssembly.
- Include an asynchronous runtime.
- Directly manage threads, sockets, process signals, or HTTP requests.
- Provide persistence or distributed coordination.
- Guarantee that one WebAssembly binary can be installed unchanged in every gateway.
- Guarantee global limits across processes, workers, or nodes without external coordination.

## Responsibility boundary

The core acts as a state machine. It owns admission decisions and control state, while the host owns actual work and platform resources.

### Core responsibilities

- Validate controller configuration.
- Track request state and legal transitions.
- Select work according to priority and FIFO order.
- Enforce the current concurrency limit.
- Expire queued requests.
- Aggregate execution statistics.
- Update PID and auto-tuning state.
- Return explicit actions for the host to perform.

### Host responsibilities

- Associate controller request IDs with real requests, callbacks, or continuations.
- Execute work selected by the core.
- Report completion or failure.
- Provide monotonic timestamps.
- Schedule the next controller deadline.
- Translate controller decisions into framework- or gateway-specific behavior.
- Integrate logging, metrics, shutdown, and lifecycle events.

## Execution model

A conceptual API has the following shape:

```text
Controller::new(config)

submit(priority, now_ms)                  -> Actions
complete(request_id, outcome, now_ms)     -> Actions
advance_time(now_ms)                      -> Actions
shutdown(now_ms)                          -> Actions
next_deadline()                           -> Option<Timestamp>
```

Returned actions represent decisions rather than host operations:

```text
Start(request_id)
Reject(request_id, reason)
Evict(request_id, reason)
Cancel(request_id)
ThresholdChanged(value)
ConcurrencyChanged(value)
```

For example, an adapter stores `request_id -> host task`, executes a task after receiving `Start`, and calls `complete` when that task settles. This avoids passing language-specific closures or return values through the WebAssembly boundary.

The exact public API is a design subject, but the separation between decision and execution is an architectural invariant.

## Time and timers

WebAssembly does not universally provide timers, a monotonic clock, an event loop, or threads. The controller therefore does not read time directly:

- The host supplies monotonic timestamps.
- The core calculates its next deadline.
- The host schedules the corresponding wake-up.
- Tests can use a fake clock and remain deterministic.

This model also avoids creating one host timer for every queued request. A deadline heap can process expirations efficiently when the host advances time.

Timestamps supplied to one controller instance must never move backwards. The final API must define whether a regressing timestamp is rejected or clamped.

## Concurrency model

The core does not execute concurrent work. It tracks:

- The current capacity limit.
- The number of active requests.
- Waiting requests.
- Priority ordering.
- Work that should start when capacity becomes available.

Calls into one controller instance must be serialized. A native binding can add external synchronization, such as a mutex or single-owner event loop, without adding synchronization to the portable algorithm.

Controller state normally belongs to one WebAssembly instance or gateway worker. If a gateway creates several instances, each one has an independent queue, PID state, statistics, and concurrency limit. A process-wide or cluster-wide limit requires shared memory or an external coordinator and is outside the portable core.

## Request lifecycle

The controller maintains explicit request states. The expected lifecycle is:

```text
Created  -> Queued | Rejected
Queued   -> Started | Evicted | Cancelled
Started  -> Completed | Failed
```

Invalid transitions must be rejected. Shutdown cancels queued work and rejects future submissions, while work already started remains owned by the host and may finish normally.

## Priority and queueing

Lower numerical values represent more important requests. Requests with equal priority retain FIFO ordering through a monotonic sequence number.

A zero-dependency implementation can use:

- A priority `BinaryHeap` for selecting work.
- A deadline `BinaryHeap` for expirations.
- A `HashMap<RequestId, RequestState>` for authoritative state.
- Lazy removal of stale heap entries.

This avoids requiring arbitrary deletion from Rust's standard `BinaryHeap` and keeps insertion, selection, and expiration logarithmic in normal operation.

Queue memory must be explicitly bounded. A deployment should be able to configure a maximum number of queued requests and a policy for submissions that exceed it. Although WebAssembly stores only controller metadata, the host may retain HTTP requests, buffers, and framework state for every queued ID.

## Statistics and control

The core maintains bounded samples and coherent execution buckets for:

- Successful request latency.
- Minimum, average, and percentile latency.
- Successful throughput.
- Observed in-flight work.
- Priority distribution.

The implementation must specify the percentile algorithm, rounding behavior, floating-point edge cases, and sample-window semantics. These details are part of the behavioral contract because different valid quantile definitions can produce different admission decisions.

The PID controller uses observed queue and execution activity to adjust an admission percentage. An auto-tuner uses latency and throughput feedback to update concurrency and target latency. Invalid or non-finite values must not be allowed to corrupt controller state.

## Randomness

Rust's standard library does not provide a portable random source suitable for every WebAssembly host. The core should not implicitly depend on host randomness.

A cohort can instead be:

- Supplied explicitly by the adapter.
- Derived deterministically from request or client data.
- Generated through an optional host capability.

Deterministic cohort assignment is often preferable because it provides stable admission behavior for the same client or request class.

## Persistence

Controller state is in memory:

- Queued request metadata.
- Statistics and execution windows.
- PID integral and previous error.
- Admission threshold.
- Concurrency limit.
- Auto-tuning history.

Restarting an instance discards this state. If state restoration is introduced, storage remains a host responsibility. Portable snapshots may contain control state, but never live callbacks, requests, or host continuations.

## Logging and observability

The core must not depend on a concrete logger. It can expose structured controller events that an adapter translates into logs, metrics, or traces.

Returning or draining events is preferable to invoking a host logging callback during state mutation because host callbacks can introduce re-entrancy. Diagnostic events must not be required for correct controller behavior.

## Portability layers

The implementation may be divided into focused crates:

```text
adaptive-admission-core        Host-independent Rust algorithm
adaptive-admission-component   WIT / WebAssembly Component Model interface
adaptive-admission-proxy-wasm  Proxy-Wasm integration
adaptive-admission-cabi        Optional C ABI for native bindings
```

The core can rely on standard Rust types such as `BinaryHeap`, `HashMap`, `Vec`, and `VecDeque`, with small local implementations of mean, percentile, and covariance.

Starting with `std` keeps the implementation and bindings straightforward. A move to `no_std + alloc` should only be considered when required by a supported host.

### WebAssembly interfaces

No single WebAssembly ABI is universally supported by gateways:

- Some hosts use Proxy-Wasm.
- Some support WASI or the WebAssembly Component Model.
- Native applications may prefer a C ABI or language-specific embedding API.
- Other gateways expose proprietary SDKs.

The algorithm can remain common even when the integration artifacts differ. ABI-specific code belongs outside the core crate.

### Gateway capabilities

Reproducing priority queueing requires the gateway to pause and later resume a request. A host that only exposes a synchronous admission hook can support immediate `Allow | Reject` decisions but cannot reproduce queued execution safely.

Every gateway adapter must define:

- Whether requests can be paused and resumed.
- Whether plugin instances are per request, worker, process, or service.
- How timers are scheduled.
- How cancellation and client disconnects are reported.
- What memory limits apply to the module and retained host state.
- How module reload and shutdown affect queued and active requests.

## Behavioral guarantees

The implementation should preserve these properties:

- Lower priority values receive precedence.
- Equal priorities retain FIFO order.
- Requests only perform legal state transitions.
- The controller never starts work beyond its current concurrency limit.
- Queued requests can expire without executing host work.
- Shutdown cancels pending work without interrupting work already started.
- Statistics buckets describe coherent execution periods.
- Sample collections and queues have explicit bounds.
- Non-finite numerical values do not enter persistent control state.
- The same input event sequence produces the same output actions.

## Conformance strategy

Behavior should be specified as input traces and expected actions rather than tests coupled to internal classes. Scenarios should cover:

- Admission and rejection.
- Priority and FIFO ordering.
- Queue limits and timeout eviction.
- Completion, failure, cancellation, and shutdown.
- PID calculations and threshold changes.
- Percentile and rounding behavior.
- Statistics snapshots.
- Concurrency and latency tuning.
- Invalid transitions and non-monotonic time.

The same fixtures can be executed against the existing TypeScript implementation and the Rust core to identify intentional and accidental differences.

## Implementation sequence

A low-risk implementation sequence is:

1. Define behavioral traces and numerical compatibility fixtures.
2. Implement the host-independent Rust state machine.
3. Run cross-implementation conformance tests.
4. Compile the stable core to WebAssembly.
5. Build a Node.js adapter that reconstructs an idiomatic Promise-based API.
6. Validate one concrete gateway, including pause/resume, timers, worker scope, memory, reload, and shutdown.
7. Add integrations only after distinguishing common guarantees from host-specific behavior.

This sequence validates the architecture before committing to a gateway ABI and keeps portability concerns out of the control algorithm.
