#!/usr/bin/env sh
set -eu

root_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
component_path=${1:?component path is required}
dist_dir=${2:-dist}
component_target=${3:-wasm32-unknown-unknown}

cd "$root_dir"
version=$(sh scripts/release-version.sh)

if [ ! -f "$component_path" ]; then
    echo "component artifact not found: $component_path" >&2
    exit 1
fi

mkdir -p "$dist_dir"

component_asset="$dist_dir/adaptive-admission-component-v$version.wasm"
wit_asset="$dist_dir/adaptive-admission-world-v$version.wit"
conformance_asset="$dist_dir/adaptive-admission-conformance-v$version.tar.gz"
manifest_asset="$dist_dir/adaptive-admission-release-v$version.json"
checksums="$dist_dir/SHA256SUMS"

cp "$component_path" "$component_asset"
cp crates/adaptive-admission-component/wit/world.wit "$wit_asset"
tar -czf "$conformance_asset" conformance

component_sha256=$(sha256sum "$component_asset" | awk '{print $1}')
wit_sha256=$(sha256sum "$wit_asset" | awk '{print $1}')
conformance_sha256=$(sha256sum "$conformance_asset" | awk '{print $1}')
source_commit=$(git rev-parse HEAD)
if [ -n "$(git status --porcelain)" ]; then
    source_tree_dirty=true
else
    source_tree_dirty=false
fi
rust_version=$(rustc --version)
wasm_tools_version=$(wasm-tools --version)

cat > "$manifest_asset" <<EOF
{
  "schemaVersion": 1,
  "version": "$version",
  "tag": "v$version",
  "source": {
    "repository": "https://github.com/jfrz38/adaptive-admission-controller",
    "commit": "$source_commit",
    "treeDirty": $source_tree_dirty
  },
  "rustCrate": {
    "name": "adaptive-admission-core",
    "version": "$version"
  },
  "component": {
    "package": "jfrz38:adaptive-admission@$version",
    "world": "adaptive-admission",
    "target": "$component_target",
    "artifact": "$(basename "$component_asset")",
    "sha256": "$component_sha256",
    "ociReference": "ghcr.io/jfrz38/adaptive-admission:$version"
  },
  "wit": {
    "artifact": "$(basename "$wit_asset")",
    "sha256": "$wit_sha256"
  },
  "conformance": {
    "artifact": "$(basename "$conformance_asset")",
    "sha256": "$conformance_sha256"
  },
  "toolchain": {
    "rust": "$rust_version",
    "wasmTools": "$wasm_tools_version"
  }
}
EOF

(
    cd "$dist_dir"
    sha256sum \
        "$(basename "$component_asset")" \
        "$(basename "$wit_asset")" \
        "$(basename "$conformance_asset")" \
        "$(basename "$manifest_asset")" > "$(basename "$checksums")"
)

printf '%s\n' "$version" > "$dist_dir/VERSION"
