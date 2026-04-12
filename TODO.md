# TODO

## Server
- Presigned S3 redirect for downloads (currently proxied through server)
- Rate limiting on auth and upload endpoints
- `pm auth` CLI flow: generate session ID, open `/auth?session={id}` in browser, poll `/auth/poll`

## Packages
- Source mirror: fetch and rehost all sources to avoid upstream dependency

## Infrastructure
- Server health monitoring / alerting

---

GitHub repo secrets needed:
- KOMINKA_REPO — public URL of the server (e.g. https://repo.kominka.org)

CI uses GitHub OIDC for auth — no KOMINKA_TOKEN secret needed.

Keeping pm.ysh in sync: when you update pm.ysh in davinci, copy it to repo
and update the checksum in packages/pm/PKGBUILD.ysh.
