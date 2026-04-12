#!/bin/sh
# Build a .deb package for kominka-repo.
# Usage: ./build-deb.sh [target]
# target: aarch64-unknown-linux-gnu or x86_64-unknown-linux-gnu
# With no target, builds both aarch64 and x86_64.
#
# Requires: cross (cargo install cross) + Docker, for cross-compilation.
# ring and other C-dependent crates cannot be cross-compiled with plain cargo.

set -eu

cd "$(dirname "$0")/../server"

VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)"/\1/')
PKG="kominka-repo_${VERSION}"

build_deb() {
    TARGET="$1"
    cross build --release --target "$TARGET"
    BIN="target/${TARGET}/release/kominka-repo"

    STAGE=$(mktemp -d)
    trap 'rm -rf "$STAGE"' EXIT

    mkdir -p "$STAGE/usr/bin"
    mkdir -p "$STAGE/lib/systemd/system"
    mkdir -p "$STAGE/etc/kominka-repo"
    mkdir -p "$STAGE/DEBIAN"

    cp "$BIN" "$STAGE/usr/bin/kominka-repo"
    cp kominka-repo.service "$STAGE/lib/systemd/system/"
    cp kominka-repo.env.example "$STAGE/etc/kominka-repo/env.example"

    case "$TARGET" in
        aarch64-*) DEB_ARCH=arm64 ;;
        x86_64-*)  DEB_ARCH=amd64 ;;
        *)         DEB_ARCH="$TARGET" ;;
    esac

    cat > "$STAGE/DEBIAN/control" <<EOF
Package: kominka-repo
Version: ${VERSION}
Section: net
Priority: optional
Architecture: ${DEB_ARCH}
Maintainer: Josh <josh@kominka.org>
Description: Kominka package repository server
 A thin HTTP server backed by S3-compatible storage for hosting
 Kominka Linux packages with content-addressed storage.
EOF

    cat > "$STAGE/DEBIAN/postinst" <<'EOF'
#!/bin/sh
set -e
if ! getent group kominka-repo >/dev/null; then
    groupadd --system kominka-repo
fi
if ! getent passwd kominka-repo >/dev/null; then
    useradd --system --gid kominka-repo --home-dir /var/lib/kominka-repo \
        --shell /usr/sbin/nologin kominka-repo
fi
mkdir -p /var/lib/kominka-repo
chown kominka-repo:kominka-repo /var/lib/kominka-repo

if [ ! -f /etc/kominka-repo/env ]; then
    cp /etc/kominka-repo/env.example /etc/kominka-repo/env
    chmod 600 /etc/kominka-repo/env
fi
EOF
    chmod 755 "$STAGE/DEBIAN/postinst"

    dpkg-deb --build "$STAGE" "${PKG}_${DEB_ARCH}.deb"
    echo "Built ${PKG}_${DEB_ARCH}.deb"
}

if [ -n "${1:-}" ]; then
    build_deb "$1"
else
    build_deb aarch64-unknown-linux-gnu
    build_deb x86_64-unknown-linux-gnu
fi
