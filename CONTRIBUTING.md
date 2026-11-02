# Contributing to Adaptive Admission Controller

## Local setup

Install GNU Make and the Rust toolchain pinned in `rust-toolchain.toml`.

```bash
git clone https://github.com/jfrz38/adaptive-admission-controller.git
cd adaptive-admission-controller
make check
```

Building the WebAssembly component also requires the pinned `wasm-tools` release:

```bash
make install-wasm-tools
make component
```

Create focused branches from `develop` and use Conventional Commit messages. Keep platform-specific
HTTP, runtime, and gateway integration outside the portable core.

## Change expectations

- Preserve deterministic behavior: time, outcomes, and host actions remain explicit inputs and
  outputs.
- Keep the core independent of asynchronous runtimes, clocks, I/O, and gateway SDKs.
- Add or update tests and conformance scenarios when observable behavior changes.
- Keep the Rust API, WIT contract, and language-neutral scenarios aligned.
- Update the architecture, host-adapter, or release documentation when their contracts change.

Run the required checks before opening a pull request:

```bash
make check
make install-wasm-tools
make component
```

For packaging or publication changes, also run:

```bash
make crate-publish-check
make dist
```

## Pull requests

Open pull requests against `develop`, except release pull requests from `develop` to `main`. Explain
the observable effect and any compatibility implications. Do not commit credentials, local paths,
generated build output, or release artifacts.

See [architecture](docs/architecture.md), the [host adapter contract](docs/host-adapters.md), and the
[security policy](SECURITY.md) for the relevant project boundaries.
