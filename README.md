# Kominka Package Repository

Package definitions, package manager, and repository server for Kominka Linux.

```
pm.ysh          Package manager script
packages/       Package definitions (PKGBUILD.ysh files)
server/         Repository server (Rust)
scripts/        Maintenance scripts
tests/          pm.ysh tests
docs/           Reference docs (YSH.md, ZIG-CC.md)
```

## Package Manager

`pm.ysh` is a ~2900-line YSH script. It resolves, builds, installs, and publishes packages.

### Commands

| Command | Description |
|---------|-------------|
| `pm i pkg` | Install binary from repo, auto-resolve runtime deps |
| `pm b pkg` | Build from source, resolve build+runtime deps |
| `pm p pkg` | Upload built tarball to repo server |
| `pm u` | Update local package index from repo server |
| `pm U` | Upgrade installed packages |
| `pm t pkg` | Show full dependency tree |
| `pm l` | List installed packages |
| `pm s pkg` | Show package info / search |
| `pm r pkg` | Remove package |
| `pm c pkg` | Generate checksums for sources |
| `pm d pkg` | Download sources |
| `pm src pkg` | Backfill source mirror |

### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `KOMINKA_PATH` | `/packages` | Colon-separated path to package definitions |
| `KOMINKA_ROOT` | `` | Install root (empty = `/`) |
| `KOMINKA_REPO` | `` | Repo server URL |
| `KOMINKA_GET` | `` | Downloader binary (curl or wget) |
| `KOMINKA_INSECURE` | `` | Skip TLS verification if `1` |
| `KOMINKA_COMPRESS` | `gz` | Tarball compression format |
| `KOMINKA_FORCE` | `` | Force reinstall if `1` |
| `KOMINKA_TOKEN` | `` | Auth token for `pm p` uploads |

### Key Behaviors

- **Parallel downloads** with live progress display
- **Binary cache** at `~/.cache/kominka/bin/` — pre-seeded tarballs skip downloads
- **Makedep skip optimization** — if a runtime dep has a pre-built binary, its makedeps are skipped
- **Source mirroring** — processed source tarballs are uploaded alongside binaries; subsequent builds pull from the mirror

### Storage Layout

```
~/.cache/kominka/bin/       Binary cache (pkg@ver-rel.tar.gz)
~/.cache/kominka/src/       Processed source cache (pkg@ver-rel.tar.bz2)
/var/db/kominka/installed/  Installed package database
  {pkg}/version             "ver rel" or "system 1"
  {pkg}/depends             Runtime deps, one per line
  {pkg}/manifest            Installed file paths
```

## Package Format

Each package is a directory under `packages/` containing `PKGBUILD.ysh`:

```ysh
#!/usr/local/bin/ysh

var name      = 'example'
var ver       = '1.0.0'
var rel       = '1'
var deps      = ['musl', 'zlib']      # runtime deps
var mkdeps    = ['zig', 'make']       # build-only deps

var sources   = ['https://example.com/example-VERSION.tar.gz']
var checksums = ['sha256hash...']

proc build(dest) {
    # dest is the staging directory (DESTDIR)
    env CC="$(command -v zig) cc -target $(uname -m)-linux-musl" \
    ./configure --prefix=/usr
    make
    make DESTDIR=$dest install
}
```

Source URL tokens `VERSION`, `MAJOR`, `MINOR`, `PATCH`, `ARCH`, `GOARCH` are substituted from the package fields.

## Server Setup

**1. Create an R2 bucket** in the Cloudflare dashboard.

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

# JWT/OIDC for CI
JWT_JWKS_URL=https://token.actions.githubusercontent.com/.well-known/jwks
JWT_ISSUER=https://token.actions.githubusercontent.com
JWT_AUDIENCE=kominka-repo
JWT_SUBJECT_PATTERN=repo:kominka-linux/*
```

**3. Run**

```sh
# Local dev
source .env && make dev

# Production (systemd)
./scripts/build-deb.sh && dpkg -i kominka-repo_*.deb
systemctl enable --now kominka-repo
```

Put a reverse proxy (caddy, nginx) in front for TLS. The server binds `127.0.0.1:3000`.

**4. First login** — open `/auth`, register a passkey, copy the generated `KOMINKA_TOKEN` into `.env`.

## Client Setup

`pm` authenticates with `KOMINKA_TOKEN` in the environment. Get a token at `/auth/settings`.

For CI, GitHub Actions uses a short-lived OIDC token automatically — no stored secret needed. The workflow needs `id-token: write`. Configure `JWT_*` vars on the server to validate it.

## API

### Public

```
GET /health
GET /{arch}/packages.json
GET /{arch}/{pkg}/{ver}-{rel}.tar.gz
GET /src/{pkg}/{ver}-{rel}.tar.bz2
```

### Auth

```
GET  /auth                            → login page
POST /auth/register/options           → start passkey registration
POST /auth/register/verify            → complete registration
POST /auth/authenticate/options       → start authentication
POST /auth/authenticate/verify        → complete authentication
GET  /auth/settings                   → token management page
POST /auth/tokens                     → create token
POST /auth/tokens/delete              → delete token
GET  /auth/logout
```

### Authenticated (`Authorization: Bearer <token>`)

```
POST /api/upload      X-Arch, X-Pkg, X-Ver, X-Rel, X-Deps, X-Mkdeps; body: tarball
POST /api/upload-src  X-Pkg, X-Ver, X-Rel; body: source .tar.bz2
POST /api/upload-url  X-Arch, X-Pkg, X-Ver, X-Rel → presigned PUT URL (large files)
POST /api/update-index  X-Arch, X-Pkg, X-Ver, X-Rel, X-Sha256, X-Deps, X-Mkdeps
POST /api/publish     {"arch","pkg","ver","rel","deps","mkdeps"}  — metapackages
POST /api/reindex     {"arch","pkg","ver","rel"}
POST /api/delete      {"arch","pkg"}
```

## Storage

Tarballs at `{arch}/{pkg}/{ver}-{rel}.tar.gz` in R2. Source tarballs at `src/{pkg}/{ver}-{rel}.tar.bz2`. The index (`packages.json`) tracks sha256 per tarball.
