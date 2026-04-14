# Kominka Linux base image (FROM scratch, ~57MB).
#
# Bootstrap: busybox:latest is used ONLY to download our own packages from R2
# via wget. The scratch stage copies only our packages — no Docker Hub files
# end up in the final image or in any stage that actually executes our code.
#
# KARCH: architecture string passed by the caller.
#   x86_64-linux-gnu  or  aarch64-linux-gnu
#
# Usage:
#   KARCH=$(uname -m | sed 's/x86_64/x86_64-linux-gnu/;s/aarch64/aarch64-linux-gnu/')
#   docker build --build-context packages=<dir> --build-context pm=<pm dir> \
#     --build-arg KARCH=$KARCH --build-arg REPO_URL=http://localhost:3000 \
#     --network=host -t kominka:core .

FROM busybox:latest AS fetch

ARG R2=https://pub-15b3a4c25627476493c0e1a68993f4d8.r2.dev
ARG KARCH=aarch64-linux-gnu

# Minimal bootstrap environment: musl + baselayout + busybox + ysh.
# zlib/boringssl/curl are no longer extracted here — they are pre-cached
# below so pm i core can install them without making any HTTPS calls.
RUN mkdir -p /pkg && \
    wget --no-check-certificate -qO- "$R2/$KARCH/musl/1.2.6-23.tar.gz"      | tar xzf - -C /pkg && \
    wget --no-check-certificate -qO- "$R2/$KARCH/baselayout/1-9.tar.gz"       | tar xzf - -C /pkg && \
    wget --no-check-certificate -qO- "$R2/$KARCH/busybox/1.36.1-12.tar.gz"    | tar xzf - -C /pkg && \
    wget --no-check-certificate -qO- "$R2/$KARCH/ysh/0.37.0-4.tar.gz"         | tar xzf - -C /pkg

# Pre-populate pm's binary cache with all core packages.
# pm i core checks for pkg@ver-rel.tar.gz in $HOME/.cache/kominka/bin/ before
# downloading. Pre-seeding the cache means pm never calls curl — avoiding the
# HTTPS dependency during bootstrap (our curl crashes on x86_64 during SSL init).
# Update versions here whenever any of these packages change.
RUN mkdir -p /cache && \
    wget --no-check-certificate -qO "/cache/musl@1.2.6-23.tar.gz"               "$R2/$KARCH/musl/1.2.6-23.tar.gz" && \
    wget --no-check-certificate -qO "/cache/baselayout@1-9.tar.gz"               "$R2/$KARCH/baselayout/1-9.tar.gz" && \
    wget --no-check-certificate -qO "/cache/busybox@1.36.1-12.tar.gz"            "$R2/$KARCH/busybox/1.36.1-12.tar.gz" && \
    wget --no-check-certificate -qO "/cache/ysh@0.37.0-4.tar.gz"                 "$R2/$KARCH/ysh/0.37.0-4.tar.gz" && \
    wget --no-check-certificate -qO "/cache/zlib@1.3.2-4.tar.gz"                 "$R2/$KARCH/zlib/1.3.2-4.tar.gz" && \
    wget --no-check-certificate -qO "/cache/boringssl@0.20260327.0-9.tar.gz"     "$R2/$KARCH/boringssl/0.20260327.0-9.tar.gz" && \
    wget --no-check-certificate -qO "/cache/curl@8.19.0-9.tar.gz"                "$R2/$KARCH/curl/8.19.0-9.tar.gz" && \
    wget --no-check-certificate -qO "/cache/mimalloc@2.2.7-1.tar.gz"             "$R2/$KARCH/mimalloc/2.2.7-1.tar.gz" && \
    wget --no-check-certificate -qO "/cache/baseinit@2.0.0-1.tar.gz"             "$R2/$KARCH/baseinit/2.0.0-1.tar.gz" && \
    wget --no-check-certificate -qO "/cache/runit@2.3.1-2.tar.gz"                "$R2/$KARCH/runit/2.3.1-2.tar.gz" && \
    wget --no-check-certificate -qO "/cache/ca-certificates@2026.03.19-1.tar.gz" "$R2/$KARCH/ca-certificates/2026.03.19-1.tar.gz"

FROM scratch AS bootstrap

ARG REPO_URL=
ARG R2_PUBLIC_URL=https://pub-15b3a4c25627476493c0e1a68993f4d8.r2.dev

# Copy only our own packages — no Docker Hub files in this stage.
COPY --from=fetch /pkg /

# ysh is statically compiled; use it as the shell for all RUN commands.
SHELL ["/usr/local/bin/ysh", "-c"]

# Promote ARGs to ENV so they're available without ${} substitution in ysh.
ENV KOMINKA_REPO=${REPO_URL} \
    R2_PUBLIC_URL=${R2_PUBLIC_URL} \
    KOMINKA_GET=/usr/bin/curl \
    LOGNAME=root \
    HOME=/root

COPY --from=pm pm.ysh /usr/bin/pm
RUN chmod +x /usr/bin/pm
COPY --from=packages / /packages
RUN find /packages -name build -exec chmod +x {} + && \
    find /packages -name post-install -exec chmod +x {} +

# Seed pm binary cache — pm i core finds all packages here and skips downloads.
COPY --from=fetch /cache /root/.cache/kominka/bin/

RUN mkdir -p /kominka-root/var/db/kominka/installed \
             /kominka-root/var/db/kominka/choices

RUN KOMINKA_PATH=/packages \
    KOMINKA_ROOT=/kominka-root \
    KOMINKA_COMPRESS=gz \
    KOMINKA_COLOR=0 \
    KOMINKA_PROMPT=0 \
    KOMINKA_STRIP=0 \
    KOMINKA_FORCE=1 \
    KOMINKA_INSECURE=1 \
    ysh /usr/bin/pm i core

RUN cp /usr/bin/pm /kominka-root/usr/bin/pm

FROM scratch

COPY --from=bootstrap /kominka-root /

ENV PATH=/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin \
    HOME=/root \
    LOGNAME=root

CMD ["/bin/sh"]
