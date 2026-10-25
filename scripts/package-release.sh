#!/usr/bin/env sh
set -eu

root_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
component_path=${1:?component path is required}
dist_dir=${2:-dist}

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
checksums="$dist_dir/SHA256SUMS"

cp "$component_path" "$component_asset"
cp crates/adaptive-admission-component/wit/world.wit "$wit_asset"
tar -czf "$conformance_asset" conformance

(
    cd "$dist_dir"
    sha256sum \
        "$(basename "$component_asset")" \
        "$(basename "$wit_asset")" \
        "$(basename "$conformance_asset")" > "$(basename "$checksums")"
)

printf '%s\n' "$version" > "$dist_dir/VERSION"
