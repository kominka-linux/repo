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

# Minimal bootstrap environment: only musl + baselayout + busybox + ysh needed
# to run pm. All other packages are pre-cached below so pm never calls curl.
RUN mkdir -p /pkg && \
    wget --no-check-certificate -qO- "$R2/$KARCH/musl/1.2.6-24.tar.gz"      | tar xzf - -C /pkg && \
    wget --no-check-certificate -qO- "$R2/$KARCH/baselayout/1-9.tar.gz"       | tar xzf - -C /pkg && \
    wget --no-check-certificate -qO- "$R2/$KARCH/busybox/1.36.1-12.tar.gz"    | tar xzf - -C /pkg && \
    wget --no-check-certificate -qO- "$R2/$KARCH/ysh/0.37.0-4.tar.gz"         | tar xzf - -C /pkg

# Pre-populate pm binary cache with all packages needed for both:
#   1. pm i core (kominka:core base packages)
#   2. pm i zig busybox make binutils-strip (kominka:build-essential tools)
# pm checks pkg@ver-rel.tar.gz in $XDG_CACHE_HOME/kominka/bin/ before downloading.
# Pre-seeding bypasses the HTTPS download entirely — critical because our x86_64
# boringssl crashes with SIGSEGV during SSL_library_init() (zig cc x86_64 bug).
# Update versions here whenever any of these packages change.
RUN mkdir -p /cache && \
    wget --no-check-certificate -qO "/cache/musl@1.2.6-24.tar.gz"               "$R2/$KARCH/musl/1.2.6-24.tar.gz" && \
    wget --no-check-certificate -qO "/cache/baselayout@1-9.tar.gz"               "$R2/$KARCH/baselayout/1-9.tar.gz" && \
    wget --no-check-certificate -qO "/cache/busybox@1.36.1-12.tar.gz"            "$R2/$KARCH/busybox/1.36.1-12.tar.gz" && \
    wget --no-check-certificate -qO "/cache/ysh@0.37.0-4.tar.gz"                 "$R2/$KARCH/ysh/0.37.0-4.tar.gz" && \
    wget --no-check-certificate -qO "/cache/zlib@1.3.2-4.tar.gz"                 "$R2/$KARCH/zlib/1.3.2-4.tar.gz" && \
    wget --no-check-certificate -qO "/cache/boringssl@0.20260327.0-9.tar.gz"     "$R2/$KARCH/boringssl/0.20260327.0-9.tar.gz" && \
    wget --no-check-certificate -qO "/cache/curl@8.19.0-9.tar.gz"                "$R2/$KARCH/curl/8.19.0-9.tar.gz" && \
    wget --no-check-certificate -qO "/cache/mimalloc@2.2.7-1.tar.gz"             "$R2/$KARCH/mimalloc/2.2.7-1.tar.gz" && \
    wget --no-check-certificate -qO "/cache/baseinit@2.0.0-1.tar.gz"             "$R2/$KARCH/baseinit/2.0.0-1.tar.gz" && \
    wget --no-check-certificate -qO "/cache/runit@2.3.1-2.tar.gz"                "$R2/$KARCH/runit/2.3.1-2.tar.gz" && \
    wget --no-check-certificate -qO "/cache/ca-certificates@2026.03.19-1.tar.gz" "$R2/$KARCH/ca-certificates/2026.03.19-1.tar.gz" && \
    wget --no-check-certificate -qO "/cache/zig@0.15.2-11.tar.gz"                "$R2/$KARCH/zig/0.15.2-11.tar.gz" && \
    wget --no-check-certificate -qO "/cache/make@4.4.1-3.tar.gz"                 "$R2/$KARCH/make/4.4.1-3.tar.gz" && \
    wget --no-check-certificate -qO "/cache/binutils-strip@2.44-2.tar.gz"        "$R2/$KARCH/binutils-strip/2.44-2.tar.gz"

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

# Seed pm binary cache inside /kominka-root (persists to final image).
# XDG_CACHE_HOME=/kominka-root/root/.cache in pm i core below means the
# pm cache lives at /kominka-root/root/.cache/kominka/bin/ = /root/.cache/...
# in the final image. pm finds all tarballs there and skips all downloads.
COPY --from=fetch /cache /kominka-root/root/.cache/kominka/bin/

# Embed build-essential PKGBUILDs in the image so pm can resolve versions
# without network access. busybox is in core; only zig/make/binutils-strip.
RUN mkdir -p \
        /kominka-root/usr/lib/kominka/packages/zig \
        /kominka-root/usr/lib/kominka/packages/make \
        /kominka-root/usr/lib/kominka/packages/binutils-strip && \
    cp /packages/zig/PKGBUILD.ysh \
        /kominka-root/usr/lib/kominka/packages/zig/PKGBUILD.ysh && \
    cp /packages/make/PKGBUILD.ysh \
        /kominka-root/usr/lib/kominka/packages/make/PKGBUILD.ysh && \
    cp /packages/binutils-strip/PKGBUILD.ysh \
        /kominka-root/usr/lib/kominka/packages/binutils-strip/PKGBUILD.ysh

RUN mkdir -p /kominka-root/var/db/kominka/installed \
             /kominka-root/var/db/kominka/choices

RUN XDG_CACHE_HOME=/kominka-root/root/.cache \
    KOMINKA_PATH=/packages \
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
    LOGNAME=root \
    KOMINKA_PATH=/usr/lib/kominka/packages

CMD ["/bin/sh"]
