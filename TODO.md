# TODO

## Server
- V2 auth: browser passkeys + JWT/OIDC for CI (see REPOSITORY.md for full design)
- Presigned S3 redirect for downloads (currently proxied through server)

## Packages
- Source mirror: fetch and rehost all sources to avoid upstream dependency

## Infrastructure
- Rate limiting on upload endpoint
- Server health monitoring / alerting

---

 Set these secrets in the ~/d/repo GitHub repo settings:
  - KOMINKA_REPO — public URL of your server (e.g. https://repo.kominka.org)
  - KOMINKA_TOKEN — the API key from ~/d/repo/.env

  To rebuild all x86_64 packages, trigger the workflow with use_debian=false
  for most packages, use_debian=true for: glibc, linux, git, strace.

  Keeping pm.ysh in sync: when you update pm.ysh in davinci, copy it to repo
  and update the checksum in packages/pm/PKGBUILD.ysh. We could automate this
   but for now manual sync is fine.

