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

wit_file=crates/adaptive-admission-component/wit/world.wit
temporary_file=$(mktemp)
trap 'rm -f "$temporary_file"' EXIT

if ! awk -v package="package jfrz38:adaptive-admission@$version;" '
    BEGIN { declarations = 0 }
    /^package jfrz38:adaptive-admission@[0-9]+\.[0-9]+\.[0-9]+;$/ {
        print package
        declarations += 1
        next
    }
    { print }
    END {
        if (declarations != 1) {
            exit 1
        }
    }
' "$wit_file" > "$temporary_file"; then
    echo "$wit_file must contain exactly one simple SemVer package declaration" >&2
    exit 1
fi

if cmp -s "$temporary_file" "$wit_file"; then
    rm -f "$temporary_file"
else
    mv "$temporary_file" "$wit_file"
fi
trap - EXIT
