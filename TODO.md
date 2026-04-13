# TODO

## Server
- Presigned S3 redirect for downloads (currently proxied through server)
- Rate limiting on auth and upload endpoints
- `pm auth` CLI flow: generate session ID, open `/auth?session={id}` in browser, poll `/auth/poll`

## Packages
- Source mirror: fetch and rehost all sources to avoid upstream dependency
- builds should be done in a linux namespace container and in a maximally reproducible manner

## Infrastructure
- Server health monitoring / alerting

