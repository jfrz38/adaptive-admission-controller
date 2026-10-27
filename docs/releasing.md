# Release process

GitHub Releases distribute the portable artifacts consumed by external language packages and
gateway adapters. A separate manual workflow promotes a validated release component to the GitHub
Container Registry (GHCR) and publishes the native Rust core to crates.io. This repository does not
publish host adapters.

## Version source

`workspace.package.version` in the root `Cargo.toml` is the release version. Both crates inherit it, and the package declaration in `crates/adaptive-admission-component/wit/world.wit` must use the same version.

Run the `Bump Version` workflow and choose a patch, minor, or major increment to prepare a release. It uses `auto-version-bump-action` to update the workspace version, synchronize WIT and `Cargo.lock`, and open a draft pull request against `develop`. Marking that pull request ready for review triggers the normal CI checks. The repository setting **Allow GitHub Actions to create and approve pull requests** must be enabled.

For a manual change, update `workspace.package.version` and run `make sync-version`. `make version-check` rejects mismatches, and release automation refuses to overwrite an existing `v<version>` tag.

## Validation

Pull requests targeting `develop` or `main` run:

- `make check` for formatting, Clippy, tests, and documentation.
- `make component` for the release-target build, component conversion, and component validation.

Pull requests targeting `main` additionally use `check-version-change` to compare against the latest
published release and verify that the proposed version has no existing tag or release. When no
published release exists, the current workspace version is accepted as the initial version. Later
releases must use a higher SemVer version. Branch protection should require these checks before
merge.

To reproduce the complete validation locally:

```bash
make check
make install-wasm-tools
make component
```

## Publication

Every push to `main` is treated as release intent. The release workflow:

1. Runs all required Rust checks and verifies that the version increased from the latest published release, when one exists.
2. Builds and validates the distribution with read-only repository permissions.
3. Transfers those exact assets to an isolated publication job.
4. Creates `v<version>` on the validated commit and publishes the GitHub Release.

The publication job is the only job with `contents: write`. Assets are uploaded to a draft and verified before it becomes public. If asset upload is interrupted, rerunning the same commit resumes that draft; published releases and unrelated tags are never overwritten.

Each release contains:

- `adaptive-admission-component-v<version>.wasm`: validated WebAssembly component.
- `adaptive-admission-world-v<version>.wit`: source WIT contract.
- `adaptive-admission-conformance-v<version>.tar.gz`: conformance documentation, schema, and scenarios.
- `adaptive-admission-release-v<version>.json`: machine-readable source, toolchain, artifact,
  checksum, Rust crate, and expected OCI reference metadata.
- `SHA256SUMS`: SHA-256 digests for the component, WIT, conformance archive, and release manifest.

Build the same asset set locally with `make dist`. Generated files are written to `dist/`, which is ignored by Git.

## WebAssembly component registry

Run the `Publish Release` workflow from `main` and select `wasm-oci` to promote the component from
the latest stable GitHub Release to:

```text
ghcr.io/jfrz38/adaptive-admission:<version>
```

The workflow verifies that the release tag belongs to `main`, validates every release checksum,
checks the source commit and clean-tree marker in the release manifest, and uses
`check-version-change` to resolve the exact GHCR tag and any existing manifest digest. It transfers
those exact assets to a job with `packages: write`; the privileged job never rebuilds the component.

Publication is idempotent. If the version tag already exists with the expected component bytes, the
workflow succeeds without moving it. It fails if the tag resolves to different bytes. A newly
created GHCR package is private by default; change its visibility to public from the package settings
only when the repository is ready for public distribution. Public packages cannot be made private
again.

Consumers can fetch a published component by version:

```bash
wkg oci pull ghcr.io/jfrz38/adaptive-admission:0.1.0 \
  --output adaptive-admission-component.wasm
```

For a fully immutable reference, use the digest reported by the publication workflow:

```bash
wkg oci pull ghcr.io/jfrz38/adaptive-admission@sha256:<digest> \
  --output adaptive-admission-component.wasm
```

## Rust crate registry

The same `Publish Release` workflow can publish `adaptive-admission-core` to crates.io. Select
`rust-crate` for only the native crate or `all` to publish both registry artifacts. The workflow:

1. Checks out the validated commit identified by the latest stable GitHub Release.
2. Uses `check-version-change` to resolve the inherited workspace version and compare it with
   crates.io.
3. Runs `cargo publish --dry-run` against the tagged source.
4. Exchanges the job's GitHub OIDC identity for a short-lived crates.io token.
5. Publishes only `adaptive-admission-core`; the component crate is not publishable.

If the exact crate version is already present, repeating the workflow succeeds without publishing
again. It rejects a release older than the latest non-yanked crates.io version. Published crate
versions cannot be replaced; yank a defective version and prepare a new release instead.

Before the first publication, configure a pending trusted publisher for
`adaptive-admission-core` in crates.io with:

- GitHub owner: `jfrz38`
- Repository: `adaptive-admission-controller`
- Workflow: `publish.yml`
- Environment: `crates-publish`

Also create and protect the matching `crates-publish` GitHub environment. No long-lived crates.io
token is stored in the repository.

Rust consumers can pin a published version with:

```toml
[dependencies]
adaptive-admission-core = "=0.1.0"
```
