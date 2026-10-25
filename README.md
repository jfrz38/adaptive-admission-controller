# Adaptive Admission Controller

Adaptive Admission Controller is a portable admission-control engine written in Rust and designed for distribution through WebAssembly.

It protects services under load by combining priority-aware admission, bounded concurrency, queue eviction, runtime statistics, PID feedback, and automatic concurrency tuning. The core is independent of HTTP frameworks, gateway SDKs, asynchronous runtimes, and infrastructure concerns.

## Design principles

- **Portable core:** the same control algorithm can be embedded in different languages and gateways.
- **Host-driven execution:** the core decides what should run; the host owns requests, tasks, timers, and I/O.
- **Deterministic state:** time and execution outcomes are explicit inputs, making behavior reproducible and testable.
- **Minimal dependencies:** the algorithm targets zero external production dependencies; integration dependencies remain isolated in adapters.
- **Explicit scope:** controller state and limits apply to one instance or worker unless an external coordination mechanism is provided.

## Documentation

See [Architecture and project scope](docs/architecture.md) for:

- Goals and non-goals.
- The host/core responsibility boundary.
- The proposed state-machine API.
- Concurrency, timing, persistence, and portability constraints.
- WebAssembly and gateway integration strategy.
- Behavioral guarantees and implementation approach.

## Current status

The first six implementation iterations provide the `adaptive-admission-core` Rust crate with deterministic admission, bounded priority/FIFO queueing, concurrency enforcement, expiration, cancellation, shutdown, bounded runtime statistics, coherent host-driven execution windows, PID feedback that adapts the priority threshold, queue timeouts adapted from published execution latency, and automatic concurrency and target-latency tuning. The `adaptive-admission-component` crate exposes that state machine as a synchronous WebAssembly Component Model resource described with WIT. Host-language and gateway adapters remain subsequent increments.

## Development

The repository pins its Rust toolchain. Validate the workspace with:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets
cargo doc --workspace --no-deps
cargo build -p adaptive-admission-component --target wasm32-unknown-unknown --release
wasm-tools component new target/wasm32-unknown-unknown/release/adaptive_admission_component.wasm \
  -o target/wasm32-unknown-unknown/release/adaptive_admission_component.component.wasm
```

The final component artifact is written to
`target/wasm32-unknown-unknown/release/adaptive_admission_component.component.wasm`. The
componentization step requires `wasm-tools` 1.262.0, which can also inspect the exported contract
with `wasm-tools component wit <artifact>`. Building for the capability-free target avoids adding
ambient WASI clock, random, or I/O imports to the component.

## License

Distributed under the [MIT License](LICENSE).
