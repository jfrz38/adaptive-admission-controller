# Release process

GitHub Releases distribute the portable artifacts consumed by external language packages and gateway adapters. This repository does not publish those adapters or publish its crates to crates.io.

## Version source

`workspace.package.version` in the root `Cargo.toml` is the release version. Both crates inherit it, and the package declaration in `crates/adaptive-admission-component/wit/world.wit` must use the same version.

Update both sources in a pull request before merging a new release to `main`. `make version-check` rejects mismatches, and release automation refuses to overwrite an existing `v<version>` tag.

## Validation

Pull requests targeting `develop` or `main` run:

- `make check` for formatting, Clippy, tests, and documentation.
- `make component` for the release-target build, component conversion, and component validation.

Pull requests targeting `main` additionally verify that the proposed version has no existing tag. Branch protection should require these checks before merge.

To reproduce the complete validation locally:

```bash
make check
make install-wasm-tools
make component
```

## Publication

Every push to `main` is treated as release intent. The release workflow:

1. Runs all required Rust checks.
2. Builds and validates the distribution with read-only repository permissions.
3. Transfers those exact assets to an isolated publication job.
4. Creates `v<version>` on the validated commit and publishes the GitHub Release.

The publication job is the only job with `contents: write`. Assets are uploaded to a draft and verified before it becomes public. If asset upload is interrupted, rerunning the same commit resumes that draft; published releases and unrelated tags are never overwritten.

Each release contains:

- `adaptive-admission-component-v<version>.wasm`: validated WebAssembly component.
- `adaptive-admission-world-v<version>.wit`: source WIT contract.
- `adaptive-admission-conformance-v<version>.tar.gz`: conformance documentation, schema, and scenarios.
- `SHA256SUMS`: SHA-256 digests for all three assets.

Build the same asset set locally with `make dist`. Generated files are written to `dist/`, which is ignored by Git.
