# Repository Design

## Content-Addressed Binary Cache

Tarballs are named by a hash of their build inputs, not by version string.

```
hash = sha256(contents of PKGBUILD.ysh)
```

If anything in the package definition changes (version, dep, build script,
source URL), the hash changes. Does NOT include transitive dependency hashes —
avoids Nix-style rebuild cascades. Computable before building, which enables
cache-hit detection: if `{arch}/{pkg}/{hash}.tar.gz` exists on R2, skip the
build entirely.

```
Local:  ${bin_dir}/${pkg}@${hash}.tar.gz
R2:     ${arch}/${pkg}/${hash}.tar.gz
```

The installed database gains a `hash` file at
`/var/db/kominka/installed/{pkg}/hash`, written during build (included in the
tarball and tracked by the manifest). This lets `pkg_outdated` compare hashes
instead of version-release strings — catches cases where the build script
changed without a version bump.

## Package Index

One JSON file per architecture, stored in R2, served by the server.

```json
{
  "_version": 1,
  "packages": {
    "curl": {
      "ver": "8.19.0",
      "rel": "6",
      "deps": ["boringssl", "zlib"],
      "hash": "a1b2c3d4...",
      "sha256": "e5f6a7b8..."
    }
  }
}
```

- `_version` — index format version (for future evolution)
- `hash` — PKGBUILD.ysh content hash (R2 key / build identity)
- `sha256` — tarball content hash (download integrity verification; empty for
  metapackages)
- `deps` — runtime deps only (mkdeps not needed for `pm i`)

Updated server-side on each upload via read-modify-write. Server keeps the
index in memory and writes to S3 on mutation. On startup, reads from S3 to
hydrate.

## V2: Passkey Authentication + JWT for CI

The static API key is sufficient for a single maintainer but doesn't scale to
multiple contributors and lacks the security properties of modern
authentication. V2 replaces it with browser passkeys for human auth and
JWT/OIDC for CI.

### Dependencies Added

| Crate | Role |
|-------|------|
| `rusqlite` (bundled) | Auth state: users, credentials, tokens, sessions |
| `webauthn-rs` | Passkey registration/authentication |
| `jsonwebtoken` | CI OIDC token verification |

### SQLite Schema

```sql
CREATE TABLE users (
  id         TEXT PRIMARY KEY,
  name       TEXT NOT NULL UNIQUE,
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE credentials (
  id         TEXT PRIMARY KEY,       -- base64url credential ID
  user_id    TEXT NOT NULL REFERENCES users(id),
  public_key BLOB NOT NULL,          -- COSE public key
  counter    INTEGER NOT NULL DEFAULT 0,
  transports TEXT,                   -- JSON array: ["internal","hybrid"]
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE tokens (
  id         TEXT PRIMARY KEY,
  user_id    TEXT NOT NULL REFERENCES users(id),
  token_hash TEXT NOT NULL UNIQUE,   -- SHA-256 of bearer token
  name       TEXT NOT NULL DEFAULT 'cli',
  created_at TEXT NOT NULL DEFAULT (datetime('now')),
  last_used  TEXT
);

CREATE TABLE sessions (
  id         TEXT PRIMARY KEY,       -- 64 hex chars
  token      TEXT,                   -- plaintext (ephemeral, returned once then cleared)
  challenge  TEXT,                   -- WebAuthn challenge
  user_id    TEXT,
  status     TEXT NOT NULL DEFAULT 'pending',  -- pending | completed | consumed
  created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
```

### Configuration Added

```sh
ALLOWED_USERS=josh
RP_ID=repo.kominka.org
RP_ORIGIN=https://repo.kominka.org
DB_PATH=/var/lib/kominka-repo/auth.db
JWT_JWKS_URL=https://token.actions.githubusercontent.com/.well-known/jwks
JWT_ISSUER=https://token.actions.githubusercontent.com
JWT_AUDIENCE=kominka-repo
JWT_SUBJECT_PATTERN=repo:josh/*
```

### Passkey Auth Flow

```
pm auth
  1. openssl rand -hex 32 → session ID
  2. Open https://repo.kominka.org/auth?session={id} in browser
     (macOS: open, Linux: xdg-open, fallback: print URL)
  3. Poll GET /auth/poll?session={id} every 2s (up to 5 min)
  4. Browser: user taps passkey → server creates token → binds to session
  5. Poll returns token (once, then server clears it from session row)
  6. pm stores token via auth_token_store
```

Session expiry: 10 minutes. Token returned exactly once, then cleared.

### API Endpoints Added

```
GET  /auth?session={id}               # passkey HTML page
POST /auth/register/options            # registration challenge
POST /auth/register/verify             # verify + create user + token
POST /auth/authenticate/options        # authentication challenge
POST /auth/authenticate/verify         # verify assertion + create token
GET  /auth/poll?session={id}           # CLI polls for completed token
```

### Auth Page

Single HTML file with `@simplewebauthn/browser` bundled inline (pre-built with
esbuild, committed as static asset). Minimal styling (system font, centered
card, dark mode via `prefers-color-scheme`).

Flow:
1. Page reads `session` from query params
2. If user has no passkey registered: show Register button (enter username,
   tap passkey)
3. If registered: show Sign In button (tap passkey, discoverable)
4. On success: "Done — you can close this tab"

Allowed usernames hardcoded in `ALLOWED_USERS` env var. Registration rejects
unknown usernames.

### Auth Middleware (replaces static key check)

1. Extract bearer token from `Authorization` header
2. SHA-256 hash it, check `tokens` table → authenticated (passkey path)
3. If not found, attempt JWT decode + JWKS verification → authenticated (CI)
4. Neither → `401 Unauthorized`

Token properties: 64 random hex bytes (256 bits entropy). Stored as SHA-256
hash in DB — server never stores plaintext. No expiration (long-lived like SSH
keys). `last_used` updated on each upload.

JWT config is optional — if `JWT_JWKS_URL` is unset, only passkey-issued
tokens work.

### JWT/OIDC for CI

GitHub Actions provides a short-lived OIDC token. The CI job sets
`KOMINKA_TOKEN` and calls `pm p`. The server validates the JWT by:

1. Fetching JWKS keys from `JWT_JWKS_URL` (cached, periodically refreshed)
2. Verifying JWT signature against JWKS
3. Checking claims: `iss`, `aud`, `sub` against configured values

Example GitHub Actions usage:
```yaml
permissions:
  id-token: write
  contents: read

steps:
  - name: Get OIDC token
    run: |
      TOKEN=$(curl -s \
        -H "Authorization: Bearer $ACTIONS_ID_TOKEN_REQUEST_TOKEN" \
        "$ACTIONS_ID_TOKEN_REQUEST_URL&audience=kominka-repo" | jq -r .value)
      echo "KOMINKA_TOKEN=$TOKEN" >> $GITHUB_ENV

  - name: Upload package
    run: pm p curl
    env:
      KOMINKA_REPO: https://repo.kominka.org
```

### V2 Presigned Downloads

Replace the proxy-based GET handler with presigned S3 URLs:

1. S3 HeadObject to verify existence → 404 if missing
2. Generate presigned S3 GET URL (1 hour TTL)
3. Return `302 Location: <presigned URL>`
4. On presign failure: fall back to proxy

curl (`-fLo`) and busybox wget both follow 302 redirects and preserve query
parameters in the redirect URL (where the S3 signature lives). This offloads
download bandwidth from the server to R2.

### Rate Limiting

In-memory token bucket:
- Auth endpoints: 10 requests/min per IP
- Uploads: 60 requests/min per token
- Index/tarball GETs: no limit (public, cacheable)
