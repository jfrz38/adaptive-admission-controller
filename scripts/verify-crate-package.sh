#!/usr/bin/env bash
set -euo pipefail

version=$(sh scripts/release-version.sh)
package="adaptive-admission-core-${version}"
archive="target/package/${package}.crate"

if ! cmp -s LICENSE crates/adaptive-admission-core/LICENSE; then
    echo "crate license does not match the repository license" >&2
    exit 1
fi

if [ ! -f "${archive}" ]; then
    echo "crate package not found: ${archive}" >&2
    exit 1
fi

expected=$(cat <<EOF
.cargo_vcs_info.json
Cargo.lock
Cargo.toml
Cargo.toml.orig
LICENSE
README.md
src/admission.rs
src/auto_tuning.rs
src/controller.rs
src/error.rs
src/lib.rs
src/pid.rs
src/priority_queue.rs
src/queue_timeout.rs
src/scheduler.rs
src/statistics.rs
src/types.rs
EOF
)

actual=$(
    tar -tf "${archive}" |
        sed "s#^${package}/##" |
        sed '/^$/d' |
        sort
)

expected=$(printf '%s\n' "${expected}" | sort)

if [ "${actual}" != "${expected}" ]; then
    echo "crate package contents do not match the expected public package:" >&2
    diff -u \
        <(printf '%s\n' "${expected}") \
        <(printf '%s\n' "${actual}") >&2 || true
    exit 1
fi

echo "verified ${archive}"
