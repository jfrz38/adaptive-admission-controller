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

## License

Distributed under the [MIT License](LICENSE).
