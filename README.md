# Kominka Package Repository

Package definitions and repository server for [Kominka Linux](https://github.com/user/davinci).

## Layout

```
packages/       Package definitions (PKGBUILD.ysh files)
server/         Repository server (Rust)
scripts/        Build .deb
```

## Server Setup

### 1. Create an R2 Bucket

In the Cloudflare dashboard:

1. Go to R2 → Create bucket (e.g., `kominka-packages`)
2. Go to R2 → Manage R2 API Tokens → Create API token
3. Choose "Object Read & Write" permissions, scope to your bucket
4. Save the Access Key ID and Secret Access Key

Note the S3 API endpoint — it's `https://<ACCOUNT_ID>.r2.cloudflarestorage.com`.

### 2. Generate an API Key

```sh
openssl rand -hex 32
```

This is the shared secret between the server and `pm`. Anyone with this key
can upload packages.

### 3. Configure

Copy the example env file and fill in credentials:

```sh
cp server/kominka-repo.env.example /etc/kominka-repo/env
chmod 600 /etc/kominka-repo/env
```

```sh
LISTEN_ADDR=127.0.0.1:3000
S3_ENDPOINT=https://<ACCOUNT_ID>.r2.cloudflarestorage.com
S3_BUCKET=kominka-packages
S3_ACCESS_KEY_ID=<from step 1>
S3_SECRET_ACCESS_KEY=<from step 1>
S3_REGION=auto
API_KEY=<from step 2>
```

### 4. Run

Development (source secrets from .env):

```sh
cd server
source ~/d/repo/.env
cargo run
```

Production (systemd):

```sh
# Build and install the .deb
./scripts/build-deb.sh
dpkg -i kominka-repo_*.deb

# Edit /etc/kominka-repo/env with real credentials
systemctl enable --now kominka-repo
```

Put a reverse proxy (caddy, nginx) in front for TLS. The server listens on
`127.0.0.1:3000` with no TLS.

### 5. Populate the Index

After the server is running, publish packages with `pm p`. This handles both
regular packages (uploads the tarball) and metapackages (registers metadata
only):

```sh
# Source credentials from .env, then build+upload each package
source ~/d/repo/.env

# From ~/d/davinci — make targets source .env automatically:
make rebuild-curl
make rebuild-glibc-debian   # packages needing gcc

# Or publish all at once (from ~/d/davinci with KOMINKA_PATH set):
for pkg in packages/*/; do pm p "$(basename "$pkg")"; done
```

## Client Setup (pm)

### Auth

Store the API key so `pm p` can upload:

```sh
# Option A: pm auth (prompts for the key, stores in keychain/file)
KOMINKA_REPO=https://repo.kominka.org pm auth

# Option B: environment variable (for CI)
export KOMINKA_TOKEN=<your API_KEY>
```

Token storage locations:
- macOS: Keychain (`security find-generic-password -s kominka-repo`)
- Linux: `~/.config/kominka/token`

### Usage

```sh
export KOMINKA_REPO=https://repo.kominka.org

# Update the package index
pm u

# Install a package (resolves deps from remote index, no git checkout needed)
pm i curl

# Build and upload a package
pm b curl
pm p curl

# Upload a metapackage
pm p core
```

## API

### Public

```
GET /health                           → {"status":"ok"}
GET /{arch}/packages.json             → package index (JSON)
GET /{arch}/{pkg}/{ver}-{rel}.tar.gz  → tarball
```

### Authenticated (Authorization: Bearer <API_KEY>)

```
POST /api/upload                      → upload tarball
  Headers: X-Arch, X-Pkg, X-Ver, X-Rel, X-Deps, X-Mkdeps
  Body: tarball bytes
  Returns: {"ok":true,"sha256":"..."}

POST /api/publish                     → register metapackage
  Body: {"arch","pkg","ver","rel","deps","mkdeps"}
  Returns: {"ok":true}
```

## Storage Layout

Tarballs are stored at `{arch}/{pkg}/{ver}-{rel}.tar.gz` in R2. The index
tracks the sha256 of each tarball for integrity verification on download.

See `REPOSITORY.md` for the V2 auth design (passkeys + JWT/OIDC, not yet implemented).
