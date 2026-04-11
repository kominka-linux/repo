# Repository Architecture

## What This Is

Package repository for Kominka Linux. Two parts:

1. **packages/** — PKGBUILD.ysh definitions for ~40 packages. Each defines
   name, version, dependencies, sources, and a `build()` proc.

2. **server/** — Rust HTTP server that stores package tarballs in Cloudflare R2
   (via S3 API) and maintains a per-architecture JSON index.

The client is `pm` (package manager) in the `davinci` repo at `~/d/davinci/pm.ysh`.

## Server

~400 lines of Rust. Blocking, threaded (tiny_http). No async runtime.

```
server/src/
  lib.rs         AppState (Storage + API key hash + index cache)
  main.rs        tiny_http server loop, thread-per-request
  packages.rs    route() dispatcher, all HTTP handlers, PackageIndex type
  auth.rs        Bearer token validation (SHA-256 comparison)
  s3.rs          Storage enum (S3 via ureq + SigV4 signing, or Memory for tests)
```

### Request Flow

`main.rs` reads each request (method, path, headers, body) and calls
`packages::route()` which returns a `Response { status, content_type, body }`.
The main loop writes it back via tiny_http.

### Storage

`s3::Storage` is an enum:
- `S3 { endpoint, bucket, access_key, secret_key, region }` — real R2, uses
  ureq for HTTP and manual AWS SigV4 signing (~80 lines)
- `Memory(RwLock<HashMap<String, Vec<u8>>>)` — in-memory, for tests

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
      "hash": "<sha256 of PKGBUILD.ysh>",
      "sha256": "<sha256 of tarball>"
    }
  }
}
```

### Content-Addressed Tarballs

R2 key: `{arch}/{pkg}/{hash}.tar.gz` where `hash = sha256(PKGBUILD.ysh)`.
Any change to the package definition produces a new hash. Version strings are
metadata in the index, not part of the storage key.

### Auth

V1: static API key. Server stores `sha256(API_KEY)`, compares against
`sha256(bearer_token)` on each authenticated request. No database.

V2 (designed, not yet implemented): browser passkeys + JWT/OIDC for CI.
See `REPOSITORY.md` in davinci for the full v2 design including SQLite schema,
WebAuthn flow, and GitHub Actions OIDC integration.

## pm Integration

The `davinci` repo's `pm.ysh` uses `KOMINKA_REPO` to talk to this server:

- `pm i curl` — fetches `packages.json`, resolves deps, downloads tarballs
  by hash from the server. No git checkout of package definitions needed.
- `pm p curl` — POSTs the built tarball to `/api/upload` with metadata headers.
- `pm u` — downloads fresh `packages.json` before doing git pull.
- `pm auth` — prompts for the API key and stores it in keychain or file.

Key pm procs: `index_load`, `index_refresh`, `_download`, `pkg_hash`,
`pkg_cache`, `pkg_upload`, `auth_token_load`, `auth_token_store`.

To populate the index from scratch: `for pkg in packages/*/; do pm p "$(basename "$pkg")"; done`.
This handles both regular packages (uploads tarball) and metapackages (publishes metadata).

## Dependencies

8 direct, ~63 total (including transitive). No async runtime.

| Crate | Role |
|-------|------|
| tiny_http | HTTP server |
| ureq | HTTP client (S3 calls) |
| sha2 | SHA-256 (content hashing, auth) |
| hmac | HMAC-SHA256 (SigV4 signing) |
| serde + serde_json | JSON serialization |
| tracing + tracing-subscriber | Logging |

## Testing

`cargo test` runs 14 integration tests in `tests/api.rs`. Tests call
`packages::route()` directly with `Storage::Memory` — no HTTP, no S3, no
threads. Covers: upload + index round-trip, auth rejection, input validation,
arch isolation, metapackage publishing, SHA-256 correctness, index
accumulation and overwrite behavior.
