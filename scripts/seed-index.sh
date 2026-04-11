#!/bin/sh
# Generate packages.json from PKGBUILDs and upload via the repo server API.
#
# Requires: ysh, curl, jq
# Environment: KOMINKA_REPO, KOMINKA_TOKEN
#
# Usage: ./seed-index.sh [packages_dir]

set -eu

PACKAGES_DIR="${1:-$(dirname "$0")/../packages}"
REPO_URL="${KOMINKA_REPO:?KOMINKA_REPO must be set}"
TOKEN="${KOMINKA_TOKEN:?KOMINKA_TOKEN must be set}"
YSH="${YSH:-ysh}"

for dir in "$PACKAGES_DIR"/*/; do
    [ -f "$dir/PKGBUILD.ysh" ] || continue
    pkg="$(basename "$dir")"

    # Use ysh to parse the PKGBUILD and emit JSON metadata.
    meta=$("$YSH" -c "
        source $dir/PKGBUILD.ysh
        json write ({name: name, ver: ver, rel: rel, deps: deps})
    " 2>/dev/null) || { echo "SKIP $pkg (parse error)"; continue; }

    ver=$(echo "$meta" | jq -r .ver)
    rel=$(echo "$meta" | jq -r .rel)
    deps=$(echo "$meta" | jq -r '[.deps[]] | join(",")')
    hash=$(shasum -a 256 "$dir/PKGBUILD.ysh" | cut -d' ' -f1)

    # Check if this package has sources (non-metapackage).
    has_sources=$("$YSH" -c "
        source $dir/PKGBUILD.ysh
        write -- \$(len(sources))
    " 2>/dev/null) || has_sources="0"

    if [ "$has_sources" = "0" ]; then
        # Metapackage: publish metadata only.
        for arch in aarch64-linux-gnu x86_64-linux-gnu; do
            printf 'PUBLISH %s/%s\n' "$arch" "$pkg"
            curl -sf -X POST "$REPO_URL/api/publish" \
                -H "Authorization: Bearer $TOKEN" \
                -H "Content-Type: application/json" \
                -d "{\"arch\":\"$arch\",\"pkg\":\"$pkg\",\"ver\":\"$ver\",\"rel\":\"$rel\",\"hash\":\"$hash\",\"deps\":[$(echo "$deps" | sed 's/,/","/g;s/^/"/;s/$/"/' | sed 's/""//' )]}" \
                || echo "  FAILED"
        done
    else
        printf 'INDEX-ONLY %s (has sources, no tarball upload)\n' "$pkg"
    fi
done

echo "Done. Packages with sources need to be built and uploaded via 'pm p'."
