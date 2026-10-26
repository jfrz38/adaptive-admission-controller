#!/usr/bin/env sh
set -eu

: "${GH_TOKEN:?GH_TOKEN is required}"
: "${GITHUB_REPOSITORY:?GITHUB_REPOSITORY is required}"

error_file=$(mktemp)
trap 'rm -f "$error_file"' EXIT

if tag=$(gh api "repos/$GITHUB_REPOSITORY/releases/latest" --jq '.tag_name' 2> "$error_file"); then
    if ! printf '%s\n' "$tag" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+$'; then
        echo "latest stable release tag is not v<SemVer>: $tag" >&2
        exit 1
    fi
    printf '%s\n' "$tag"
elif grep -Fq 'HTTP 404' "$error_file"; then
    gh api --silent "repos/$GITHUB_REPOSITORY"
    printf '\n'
else
    cat "$error_file" >&2
    exit 1
fi
