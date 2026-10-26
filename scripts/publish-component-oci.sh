#!/usr/bin/env sh
set -eu

component_path=${1:?component path is required}
reference=${2:?OCI reference is required}
expected_sha256=${3:?expected component SHA-256 is required}
source_commit=${4:?source commit is required}

: "${GH_TOKEN:?GH_TOKEN is required}"
: "${WKG_OCI_USERNAME:?WKG_OCI_USERNAME is required}"
: "${WKG_OCI_PASSWORD:?WKG_OCI_PASSWORD is required}"

case "$reference" in
    ghcr.io/jfrz38/*:*) ;;
    *)
        echo "unsupported OCI reference: $reference" >&2
        exit 1
        ;;
esac

if [ ! -f "$component_path" ]; then
    echo "component artifact not found: $component_path" >&2
    exit 1
fi

actual_sha256=$(sha256sum "$component_path" | awk '{print $1}')
if [ "$actual_sha256" != "$expected_sha256" ]; then
    echo "component checksum mismatch: expected $expected_sha256, found $actual_sha256" >&2
    exit 1
fi

path_and_tag=${reference#ghcr.io/jfrz38/}
package_name=${path_and_tag%:*}
tag=${path_and_tag##*:}
temporary_dir=$(mktemp -d)
trap 'rm -rf "$temporary_dir"' EXIT

versions_file="$temporary_dir/versions.json"
api_error="$temporary_dir/api-error"
if ! gh api \
    --paginate \
    --slurp \
    "/users/jfrz38/packages/container/$package_name/versions?per_page=100" \
    > "$versions_file" 2> "$api_error"; then
    if grep -Fq 'HTTP 404' "$api_error"; then
        printf '[]\n' > "$versions_file"
    else
        cat "$api_error" >&2
        exit 1
    fi
fi

matching_versions=$(jq --arg tag "$tag" '
    [ .[][] | select(.metadata.container.tags | index($tag)) ]
' "$versions_file")
matching_count=$(printf '%s' "$matching_versions" | jq 'length')

if [ "$matching_count" -gt 1 ]; then
    echo "multiple GHCR package versions carry tag $tag; refusing to choose one" >&2
    exit 1
fi

if [ "$matching_count" -eq 1 ]; then
    digest=$(printf '%s' "$matching_versions" | jq -r '.[0].name')
    case "$digest" in
        sha256:*) ;;
        *)
            echo "GHCR did not report a valid digest for $reference" >&2
            exit 1
            ;;
    esac
    wkg oci pull "ghcr.io/jfrz38/$package_name@$digest" --output "$temporary_dir/published.wasm"
    published_sha256=$(sha256sum "$temporary_dir/published.wasm" | awk '{print $1}')
    if [ "$published_sha256" != "$expected_sha256" ]; then
        echo "OCI tag $reference already points to different component bytes" >&2
        exit 1
    fi
    echo "$reference already publishes the expected component ($digest)."
    exit 0
fi

push_output=$(wkg oci push \
    --author "José F. Ruiz" \
    --annotation "org.opencontainers.image.source=https://github.com/jfrz38/adaptive-admission-controller" \
    --annotation "org.opencontainers.image.revision=$source_commit" \
    --annotation "org.opencontainers.image.version=$tag" \
    --annotation "org.opencontainers.image.licenses=MIT" \
    --annotation "org.opencontainers.image.description=Portable adaptive admission-control WebAssembly component" \
    "$reference" \
    "$component_path")
printf '%s\n' "$push_output"

digest=$(printf '%s\n' "$push_output" | sed -n 's/^digest: //p')
case "$digest" in
    sha256:*) ;;
    *)
        echo "wkg did not report a valid OCI digest" >&2
        exit 1
        ;;
esac

wkg oci pull "ghcr.io/jfrz38/$package_name@$digest" --output "$temporary_dir/published.wasm"
published_sha256=$(sha256sum "$temporary_dir/published.wasm" | awk '{print $1}')
if [ "$published_sha256" != "$expected_sha256" ]; then
    echo "published component checksum mismatch" >&2
    exit 1
fi

echo "Published and verified $reference at $digest."
