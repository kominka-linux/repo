#!/bin/sh
# Rebuild the package index from existing tarballs on the repo server.
# Uses PKGBUILDs for metadata (deps, mkdeps). Fetches sha256 from S3 server-side.
#
# Usage: ./scripts/reindex.sh [packages_dir]
# Env: KOMINKA_REPO, KOMINKA_TOKEN

set -eu

PACKAGES_DIR="${1:-$(dirname "$0")/../packages}"
REPO_URL="${KOMINKA_REPO:?KOMINKA_REPO must be set}"
TOKEN="${KOMINKA_TOKEN:?KOMINKA_TOKEN must be set}"
YSH="${YSH:-/usr/local/bin/ysh}"

for arch in aarch64-linux-gnu x86_64-linux-gnu; do
    for dir in "$PACKAGES_DIR"/*/; do
        [ -f "$dir/PKGBUILD.ysh" ] || continue
        pkg="$(basename "$dir")"

        meta=$("$YSH" -c "
            source $dir/PKGBUILD.ysh
            json write ({name: name, ver: ver, rel: rel, deps: deps, mkdeps: mkdeps, sources: sources})
        " 2>/dev/null) || { printf 'SKIP %s (parse error)\n' "$pkg"; continue; }

        ver=$(printf '%s' "$meta" | python3 -c "import json,sys; e=json.load(sys.stdin); print(e['ver'])")
        rel=$(printf '%s' "$meta" | python3 -c "import json,sys; e=json.load(sys.stdin); print(e['rel'])")
        nsrc=$(printf '%s' "$meta" | python3 -c "import json,sys; e=json.load(sys.stdin); print(len(e['sources']))")
        deps_json=$(printf '%s' "$meta" | python3 -c "import json,sys; e=json.load(sys.stdin); print(json.dumps(e['deps']))")
        mkdeps_json=$(printf '%s' "$meta" | python3 -c "import json,sys; e=json.load(sys.stdin); print(json.dumps(e['mkdeps']))")

        if [ "$nsrc" = "0" ]; then
            # Metapackage — use /api/publish
            printf 'PUBLISH %s/%s %s-%s\n' "$arch" "$pkg" "$ver" "$rel"
            curl -sf -X POST "$REPO_URL/api/publish" \
                -H "Authorization: Bearer $TOKEN" \
                -H "Content-Type: application/json" \
                -d "{\"arch\":\"$arch\",\"pkg\":\"$pkg\",\"ver\":\"$ver\",\"rel\":\"$rel\",\"deps\":$deps_json,\"mkdeps\":$mkdeps_json}" \
                || printf '  FAILED\n'
        else
            # Regular package — check if tarball exists, then reindex
            code=$(curl -so /dev/null -w '%{http_code}' \
                "$REPO_URL/$arch/$pkg/$ver-$rel.tar.gz")
            if [ "$code" = "200" ]; then
                printf 'REINDEX %s/%s %s-%s\n' "$arch" "$pkg" "$ver" "$rel"
                curl -sf -X POST "$REPO_URL/api/reindex" \
                    -H "Authorization: Bearer $TOKEN" \
                    -H "Content-Type: application/json" \
                    -d "{\"arch\":\"$arch\",\"pkg\":\"$pkg\",\"ver\":\"$ver\",\"rel\":\"$rel\",\"deps\":$deps_json,\"mkdeps\":$mkdeps_json}" \
                    || printf '  FAILED\n'
            else
                printf 'SKIP %s/%s (no tarball on server)\n' "$arch" "$pkg"
            fi
        fi
    done
done

printf 'Done.\n'
