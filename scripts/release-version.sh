#!/usr/bin/env sh
set -eu

root_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root_dir"

version=$(awk '
    /^\[workspace\.package\]$/ { in_workspace_package = 1; next }
    /^\[/ { in_workspace_package = 0 }
    in_workspace_package && /^version = "/ {
        value = $0
        sub(/^version = "/, "", value)
        sub(/"$/, "", value)
        print value
        exit
    }
' Cargo.toml)

if [ -z "$version" ]; then
    echo "workspace.package.version is missing" >&2
    exit 1
fi

for manifest in \
    crates/adaptive-admission-core/Cargo.toml \
    crates/adaptive-admission-component/Cargo.toml
do
    if ! grep -q '^version\.workspace = true$' "$manifest"; then
        echo "$manifest must inherit workspace.package.version" >&2
        exit 1
    fi
done

expected_wit_package="package jfrz38:adaptive-admission@$version;"
if ! grep -Fqx "$expected_wit_package" crates/adaptive-admission-component/wit/world.wit; then
    echo "WIT package version must match workspace version $version" >&2
    exit 1
fi

cargo metadata --locked --no-deps --format-version 1 >/dev/null
printf '%s\n' "$version"
