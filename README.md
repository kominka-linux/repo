# Kominka Package Repository

Package definitions and repository server for [Kominka Linux](https://github.com/user/davinci).

```
packages/       Package definitions (PKGBUILD.ysh files)
server/         Repository server (Rust)
scripts/        Build .deb
```

## Server Setup

**1. Create an R2 bucket** in the Cloudflare dashboard. Note the S3 endpoint
(`https://<ACCOUNT_ID>.r2.cloudflarestorage.com`), bucket name, and API credentials.

**2. Configure** — copy and fill in `server/kominka-repo.env.example`:

```sh
LISTEN_ADDR=127.0.0.1:3000
S3_ENDPOINT=https://<ACCOUNT_ID>.r2.cloudflarestorage.com
S3_BUCKET=kominka-packages
S3_ACCESS_KEY_ID=...
S3_SECRET_ACCESS_KEY=...
S3_REGION=auto

DB_PATH=/var/lib/kominka-repo/auth.db
ALLOWED_USERS=josh
RP_ID=repo.kominka.org
RP_ORIGIN=https://repo.kominka.org

# JWT/OIDC for CI (optional)
JWT_JWKS_URL=https://token.actions.githubusercontent.com/.well-known/jwks
JWT_ISSUER=https://token.actions.githubusercontent.com
JWT_AUDIENCE=kominka-repo
JWT_SUBJECT_PATTERN=repo:josh/*
```

**3. Run**

```sh
# Local dev (checks required env vars)
source .env && make dev

# Production (systemd)
./scripts/build-deb.sh && dpkg -i kominka-repo_*.deb
systemctl enable --now kominka-repo
```

Put a reverse proxy (caddy, nginx) in front for TLS. The server binds `127.0.0.1:3000`.

**4. First login** — open `/auth`, register a passkey, copy the generated `KOMINKA_TOKEN`
into `.env`. Subsequent logins just set a browser session; manage tokens at `/auth/settings`.

## Client Setup

`pm` authenticates with `KOMINKA_TOKEN` in the environment. Get a token at `/auth/settings`.

For CI, GitHub Actions uses a short-lived OIDC token automatically — no stored secret needed.
The workflow has `id-token: write` and fetches the JWT from GitHub's OIDC endpoint with
`audience=kominka-repo`. Configure `JWT_*` vars on the server to validate it.

## API

### Public

```
GET /health
GET /{arch}/packages.json
GET /{arch}/{pkg}/{ver}-{rel}.tar.gz
```

### Auth

```
GET  /auth                            → login page
POST /auth/register/options           → start passkey registration
POST /auth/register/verify            → complete registration
POST /auth/authenticate/options       → start authentication
POST /auth/authenticate/verify        → complete authentication
GET  /auth/settings                   → token management page
POST /auth/tokens                     → create token  {"name","expires_days"?}
POST /auth/tokens/delete              → delete token  {"id"}
GET  /auth/logout
```

### Authenticated (`Authorization: Bearer <token>`)

```
POST /api/upload     X-Arch, X-Pkg, X-Ver, X-Rel, X-Deps, X-Mkdeps; body: tarball
POST /api/publish    {"arch","pkg","ver","rel","deps","mkdeps"}
POST /api/reindex    {"arch","pkg","ver","rel"}
POST /api/delete     {"arch","pkg"}
```

## Storage

Tarballs live at `{arch}/{pkg}/{ver}-{rel}.tar.gz` in R2. The index tracks sha256 per tarball.
