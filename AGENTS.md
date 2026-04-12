# Repository Architecture

## What This Is

Package repository for Kominka Linux. Two parts:

1. **packages/** — PKGBUILD.ysh definitions for ~40 packages. Each defines
   name, version, dependencies, sources, and a `build()` proc.

2. **server/** — Rust HTTP server that stores package tarballs in Cloudflare R2
   (via S3 API) and maintains a per-architecture JSON index.

The client is `pm` (package manager) in the `davinci` repo at `~/d/davinci/pm.ysh`.

## Server

~800 lines of Rust. Blocking, threaded (tiny_http). No async runtime.

```
server/src/
  lib.rs               AppState (db, webauthn, jwks, indexes)
  main.rs              tiny_http server loop, thread-per-request
  packages.rs          route() dispatcher, all HTTP handlers, PackageIndex type
  auth.rs              Bearer token validation: DB lookup then JWT fallback
  db.rs                SQLite auth store (users, credentials, tokens, sessions)
  jwt.rs               JWKS fetch + cache, JWT/OIDC verification
  webauthn_handlers.rs Passkey registration and authentication endpoints
  s3.rs                Storage enum (S3 via ureq + SigV4 signing, or Memory for tests)
server/static/
  auth.html            Login page served at GET /auth (passkey UI, no external JS)
```

### Request Flow

`main.rs` reads each request (method, path, headers, body) and calls
`packages::route()` which returns a `Response { status, content_type, body }`.
The main loop writes it back via tiny_http.

### Storage

`s3::Storage` is an enum:
- `S3 { endpoint, bucket, access_key, secret_key, region }` — real R2, uses
  ureq for HTTP and manual AWS SigV4 signing (~80 lines in s3.rs)
- `Memory(RwLock<HashMap<String, Vec<u8>>>)` — in-memory, for tests

S3 requests include an explicit `Content-Length` header and use `read_to_end`
(not ureq's `read_to_vec` which has a 10MB default limit).

### Package Index

One `packages.json` per architecture, stored in R2 and cached in-memory
(`AppState.indexes`). Updated on every upload/publish via read-modify-write
under a `RwLock`.

Format:
```json
{
  "_version": 1,
  "packages": {
    "curl": {
      "ver": "8.19.0", "rel": "6",
      "deps": ["boringssl", "zlib"],
      "mkdeps": ["zig", "make"],
      "sha256": "<sha256 of tarball>"
    }
  }
}
```

Tarballs are stored at `{arch}/{pkg}/{ver}-{rel}.tar.gz` — version-addressed,
not content-addressed. The sha256 field is for integrity verification on download.

### Auth

Bearer token authentication. Two accepted credential types:

**Passkey-issued tokens** — 64 random hex chars (256-bit). The server stores
only the SHA-256 hash in SQLite (`tokens` table). Created when an admin signs
in via `/auth` with a registered passkey (Touch ID, hardware key). Long-lived,
no expiry. Checked by hashing the presented token and doing a DB lookup.

**GitHub OIDC JWTs** — Short-lived tokens fetched by CI via the GitHub OIDC
endpoint. Validated by fetching JWKS from `JWT_JWKS_URL`, verifying the RSA
signature, and checking `iss`, `aud`, and `sub` claims. Keys are cached in
memory for 1 hour. Enabled only when `JWT_JWKS_URL` is set in env.

Auth check order (in `auth.rs`): DB token lookup → JWT verification → 401.

### WebAuthn Flow

`/auth` serves `static/auth.html`. Registration and authentication use the
standard WebAuthn browser API with base64url helpers inlined in the HTML —
no external JS. Challenge state (`PasskeyRegistration` / `PasskeyAuthentication`)
is serialized and stored in the `sessions` table between the options and verify
round-trips. Sessions expire after 10 minutes.

On successful authentication the server creates a new token, stores its hash,
and returns the plaintext once. Tokens are also stored in the session row so
a future `pm auth` CLI flow can poll `/auth/poll?session={id}` to retrieve them.

## pm Integration

`pm.ysh` uses `KOMINKA_REPO` to talk to this server:

- `pm i curl` — fetches `packages.json`, resolves deps, downloads tarballs
  from the server. No local checkout of package definitions needed.
- `pm p curl` — POSTs the built tarball to `/api/upload` with metadata headers.
- `pm u` — downloads fresh `packages.json` before doing git pull.

Key pm procs: `index_load`, `index_refresh`, `_download`, `pkg_cache`,
`pkg_upload`, `auth_token_load`, `auth_token_store`.

To populate the index: `for pkg in packages/*/; do pm p "$(basename "$pkg")"; done`.

## Scripts

| Script | Purpose |
|--------|---------|
| `scripts/repology-latest.py` | Check package versions vs upstream (repology.org) |
| `scripts/build-deb.sh` | Build a .deb for the server binary |

Run repology check: `python3 scripts/repology-latest.py --scan packages/`

## Package Philosophy

Kominka is a **minimal, self-hosting Linux**. Every byte counts. Apply these
rules without exception when writing or reviewing a PKGBUILD:

**Disable everything you don't explicitly need.**
Configure scripts offer hundreds of optional features; the default answer is
`--disable-X` or `--without-X`. Only enable a feature if there is a concrete
use case for it in Kominka. Leaving defaults in place is a mistake.

**Concrete rules:**
- Pass `--disable-nls` always (no i18n unless the package is an i18n tool)
- Pass `--disable-static` or `--disable-shared` as appropriate — prefer
  shared for runtime deps, static only for standalone tools
- Pass `--disable-docs`, `--disable-manual`, `--disable-examples`,
  `--disable-tests` (CI runs tests separately if at all)
- Strip every optional subsystem: LDAP, Kerberos, PAM (unless that IS the
  point), Python bindings, Perl bindings, TCL, D-Bus, systemd, selinux,
  audit, gettext, iconv unless strictly required
- No `--enable-debug` or `-g` flags in release builds
- For autotools: `ac_cv_*=yes/no` overrides are fine to skip configure tests
  that would fail or pull in wrong deps

**busybox specifically** was audited against Alpine Linux's main vs extras
split. Anything Alpine puts in `busybox-extras` is optional. Our config has
been through multiple aggressive trim passes to reach ~217 applets. When
adding new applets, justify each one. When in doubt, leave it out.

**Verify your assumptions with `--help` output.** Before writing a PKGBUILD,
run `./configure --help` in a build container and read what's available.
Do not guess; do not cargo-cult flags from other distros without understanding
what they do.

**PKGBUILD structure rules:**
- `deps` = runtime libs the binary links against (verified with `ldd` output)
- `mkdeps` = tools needed to compile (zig, make, cmake, etc.) — not installed
  on target systems
- `nostrip = true` only for packages that ship pre-compiled foreign-arch
  objects (e.g., Go ships riscv64 `.syso` files that x86_64 strip rejects)
- Bump `rel` (not `ver`) for config-only changes that don't change upstream
  source

## Building Packages

```sh
# In ~/d/davinci (the pm.ysh repo):
make rebuild-<pkg>           # build with zig cc in kominka:core
make rebuild-<pkg>-debian    # build with Debian GCC (glibc, git, strace...)
```

Both source credentials from `~/d/repo/.env` automatically.

## Server Development

```sh
source .env          # load credentials and auth config
make dev             # check required vars, then: cd server && cargo run
make test            # run tests (Storage::Memory, no S3, no DB needed)
```

On first run of `make dev`, open `http://localhost:3000/auth` to register your
passkey and get a `KOMINKA_TOKEN`. Add it to `.env`.

Dependencies: tiny_http, ureq, sha2, hmac, serde/serde_json, tracing,
rusqlite (bundled SQLite), webauthn-rs, jsonwebtoken, url, uuid — all
blocking/sync. No tokio.

## Testing

`cargo test` (via `make test`) runs 15 integration tests in `tests/api.rs`.
Tests call `packages::route()` directly with `Storage::Memory` and an
in-memory SQLite DB — no HTTP, no S3, no threads, no real passkeys. Covers:
upload + index round-trip, auth rejection, input validation, arch isolation,
metapackage publishing, SHA-256 correctness, index accumulation and overwrite
behavior, large body integrity.
