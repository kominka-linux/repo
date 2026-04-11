#!/usr/bin/env python3
"""
Query repology.org for the latest version of one or more packages.

Usage:
  repology-latest.py <pkg> [<pkg> ...]          # print latest version(s)
  repology-latest.py --scan <pkgbuild-dir>       # compare repo vs latest
"""

import json
import re
import sys
import time
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path


API = "https://repology.org/api/v1/project/{}"


def fetch(name: str, retries=3):
    req = urllib.request.Request(API.format(name), headers={"User-Agent": "repology-latest/1.0"})
    for attempt in range(retries):
        try:
            with urllib.request.urlopen(req, timeout=10) as r:
                return json.loads(r.read().decode())
        except urllib.error.HTTPError as e:
            if e.code == 404:
                return None
            if e.code == 429 and attempt < retries - 1:
                time.sleep(2 ** attempt)
                continue
            raise


# Distros we trust as upstream-tracking references.
TRUSTED = {"arch", "alpine_edge", "void", "fedora_rawhide", "gentoo", "nixpkgs_unstable"}

_tok = re.compile(r"(\d+|[a-zA-Z]+)")


def _key(v: str):
    # Each token is (0, str) for alpha (pre-release) or (1, int) for numeric,
    # so alpha segments sort below numeric — "1.0rc1" < "1.0.0".
    result = []
    for t in _tok.findall(v):
        result.append((1, int(t)) if t.isdigit() else (0, t.lower()))
    return result


def latest(name: str):
    """Return (version, trusted_repos) for the newest version of name."""
    data = fetch(name)
    if not data:
        return None, []
    newest_ver = None
    for pkg in data:
        if pkg.get("status") == "newest" and "version" in pkg:
            newest_ver = pkg["version"]
            break
    if newest_ver is None:
        versions = [pkg["version"] for pkg in data if "version" in pkg]
        if not versions:
            return None, []
        newest_ver = max(versions, key=_key)
    trusted_repos = sorted({
        pkg["repo"] for pkg in data
        if pkg.get("version") == newest_ver and pkg.get("repo") in TRUSTED
    })
    return newest_ver, trusted_repos


def read_pkgbuilds(repo_dir: Path):
    pkgs = {}
    for pb in sorted(repo_dir.glob("*/PKGBUILD.ysh")):
        name = pb.parent.name
        ver = None
        for line in pb.read_text().splitlines():
            m = re.match(r"var\s+ver\s*=\s*'([^']+)'", line)
            if m:
                ver = m.group(1)
                break
        if ver:
            pkgs[name] = ver
    return pkgs


def scan(repo_dir: Path):
    pkgs = read_pkgbuilds(repo_dir)

    # Skip metapackages / in-house packages with no upstream.
    skip = {"baseinit", "baselayout", "build-essential", "core", "kominka", "liveiso"}
    to_check = {k: v for k, v in pkgs.items() if k not in skip}

    results = {}
    with ThreadPoolExecutor(max_workers=2) as pool:
        futures = {pool.submit(latest, name): name for name in to_check}
        for fut in as_completed(futures):
            name = futures[fut]
            try:
                results[name] = fut.result()
            except Exception as e:
                results[name] = (f"error: {e}", [])

    for name in sorted(to_check):
        current = to_check[name]
        ver, repos = results.get(name, (None, []))
        repo_tag = f"  [{', '.join(repos)}]" if repos else ""
        if ver is None:
            print(f"{name}: {current}  (not found on repology)")
        elif ver == current:
            print(f"{name}: {current}  (up to date){repo_tag}")
        else:
            print(f"{name}: {current} -> {ver}{repo_tag}")


if __name__ == "__main__":
    args = sys.argv[1:]

    if not args:
        print(__doc__.strip())
        sys.exit(1)

    if args[0] == "--scan":
        if len(args) < 2:
            print("--scan requires a directory argument", file=sys.stderr)
            sys.exit(1)
        scan(Path(args[1]))
    else:
        with ThreadPoolExecutor(max_workers=2) as pool:
            futures = {pool.submit(latest, name): name for name in args}
            results = {}
            for fut in as_completed(futures):
                name = futures[fut]
                results[name] = fut.result()
        for name in args:
            ver, repos = results[name]
            repo_tag = f"  [{', '.join(repos)}]" if repos else ""
            if len(args) == 1:
                print(ver or "not found")
            else:
                print(f"{name}: {ver or 'not found'}{repo_tag}")
