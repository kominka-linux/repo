# Build environment: core + build-essential
# Used by CI for all package builds.
#
# Bootstrap: busybox:latest provides sh + tar during the build only.
# The final scratch stage copies only Kominka packages — no Docker Hub
# content ends up in the final image.
#
# KARCH: architecture string passed by the caller.
#   x86_64-linux-gnu  or  aarch64-linux-gnu
#
# Usage:
#   KARCH=$(uname -m | sed 's/x86_64/x86_64-linux-gnu/;s/aarch64/aarch64-linux-gnu/;s/arm64/aarch64-linux-gnu/')
#   docker build --build-arg KARCH=$KARCH -t kominka:core .

FROM busybox:latest AS bootstrap

ARG KARCH=aarch64-linux-gnu
ARG WGET_TAG=wget-9839bca87f17e4e67c79590c58c180653f477e18
ARG REPO_URL=
ARG R2_PUBLIC_URL=https://pub-15b3a4c25627476493c0e1a68993f4d8.r2.dev

# Download our static wget binary; replaces busybox wget for all pm operations.
RUN WARCH=$(echo "$KARCH" | cut -d- -f1) && \
    wget -q -O "wget-linux-$WARCH" \
      "https://github.com/kominka-linux/seed/releases/download/$WGET_TAG/wget-linux-$WARCH" && \
    wget -q -O "wget-linux-$WARCH.sha256" \
      "https://github.com/kominka-linux/seed/releases/download/$WGET_TAG/wget-linux-$WARCH.sha256" && \
    awk -v f="wget-linux-$WARCH" '{print $1 "  " f}' "wget-linux-$WARCH.sha256" | sha256sum -c && \
    mv "wget-linux-$WARCH" /usr/bin/wget && \
    rm "wget-linux-$WARCH.sha256" && \
    chmod +x /usr/bin/wget

ADD packages/ca-certificates/files/cacert.pem /etc/ssl/certs/ca-certificates.crt

# Install ysh (static binary, used as the shell for pm).
RUN /usr/bin/wget -q -O - "$R2_PUBLIC_URL/$KARCH/ysh/0.37.0-4.tar.gz" | tar xzf - -C /

# ysh is statically compiled; use it as the shell for all RUN commands.
SHELL ["/usr/local/bin/ysh", "-c"]

# Promote ARGs to ENV so they're available without ${} substitution in ysh.
ENV KOMINKA_REPO=${REPO_URL} \
    R2_PUBLIC_URL=${R2_PUBLIC_URL} \
    KOMINKA_GET=/usr/bin/wget \
    LOGNAME=root \
    HOME=/root

COPY pm.ysh /usr/bin/pm
RUN chmod +x /usr/bin/pm

RUN XDG_CACHE_HOME=/kominka-root/root/.cache \
    KOMINKA_ROOT=/kominka-root \
    KOMINKA_COMPRESS=gz \
    KOMINKA_COLOR=0 \
    KOMINKA_PROMPT=0 \
    KOMINKA_STRIP=0 \
    KOMINKA_FORCE=1 \
    ysh /usr/bin/pm i core

RUN cp /usr/bin/pm /kominka-root/usr/bin/pm && \
    cp /usr/bin/wget /kominka-root/usr/bin/wget

FROM scratch

COPY --from=bootstrap /kominka-root /

ENV PATH=/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin \
    HOME=/root \
    LOGNAME=root \
    KOMINKA_PATH=/usr/lib/kominka/packages

CMD ["/bin/sh"]
