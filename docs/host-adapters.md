# Host adapter contract

This document specifies how a language runtime or gateway drives the synchronous state machine
exported by `adaptive-admission-component`. WIT defines the callable ABI and data shapes; it does not
own host tasks, asynchronous execution, clocks, timers, or framework lifecycle.

## Instance scope and serialization

Each exported `admission-controller` resource owns one independent controller. Queue, active count,
statistics, PID state, thresholds, and automatic tuning are not shared with other resources,
processes, or workers.

Calls into one resource must be serialized. A single-threaded event loop satisfies this only while
the adapter prevents callbacks started by an action from re-entering an in-progress component call.
Threaded hosts must provide equivalent external synchronization.

## Host-owned requests

After `submit`, the host retains a mapping equivalent to:

```text
request-id -> task, continuation, result channel, cancellation state
```

The mapping must be installed before processing the returned actions because the same submission can
immediately emit `start` or `reject`. The component stores request metadata, never the task or request
object itself.

The adapter must preserve action order and handle every action returned by `submit`, `complete`,
`cancel`, `advance-time`, and `shutdown`:

- `start`: schedule the associated host task outside the current component mutation.
- `reject`: do not execute the task; return a host error retaining the structured reason.
- `evict`: do not execute the task; return a queue-timeout error.
- `cancel`: do not execute queued work; return a cancellation or shutdown error.
- `threshold-changed` and `concurrency-changed`: optionally expose diagnostics, without making them
  necessary for correctness.

When started work succeeds, call `complete(id, success, now)` and return its original value. When it
fails, call `complete(id, failure, now)` and preserve the original host failure. Actions returned by
either completion must be processed before the adapter considers the operation drained.

## Time and wake-ups

Every timestamp must come from one monotonic millisecond clock. Wall-clock corrections must not make
timestamps regress. Hosts whose monotonic source has finer resolution must convert without using a
floating-point representation that can lose large integer values.

After every mutating operation, read `next-deadline`. Maintain at most one scheduled wake-up per
resource:

1. Cancel or invalidate the previous wake-up.
2. Schedule the new earliest deadline.
3. On wake-up, call `advance-time` with the current monotonic timestamp.
4. Process all returned actions.
5. Read and schedule `next-deadline` again.

Timer callbacks must carry a generation or equivalent token so a callback cancelled concurrently
cannot advance stale state. Calling `advance-time` later than requested is supported; the core does
not synthesize missed intervals.

## Shutdown and cancellation

`shutdown` rejects future submissions and emits `cancel` for queued work. Started work remains owned
by the host and may complete normally, so the adapter must keep the component resource alive until
those completions have been reported. A statistics deadline can remain while active work exists.

Host cancellation of queued work maps to `cancel`. Cancellation of already started work is a host
concern: the current component contract deliberately rejects that state transition and the host must
still report its eventual outcome if execution terminates.

## Capability levels

A full integration must be able to retain and later resume work, schedule deadlines, report outcomes,
and detect cancellation or shutdown. A platform exposing only a synchronous allow/reject hook cannot
reproduce priority queueing. It may use immediate admission with `max-queued-requests` set to zero,
but must document that reduced capability.

Language support alone is insufficient for a gateway integration. The gateway must also expose a
lifecycle hook with the capabilities above and a scope in which one stateful component instance can
be retained safely.

## Conformance

Every host SDK must execute the scenarios in `conformance/scenarios` against the final distributed
component, not a mock or a native reimplementation. It must additionally test language and runtime
boundaries such as fixed-width integer representation, generated error shapes, resource lifetime,
timer behavior, and package asset resolution.

Hosts may expose `latest-diagnostics` for operational tooling and simulations. The snapshot reports
the effective admission threshold, concurrency limit, adaptive queue timeout, target latency, and
filtered latency used by automatic tuning. Reading it never advances time or changes controller
state; applications must not depend on it for admission correctness.

Target-specific failures do not belong in shared scenarios when native targets legitimately differ.
For example, a `u64` collection bound can fit native 64-bit Rust but produce `numeric-overflow` when
converted to `usize` by the wasm32 component.
