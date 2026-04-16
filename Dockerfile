# Build environment: core + build-essential
# Bootstrap installs ysh + seed (via pm, statically linked) + wget + pm
# into kominka-root. pm i then runs inside the scratch container.
#
# KARCH: architecture string passed by the caller.
#   x86_64-linux-gnu  or  aarch64-linux-gnu
#   (macOS arm64 → aarch64-linux-gnu)
#
# Usage:
#   KARCH=$(uname -m | sed 's/x86_64/x86_64-linux-gnu/;s/aarch64/aarch64-linux-gnu/;s/arm64/aarch64-linux-gnu/')
#   docker build --build-arg KARCH=$KARCH -t kominka:core .

FROM busybox:latest AS bootstrap

ARG KARCH=aarch64-linux-gnu
ARG WGET_TAG=wget-c5c83721bb3ab246692318c9c279adea76899aee
ARG REPO_URL=
ARG R2_PUBLIC_URL=https://pub-15b3a4c25627476493c0e1a68993f4d8.r2.dev

# Download and verify static wget.
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

# All steps using $KARCH / $R2_PUBLIC_URL run under busybox sh (before SHELL
# switches to ysh, which doesn't expose Docker ARGs as $VAR).
RUN /usr/bin/wget -q -O - "$R2_PUBLIC_URL/$KARCH/ysh/0.37.0-4.tar.gz" | tar xzf - -C /

COPY pm.ysh /usr/bin/pm

RUN chmod +x /usr/bin/pm && \
    mkdir -p /kominka-root/usr/bin /kominka-root/etc/ssl/certs && \
    /usr/bin/wget -q -O - "$R2_PUBLIC_URL/$KARCH/ysh/0.37.0-4.tar.gz" | tar xzf - -C /kominka-root/ && \
    cp /usr/bin/wget /kominka-root/usr/bin/wget && \
    cp /usr/bin/pm /kominka-root/usr/bin/pm && \
    cp /etc/ssl/certs/ca-certificates.crt /kominka-root/etc/ssl/certs/ca-certificates.crt

SHELL ["/usr/local/bin/ysh", "-c"]

ENV KOMINKA_REPO=${REPO_URL} \
    R2_PUBLIC_URL=${R2_PUBLIC_URL} \
    LOGNAME=root \
    HOME=/root

# Install seed into kominka-root via pm (statically linked multicall binary).
# seed includes wget; copy our TLS-capable wget over the applet so scratch
# has working HTTPS for pm i.
RUN KOMINKA_ROOT=/kominka-root \
    KOMINKA_COLOR=0 KOMINKA_PROMPT=0 KOMINKA_STRIP=0 KOMINKA_FORCE=1 \
    pm i seed && \
    cp /usr/bin/wget /kominka-root/usr/bin/wget

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
    pm i core build-essential

CMD ["/bin/sh"]
