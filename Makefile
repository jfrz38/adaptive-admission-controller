.DEFAULT_GOAL := help

WASM_TOOLS_VERSION ?= 1.262.0
WKG_VERSION ?= 0.16.1
WASM_TARGET ?= wasm32-unknown-unknown
DIST_DIR ?= dist
COMPONENT_MODULE := target/$(WASM_TARGET)/release/adaptive_admission_component.wasm
COMPONENT_ARTIFACT := target/$(WASM_TARGET)/release/adaptive_admission_component.component.wasm

.PHONY: help
help: ## Show available targets
	@awk 'BEGIN {FS = ":.*?## "} /^[a-zA-Z_-]+:.*?## / {sub("\\\\n",sprintf("\n%22c"," "), $$2);printf " \033[36m%-20s\033[0m  %s\n", $$1, $$2}' $(MAKEFILE_LIST)

.PHONY: fmt fmt-check lint test doc crate-package-check crate-publish-check check
fmt: ## Format Rust sources
	cargo fmt --all

fmt-check: ## Check Rust source formatting
	cargo fmt --all --check

lint: ## Run Clippy with warnings denied
	cargo clippy --workspace --all-targets --all-features --locked -- -D warnings

test: ## Run all workspace tests
	cargo test --workspace --all-targets --locked

doc: ## Build workspace documentation with warnings denied
	RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked

crate-package-check: ## Package and verify the public Rust crate without publishing it
	cargo package --locked --allow-dirty -p adaptive-admission-core

crate-publish-check: ## Run the crates.io publication checks without uploading the crate
	cargo publish --dry-run --locked --allow-dirty -p adaptive-admission-core

check: fmt-check lint test doc crate-package-check ## Run all required Rust checks

.PHONY: version-check sync-version install-wasm-tools wasm-tools-check install-wkg wkg-check component dist
version-check: ## Verify that Cargo and WIT versions agree
	@sh scripts/release-version.sh

sync-version: ## Synchronize WIT and Cargo.lock with the workspace version
	@sh scripts/sync-wit-version.sh
	cargo check --workspace
	@$(MAKE) version-check

install-wasm-tools: ## Install the pinned wasm-tools release
	@actual_version="$$(wasm-tools --version 2>/dev/null || true)"; \
	expected_version="wasm-tools $(WASM_TOOLS_VERSION)"; \
	if [ "$$actual_version" = "$$expected_version" ]; then \
		echo "$$expected_version is already installed"; \
	else \
		cargo install wasm-tools --version "=$(WASM_TOOLS_VERSION)" --locked; \
	fi

wasm-tools-check:
	@actual_version="$$(wasm-tools --version 2>/dev/null || true)"; \
	expected_version="wasm-tools $(WASM_TOOLS_VERSION)"; \
	if [ "$$actual_version" != "$$expected_version" ]; then \
		echo "expected $$expected_version, found $${actual_version:-not installed}" >&2; \
		echo "run 'make install-wasm-tools' before building the component" >&2; \
		exit 1; \
	fi

install-wkg: ## Install the pinned WebAssembly package registry client
	@actual_version="$$(wkg --version 2>/dev/null || true)"; \
	expected_version="wkg $(WKG_VERSION)"; \
	if [ "$$actual_version" = "$$expected_version" ]; then \
		echo "$$expected_version is already installed"; \
	else \
		cargo install wkg --version "=$(WKG_VERSION)" --locked; \
	fi

wkg-check:
	@actual_version="$$(wkg --version 2>/dev/null || true)"; \
	expected_version="wkg $(WKG_VERSION)"; \
	if [ "$$actual_version" != "$$expected_version" ]; then \
		echo "expected $$expected_version, found $${actual_version:-not installed}" >&2; \
		echo "run 'make install-wkg' before publishing the component" >&2; \
		exit 1; \
	fi

component: version-check wasm-tools-check ## Build and validate the WebAssembly component
	cargo build --locked -p adaptive-admission-component --target "$(WASM_TARGET)" --release
	wasm-tools component new "$(COMPONENT_MODULE)" -o "$(COMPONENT_ARTIFACT)"
	wasm-tools validate "$(COMPONENT_ARTIFACT)"
	@version="$$(sh scripts/release-version.sh)"; \
	wasm-tools component targets \
		crates/adaptive-admission-component/wit/world.wit \
		"$(COMPONENT_ARTIFACT)" \
		--world "jfrz38:adaptive-admission/adaptive-admission@$$version"

dist: component ## Build versioned release assets in DIST_DIR
	@sh scripts/package-release.sh "$(COMPONENT_ARTIFACT)" "$(DIST_DIR)" "$(WASM_TARGET)"
