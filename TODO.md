# TODO

## Server
- V2 auth: browser passkeys + JWT/OIDC for CI (see REPOSITORY.md for full design)
- Presigned S3 redirect for downloads (currently proxied through server)
- Stale tarball cleanup script (diff R2 objects against index, delete orphans)
- x86_64 package builds (only aarch64 built so far)

## Packages
- `kominka`/`pm` package: needs proper PKGBUILD pointing at pm.ysh
- `cargo` and `go`: large binaries, consider whether they belong in build-essential
- Source mirror: fetch and rehost all sources to avoid upstream dependency
- Kernel headers: consolidate linux-headers versioning

## Infrastructure
- Rate limiting on upload endpoint
- Server health monitoring / alerting
