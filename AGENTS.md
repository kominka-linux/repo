# Kominka Package System

Three parts: `pm.ysh` (package manager), `server/` (repository server), `packages/` (package definitions).

Read `docs/YSH.md` before editing `pm.ysh` — it is the canonical style guide and gotcha reference for YSH.

## Repository Layout

```
pm.ysh                   Package manager script (~2900 lines)
packages/                Package definitions (one dir per package, each with PKGBUILD.ysh)
server/src/              Rust repository server
  lib.rs                 AppState (db, webauthn, jwks, indexes)
  main.rs                tiny_http server loop, thread-per-request
  packages.rs            route() dispatcher, all HTTP handlers, PackageIndex type
  auth.rs                Bearer token validation: DB lookup then JWT fallback
  db.rs                  SQLite auth store (users, credentials, tokens, sessions)
  jwt.rs                 JWKS fetch + cache, JWT/OIDC verification
  webauthn_handlers.rs   Passkey registration and authentication endpoints
  s3.rs                  Storage enum (S3 via ureq + SigV4 signing, or Memory for tests)
server/static/
  auth.html              Login page served at GET /auth (passkey UI, no external JS)
scripts/                 Build and maintenance scripts
tests/                   pm.ysh tests (Python)
docs/
  YSH.md                 YSH language reference and style guide
  ZIG-CC.md              zig cc + musl build investigation notes
```

## Package Manager (pm.ysh)

### Key globals (imported from ENV at startup)

| Variable | Purpose |
|----------|---------|
| `KOMINKA_PATH` | Colon-separated search path for package definitions |
| `KOMINKA_ROOT` | Install root (empty = `/`) |
| `KOMINKA_REPO` | Repo server base URL |
| `KOMINKA_COMPRESS` | Tarball compression: `gz`, `xz`, `zst` |
| `KOMINKA_TOKEN` | Bearer token for uploads |
| `R2_PUBLIC_URL` | If set, downloads go directly to R2, bypassing the server |

ENV vars are not shell vars in `ysh:all`. They are imported at the top of pm.ysh into regular vars:
```ysh
var KOMINKA_ROOT = ENV => get("KOMINKA_ROOT", "")
```
After import, use `$KOMINKA_ROOT` normally. Do not add `ENV.X` references deep in the file; import at the top instead.

### Package record

`pkg_load` (line ~470) is the central loader. It returns a typed `Dict` from three possible sources — a `PKGBUILD.ysh` file, the installed db at `/var/db/kominka/installed/{pkg}/`, or the remote index. The same `Dict` shape flows through every downstream proc:

```
{name, ver, rel, deps, mkdeps, sources, checksums, nostrip, path}
```

Packages are **never mutated globally** — each proc receives the record as a parameter (named `p` by convention). Source paths beginning with `remote:` are remote index entries, not filesystem paths.

### Dependency resolution

- `pkg_depends` (line ~1246): recursive dep-graph traversal; tracks explicit vs implicit deps
- `pkg_order` (line ~1317): topological sort
- **Makedep skip optimization**: if a runtime dep already has a binary in `~/.cache/kominka/bin/`, its build-only deps are skipped entirely

### Upload flow (`pkg_upload`, line ~706)

Two paths depending on tarball size:

- **< 50 MB**: `POST /api/upload` with the tarball body; server returns `{"ok":true,"sha256":"..."}`
- **≥ 50 MB** (Cloudflare proxy limit): `POST /api/upload-url` → `PUT` directly to R2 → `POST /api/update-index` with `X-Sha256`

After the binary upload, `pkg_upload` also calls `pkg_source_upload` to upload the source tarball if one is cached.

### Source mirror flow

Every `pm b` that uses upstream sources (not the mirror) runs this sequence after `pkg_extract`:

1. `pkg_git_strip` — removes all `.git/` directories
2. `pkg_source_process` — calls `process(src)` from PKGBUILD.ysh if defined
3. `pkg_source_pack` — packs `mak_dir/{pkg}/` as `~/.cache/kominka/src/{pkg}@{ver}-{rel}.tar.bz2` (always bzip2)

When the remote index has `src_sha256` for the current ver-rel, `pm b` downloads the mirror tarball instead of individual upstream sources, skipping steps 1–3.

### Storage

```
~/.cache/kominka/bin/           Binary cache: pkg@ver-rel.tar.gz
~/.cache/kominka/src/           Processed source cache: pkg@ver-rel.tar.bz2
~/.cache/kominka/packages.json  Cached package index (per arch)
/var/db/kominka/installed/{pkg}/
  version                       "ver rel"  (or "system 1" for pre-installed)
  depends                       One dep per line, prefixed "runtime:" or "make:"
  manifest                      Newline-separated installed file paths
```

### Main dispatch

The `args` proc near the end of the file (~line 2616) dispatches commands. New commands go there. Keep new procs under 50 lines; extract helpers if larger. Proc names use hyphens (`pkg-install`), var/func names use underscores (`find_version`).

## PKGBUILD.ysh Format

```ysh
#!/usr/local/bin/ysh

var name      = 'example'
var ver       = '1.0.0'
var rel       = '1'
var deps      = ['musl', 'zlib']      # runtime deps
var mkdeps    = ['make', 'zig']       # build-only deps

var sources   = [
    'https://example.com/example-VERSION.tar.gz',
    'files/my.patch patch',           # second field is destination subdir
]
var checksums = ['sha256hexhere...']

proc build(dest) {
    # dest is the staging directory (DESTDIR)
    ./configure --prefix=/usr
    make
    make DESTDIR=$dest install
}
```

**Source URL substitutions**: `VERSION`, `RELEASE`, `MAJOR`, `MINOR`, `PATCH`, `ARCH`, `GOARCH`, `IDENT`, `PACKAGE` are replaced from the package's own fields.

**Source types**:
- `https://...` — downloaded and verified against checksum
- `git+https://repo.git@branch` — checked out into `src/`
- `files/name` — copied from the package's `files/` directory
- `files/name subdir` — copied into `subdir/` inside the build tree

**Arch-specific checksums**: use `checksums_aarch64` or `checksums_x86_64` to override `checksums` for a specific arch.

**Metapackages**: set `sources = []` and have `build` do nothing (`true`). The `deps` list carries all meaning. Metapackages are skipped by `pm src`.

**`nostrip`**: set `var nostrip = true` to skip binary stripping (needed for Go, Rust, and packages with split debug info).

**`process(src)`**: optional proc to strip unnecessary files from the source tree before it is packed and mirrored. Called after extraction and `.git` removal, before `build`. CWD is set to `src`; use relative paths freely.

```ysh
proc process(src) {
    rm -rf $src/test $src/fuzz $src/third_party/googletest
}
```

### PKGBUILD conventions

- `deps` = runtime libs the binary links against (verified with `ldd` output)
- `mkdeps` = tools needed to compile (zig, make, cmake, etc.) — not installed on target systems
- `nostrip = true` only for packages that ship pre-compiled foreign-arch objects (e.g., Go ships riscv64 `.syso` files that x86_64 strip rejects)
- Bump `rel` (not `ver`) for config-only changes that don't change upstream source
- Keep `build()` under 50 lines; split helpers into nested procs if needed
- Pass compiler flags as a list, then splice: `var flags = ['-O2', ...]; cc @flags`
- `command -v cc` is more reliable than `$CC` — the zig cc wrapper is on PATH
- Comments explaining *why* a flag exists are expected and should be preserved

## Package Philosophy

Kominka is a **minimal, self-hosting Linux**. Every byte counts. Apply these rules without exception when writing or reviewing a PKGBUILD:

**Disable everything you don't explicitly need.**
Configure scripts offer hundreds of optional features; the default answer is `--disable-X` or `--without-X`. Only enable a feature if there is a concrete use case for it in Kominka.

**Concrete rules:**
- Pass `--disable-nls` always (no i18n unless the package is an i18n tool)
- Pass `--disable-static` or `--disable-shared` as appropriate — prefer shared for runtime deps, static only for standalone tools
- Pass `--disable-docs`, `--disable-manual`, `--disable-examples`, `--disable-tests`
- Strip every optional subsystem: LDAP, Kerberos, PAM (unless that IS the point), Python bindings, Perl bindings, TCL, D-Bus, systemd, selinux, audit, gettext, iconv unless strictly required
- No `--enable-debug` or `-g` flags in release builds
- For autotools: `ac_cv_*=yes/no` overrides are fine to skip configure tests

**busybox specifically** was audited against Alpine's main vs extras split. Our config has been through multiple aggressive trim passes to reach ~217 applets. Justify each new applet. When in doubt, leave it out.

**Verify your assumptions with `--help` output.** Run `./configure --help` in a build container and read what's available. Do not guess; do not cargo-cult flags from other distros.

## Repository Server (server/)

### Request Flow

`main.rs` reads each request (method, path, headers, body) and calls `packages::route()` which returns a `Response { status, content_type, body }`. The main loop writes it back via tiny_http. ~800 lines of Rust. Blocking, threaded (tiny_http). No async runtime.

### Storage

`s3::Storage` is an enum:
- `S3 { endpoint, bucket, access_key, secret_key, region }` — real R2, uses ureq for HTTP and manual AWS SigV4 signing (~80 lines in s3.rs)
- `Memory(RwLock<HashMap<String, Vec<u8>>>)` — in-memory, for tests

S3 requests include an explicit `Content-Length` header and use `read_to_end` (not ureq's `read_to_vec` which has a 10MB default limit).

**Tarball downloads redirect to R2's public URL** (`R2_PUBLIC_URL` env var) rather than proxying bytes through the server. `packages.json` is still proxied since it's small and benefits from the in-memory index cache.

### Package Index

```rust
PackageEntry { ver, rel, deps: Vec<String>, mkdeps: Vec<String>, sha256, src_sha256: Option<String> }
PackageIndex { _version: 1, packages: HashMap<name, PackageEntry> }
```

One `packages.json` per architecture, stored in R2 and cached in-memory (`AppState.indexes`). Updated on every upload/publish via read-modify-write under a `RwLock`. Stored at `{arch}/packages.json`; binary tarballs at `{arch}/{pkg}/{ver}-{rel}.tar.gz`; source tarballs at `src/{pkg}/{ver}-{rel}.tar.bz2` (arch-independent).

`src_sha256` is omitted from JSON when `None`. Set by `POST /api/upload-src`.

### Authentication

Two accepted credential types:

**Passkey-issued tokens** — 64 random hex chars (256-bit). Server stores only the SHA-256 hash in SQLite. Created when an admin signs in via `/auth` with a registered passkey. Long-lived, no expiry.

**GitHub OIDC JWTs** — Short-lived tokens fetched by CI. Validated by fetching JWKS from `JWT_JWKS_URL`, verifying the RSA signature, and checking `iss`, `aud`, and `sub` claims. Keys cached in memory for 1 hour. Enabled only when `JWT_JWKS_URL` is set.

Auth check order (`auth.rs`): DB token lookup → JWT verification → 401 `{"error":"unauthorized"}`.

### SQLite Schema

```sql
CREATE TABLE users (
  id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE TABLE credentials (
  id TEXT PRIMARY KEY,        -- hex credential ID
  user_id TEXT NOT NULL REFERENCES users(id),
  passkey TEXT NOT NULL,      -- JSON: {cred_id, x, y, sign_count}
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE TABLE tokens (
  id TEXT PRIMARY KEY,
  user_id TEXT NOT NULL REFERENCES users(id),
  token_hash TEXT NOT NULL UNIQUE,  -- SHA-256 of bearer token
  name TEXT NOT NULL DEFAULT 'cli',
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  last_used TEXT, expires_at TEXT   -- NULL means never expires
);
CREATE TABLE browser_sessions (
  id TEXT PRIMARY KEY,
  user_id TEXT NOT NULL REFERENCES users(id),
  session_hash TEXT NOT NULL UNIQUE,
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  expires_at TEXT NOT NULL DEFAULT (datetime('now', '+30 days'))
);
CREATE TABLE sessions (
  id TEXT PRIMARY KEY,         -- 64 hex chars
  token TEXT,                  -- plaintext (ephemeral, returned once then cleared)
  challenge TEXT,              -- WebAuthn challenge
  user_id TEXT, status TEXT NOT NULL DEFAULT 'pending',
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
```

### WebAuthn Flow

`/auth` serves `static/auth.html`. Registration and authentication use the standard WebAuthn browser API with base64url helpers inlined — no external JS. Challenge state is stored in the `sessions` table between options and verify round-trips. Sessions expire after 10 minutes.

`pm auth` polling flow:
1. Client creates a session (gets a 64-char hex session ID)
2. Opens `https://repo.kominka.org/auth?session={id}` in browser
3. Polls `GET /auth/poll?session={id}` every 2s (up to 5 min)
4. User taps passkey → server creates token → binds to session
5. Poll returns token (once, then server clears it)
6. `pm` stores token via `auth_token_store`

### HTTP API Endpoints

```
GET  /auth?session={id}               passkey login page
GET  /auth/settings                   token management UI (browser session required)
GET  /auth/logout
GET  /auth/poll?session={id}          CLI polls for completed token
POST /auth/register/options           registration challenge
POST /auth/register/verify            verify + create user + token
POST /auth/authenticate/options       authentication challenge
POST /auth/authenticate/verify        verify assertion + create token
POST /auth/tokens                     create a named token
POST /auth/tokens/delete              delete a token by ID

GET  /health
GET  /{arch}/packages.json
GET  /{arch}/{pkg}/{ver}-{rel}.tar.gz
GET  /src/{pkg}/{ver}-{rel}.tar.bz2

POST /api/upload        X-Arch, X-Pkg, X-Ver, X-Rel, X-Deps, X-Mkdeps; body: tarball
POST /api/upload-src    X-Pkg, X-Ver, X-Rel; body: source .tar.bz2
POST /api/upload-url    X-Arch, X-Pkg, X-Ver, X-Rel → {"url":"..."}
POST /api/update-index  X-Arch, X-Pkg, X-Ver, X-Rel, X-Sha256, X-Deps, X-Mkdeps
POST /api/publish       {"arch","pkg","ver","rel","deps","mkdeps"} — metapackages
POST /api/reindex       {"arch","pkg","ver","rel"} — recompute sha256 from R2
POST /api/delete        {"arch","pkg"}
```

All `/api/*` endpoints require `Authorization: Bearer <token>` or session cookie.

**Deps/mkdeps encoding**: comma-separated string in X-headers (`"curl,zlib"`); JSON array in body endpoints.

**Known architectures**: `aarch64-linux-gnu`, `x86_64-linux-gnu`.

**Package name rules** (`valid_pkg_name`): lowercase alphanumeric, `.`, `-`, `_`; must start with alphanumeric.

### Rate Limiting

In-memory token bucket:
- Auth endpoints: 10 requests/min per IP
- Uploads: 60 requests/min per token
- Index/tarball GETs: no limit (public, cacheable)

## Building Packages

```sh
# In this repo (pm.ysh and packages/ are co-located):
make rebuild-<pkg>           # build with zig cc in kominka:core
make rebuild-<pkg>-debian    # build with Debian GCC (glibc, git, strace...)
```

Both source credentials from `.env` automatically.

## Scripts

| Script | Purpose |
|--------|---------|
| `scripts/repology-latest.py` | Check package versions vs upstream (repology.org) |
| `scripts/build-deb.sh` | Build a .deb for the server binary |
| `scripts/reindex.sh` | Re-register packages already in R2 |

Run repology check: `python3 scripts/repology-latest.py --scan packages/`

## Development

### Server

```sh
source .env          # load credentials and auth config
make dev             # check required vars, then: cd server && cargo run
make test            # run tests (Storage::Memory, no S3, no DB needed)
```

On first run of `make dev`, open `http://localhost:3000/auth` to register your passkey and get a `KOMINKA_TOKEN`. Add it to `.env`.

`cargo test` runs tests across `server/tests/api.rs` and `server/src/db.rs`. Tests call `packages::route()` directly with `Storage::Memory` and an in-memory SQLite DB — no HTTP, no S3, no threads, no real passkeys.

### pm.ysh

```sh
python3 tests/test_pm_cheap.py   # cheap tests: dep resolution, search, install, etc.
```

## Common Tasks

### Add a new package

1. Create `packages/{name}/PKGBUILD.ysh` with the fields above.
2. Run `pm c {name}` to generate checksums.
3. Add `proc process(src)` if the upstream has large test/benchmark dirs worth stripping.
4. Test with `pm b {name}` — builds in a temp dir, staging to a `dest/` prefix.
5. Upload with `pm p {name}` (requires auth token).

### Backfill source mirrors

For packages already in the repo that predate source mirroring:

```
pm src pkg1 pkg2 pkg3 ...
```

Downloads upstream sources, strips `.git/`, runs `process()` if defined, packs, and uploads. Skips packages already mirrored at the current ver-rel.

### Bump a package version

1. Update `ver` and optionally `rel` in `PKGBUILD.ysh`.
2. Update `checksums` (run `pm c {name}` or compute `sha256sum` on the new source).
3. Rebuild and upload.

### Modify the server

Route dispatch is in `packages.rs:route()`. All index mutations go through `update_index()` which holds the write lock, updates the in-memory `state.indexes`, and persists to S3. Build with `cargo build` (not `--release`).

## YSH Patterns in pm.ysh

**Dict mutation inside procs** — `setvar d[k] = v` looks for a local `var d`. For globals use `setglobal d[k] = v`. `call list->append(x)` works on both because it mutates in-place.

**No `||` on proc calls** — `my_proc || die` triggers OILS-ERR-301. Use:
```ysh
try { my_proc }
if failed { die "msg" }
```

**Splice, don't split** — `@flags` splices a list as separate words; `$flags` would pass the whole list as one string. Always build flag lists and splice.

**Globbing** — bare `*.tar.gz` does not expand. Use `@[glob('*.tar.gz')]`.

**Backslash in expression context** — `var x = '\n'` is OILS-ERR-20. Use `u'\n'` (J8) or `$[newline]`.

**Literal `@` in command arguments** — bare `@` at the start of a word is a splice operator (`parse_at_all`). To pass a literal `@` to a command (e.g. curl's `--data-binary @file`), quote it: `"@${file}"` or `'@'$file`.
