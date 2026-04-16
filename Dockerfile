# Build environment: core + build-essential
# Bootstrap downloads ysh from R2 and seed from GitHub releases into
# kominka-root, then pm i runs inside the scratch container.
#
# KARCH: architecture string passed by the caller.
#   x86_64-linux-gnu  or  aarch64-linux-gnu
#   (macOS arm64 → aarch64-linux-gnu)
#
# Usage:
#   KARCH=$(uname -m | sed 's/x86_64/x86_64-linux-gnu/;s/aarch64/aarch64-linux-gnu/;s/arm64/aarch64-linux-gnu/')
#   docker build --build-arg KARCH=$KARCH -t kominka:core .

FROM alpine:latest AS bootstrap

ARG KARCH=aarch64-linux-gnu
ARG SEED_VER=2138a1ae863d05591ecda011703364ebc0f05958
ARG REPO_URL=
ARG R2_PUBLIC_URL=https://pub-15b3a4c25627476493c0e1a68993f4d8.r2.dev

ADD packages/ca-certificates/files/cacert.pem /etc/ssl/certs/ca-certificates.crt

COPY pm.ysh /usr/bin/pm

RUN mkdir -p /kominka-root/usr/bin /kominka-root/etc/ssl/certs && \
    wget -q -O - "$R2_PUBLIC_URL/$KARCH/ysh/0.37.0-4.tar.gz" | tar xzf - -C /kominka-root/ && \
    SARCH=$(echo "$KARCH" | cut -d- -f1) && \
    wget -q -O - "https://github.com/kominka-linux/seed/releases/download/seed-$SEED_VER/seed-linux-$SARCH.tar.gz" | tar xzf - --strip-components=1 -C /kominka-root/usr/ && \
    cp /usr/bin/pm /kominka-root/usr/bin/pm && \
    find /kominka-root/usr/bin /kominka-root/usr/local/bin -maxdepth 1 -type f -exec chmod +x {} + && \
    cp /etc/ssl/certs/ca-certificates.crt /kominka-root/etc/ssl/certs/ca-certificates.crt

FROM scratch

COPY --from=bootstrap /kominka-root /

SHELL ["/usr/local/bin/ysh", "-c"]

ARG REPO_URL=
ARG R2_PUBLIC_URL=https://pub-15b3a4c25627476493c0e1a68993f4d8.r2.dev

ENV KOMINKA_REPO=${REPO_URL} \
    R2_PUBLIC_URL=${R2_PUBLIC_URL} \
    HOME=/root \
    LOGNAME=root \
    PATH=/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin \
    KOMINKA_PATH=/usr/lib/kominka/packages

RUN KOMINKA_COLOR=0 KOMINKA_PROMPT=0 KOMINKA_STRIP=0 KOMINKA_FORCE=1 \
    pm i core build-essential && \
    find /usr/bin /usr/local/bin -maxdepth 1 -type f -exec chmod +x {} +

CMD ["/bin/sh"]
