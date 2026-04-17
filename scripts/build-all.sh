#!/bin/bash
# Build core + build-essential packages in topological dependency order.
# Run from the repo root: scripts/build-all.sh
# Requires: kominka:builder image already built (make builder).
#
# Each package's stats are flushed to .cache/runs/TIMESTAMP.jsonl immediately
# after it finishes, so data survives interruptions.
#
# To set a new baseline after a clean run:
#   make baseline
set -euo pipefail

CACHE="${CACHE:-$(pwd)/.cache}"
BASELINE="${BASELINE:-$(pwd)/scripts/build-baseline.jsonl}"
RUN_DIR="$CACHE/runs"
RUN_FILE="$RUN_DIR/$(date +%Y%m%d-%H%M%S).jsonl"

mkdir -p "$RUN_DIR"
echo "Run file: $RUN_FILE"

# Find the most recent existing run file so we can skip already-successful packages.
PREV_RUN=$(ls -t "$RUN_DIR"/*.jsonl 2>/dev/null | grep -v "$(basename "$RUN_FILE")" | head -1)

# When set, only build this one package (all others are silently skipped).
ONLY_PKG="${ONLY_PKG:-}"

# Topological waves — each wave depends only on packages in prior waves.
#   mkdeps drive the ordering, not just runtime deps.
#
#   Wave 1: no deps
#   Wave 2: needs zig
#   Wave 3: needs zig + musl
#   Wave 4: needs make (+ musl/zig)
#   Wave 5: needs m4 (bison) / cmake+samurai (zlib)
#   Wave 6: needs bison+binutils (linux) / zlib+make (dropbear)
#   Wave 7: needs zlib + dropbear (git)
WAVES=(
    "baselayout ca-certificates seed baseinit zig"
    "musl"
    "make mimalloc muon"
    "m4 binutils samurai cmake pkgconf runit ysh"
    "bison zlib"
    "linux dropbear"
    "git"
)

ALL_PKGS=()

SEED="${SEED:-$HOME/d/seed/target/aarch64-unknown-linux-musl/debug/seed}"
SEED_MOUNT=""
# The published tarball has applets as hardlinks; mounting /usr/bin/seed alone
# does not override them. Mount every applet path derived from applet_list.rs.
# Once the builder image is rebuilt with the symlink-based seed package, this
# reduces to a single mount for /usr/bin/seed.
if [[ -f "$SEED" ]]; then
    while IFS= read -r _applet; do
        SEED_MOUNT+=" -v $SEED:/usr/bin/$_applet:ro"
    done < <(grep -E 'name: "[^"]+"' "$HOME/d/seed/src/applet_list.rs" \
                 | sed 's/.*name: "\([^"]*\)".*/\1/' | sort -u)
fi

update_checksums() {
    local pkg="$1"
    # Run inside Docker so seed/ysh are available.
    # packages is mounted rw so PKGBUILD.ysh can be updated in place.
    mkdir -p "$CACHE/bin" "$CACHE/src" "$CACHE/sources"
    docker run --rm \
        -v "$(pwd)/packages:/packages" \
        -v "$(pwd)/pm.ysh:/usr/bin/pm:ro" \
        -v "$CACHE/bin:/root/.cache/kominka/bin" \
        -v "$CACHE/src:/root/.cache/kominka/src" \
        -v "$CACHE/sources:/root/.cache/kominka/sources" \
        $SEED_MOUNT \
        -e KOMINKA_PATH=/packages \
        -e KOMINKA_COLOR=0 \
        -e KOMINKA_PROMPT=0 \
        -e KOMINKA_FORCE=1 \
        -e LD_LIBRARY_PATH=/usr/lib \
        -e LOGNAME=root \
        -e HOME=/root \
        kominka:builder /usr/local/bin/ysh -c "pm uc $pkg"
}

_find_file() {
    local pattern="$1"
    local f
    for f in $pattern; do
        [[ -f "$f" ]] && { echo "$f"; return; }
    done
}

_file_bytes() {
    local f="$1"
    stat -f%z "$f" 2>/dev/null || stat -c%s "$f" 2>/dev/null || echo 0
}

build_pkg() {
    local pkg="$1"
    ALL_PKGS+=("$pkg")

    # Single-package mode: skip everything else silently.
    [[ -n "$ONLY_PKG" && "$pkg" != "$ONLY_PKG" ]] && return 0

    # Skip packages that already succeeded in the previous run; copy their
    # entry so the summary still reflects them.
    if [[ -n "$PREV_RUN" ]] && grep -q '"pkg":"'"$pkg"'".*"status":"ok"' "$PREV_RUN"; then
        echo "  [skip] $pkg"
        grep '"pkg":"'"$pkg"'"' "$PREV_RUN" >> "$RUN_FILE"
        return 0
    fi

    echo ""
    echo "────────────────────────────────────────"
    echo "  $pkg"
    echo "────────────────────────────────────────"

    update_checksums "$pkg"

    local t0=$SECONDS
    local exit_code=0
    make "$pkg" || exit_code=$?
    local elapsed=$(( SECONDS - t0 ))
    local status=ok
    [[ $exit_code -ne 0 ]] && status=FAIL

    local binball pkg_bytes=0
    binball=$(_find_file "$CACHE/bin/${pkg}@"*.tar.*)
    [[ -n "$binball" ]] && pkg_bytes=$(_file_bytes "$binball")

    local srcball src_bytes=0
    srcball=$(_find_file "$CACHE/src/${pkg}@"*.tar.*)
    [[ -n "$srcball" ]] && src_bytes=$(_file_bytes "$srcball")

    printf '{"pkg":"%s","time":%d,"pkg_bytes":%d,"src_bytes":%d,"status":"%s","ts":"%s"}\n' \
        "$pkg" "$elapsed" "$pkg_bytes" "$src_bytes" "$status" \
        "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
        >> "$RUN_FILE"

    echo "  flushed → $RUN_FILE"

    [[ $exit_code -eq 0 ]] || exit $exit_code
}

total_start=$SECONDS

wave=0
for wave_pkgs in "${WAVES[@]}"; do
    wave=$(( wave + 1 ))
    if [[ -z "$ONLY_PKG" ]]; then
        echo ""
        echo "════════════════════════════════════════"
        echo "  Wave $wave"
        echo "════════════════════════════════════════"
    fi
    for pkg in $wave_pkgs; do
        build_pkg "$pkg"
    done
done

total_time=$(( SECONDS - total_start ))

python3 - "$RUN_FILE" "$BASELINE" "$total_time" <<'PYEOF'
import sys, json, os

def load(path):
    d = {}
    try:
        with open(path) as f:
            for line in f:
                line = line.strip()
                if line:
                    rec = json.loads(line)
                    d[rec["pkg"]] = rec
    except FileNotFoundError:
        pass
    return d

cur   = load(sys.argv[1])
base  = load(sys.argv[2]) if len(sys.argv) > 2 else {}
total = int(sys.argv[3]) if len(sys.argv) > 3 else 0

def fmt_bytes(b):
    if b == 0:       return "      -"
    if b < 1024:     return f"{b:6d}B"
    if b < 1 << 20:  return f"{b / 1024:6.1f}K"
    if b < 1 << 30:  return f"{b / 1048576:6.1f}M"
    return               f"{b / 1073741824:6.1f}G"

def fmt_time(s):
    if s < 60:   return f"{s:3d}s"
    if s < 3600: return f"{s // 60}m{s % 60:02d}s"
    return           f"{s // 3600}h{(s % 3600) // 60:02d}m"

def pct(new, old):
    if old == 0 or new == 0:
        return ""
    d = (new - old) / old * 100
    sign = "+" if d >= 0 else ""
    return f"({sign}{d:.0f}%)"

has_base = bool(base)
col_w = 22

if has_base:
    hdr = f"  {'Package':<{col_w}}  {'Time':>14}  {'PkgSize':>13}  {'SrcBundle':>13}  Status"
else:
    hdr = f"  {'Package':<{col_w}}  {'Time':>6}  {'PkgSize':>7}  {'SrcBundle':>9}  Status"

print()
print("════════════════════════════════════════")
base_label = os.path.basename(sys.argv[2]) if len(sys.argv) > 2 and os.path.exists(sys.argv[2]) else None
print("  Summary" + (f"  [vs {base_label}]" if base_label else "  [no baseline]"))
print("════════════════════════════════════════")
print(hdr)
print("  " + "─" * (len(hdr) - 2))

failed = []
for pkg, rec in cur.items():
    b  = base.get(pkg, {})
    t  = fmt_time(rec["time"])
    tp = pct(rec["time"],      b.get("time",      0))
    ps = fmt_bytes(rec["pkg_bytes"])
    pp = pct(rec["pkg_bytes"], b.get("pkg_bytes", 0))
    ss = fmt_bytes(rec["src_bytes"])
    sp = pct(rec["src_bytes"], b.get("src_bytes", 0))
    st = rec["status"]
    if st != "ok":
        failed.append(pkg)

    if has_base:
        print(f"  {pkg:<{col_w}}  {t:>5} {tp:<8}  {ps} {pp:<7}  {ss} {sp:<7}  {st}")
    else:
        print(f"  {pkg:<{col_w}}  {t:>6}  {ps:>7}  {ss:>9}  {st}")

print()
print(f"  Total: {fmt_time(total)}", end="")
if has_base:
    base_total = sum(r.get("time", 0) for r in base.values())
    print(f"  {pct(total, base_total)}", end="")
print()

if failed:
    print()
    print(f"  FAILED: {', '.join(failed)}")
PYEOF
