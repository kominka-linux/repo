# Kominka Linux base image (FROM scratch, ~57MB).
#
# Bootstrap: busybox:latest (4MB static musl) provides wget+tar+sh.
# ysh is static musl — runs directly on the busybox base.
# pm resolves and installs all packages from the repo server.
#
# Usage:
#   make core   (uses repo server at localhost:3000, --network=host)
#   docker build --build-context packages=<dir> --network=host \
#     --build-arg REPO_URL=http://localhost:3000 -t kominka:core .

FROM busybox:latest AS bootstrap

ARG R2_BASE=https://pub-15b3a4c25627476493c0e1a68993f4d8.r2.dev
ARG REPO_URL=

# Detect architecture.
RUN case "$(busybox uname -m)" in \
        x86_64)  echo "x86_64-linux-gnu" > /tmp/arch ;; \
        *)       echo "aarch64-linux-gnu" > /tmp/arch ;; \
    esac

# Get ysh (static musl binary — runs on any Linux, no glibc needed).
RUN busybox mkdir -p /usr/local/bin && \
    ARCH=$(busybox cat /tmp/arch) && \
    busybox wget --no-check-certificate -qO- "$R2_BASE/$ARCH/ysh/0.37.0-2.tar.gz" | \
    busybox tar xzf - -C / ./usr/local/bin/

# Install pm and package definitions.
COPY pm.ysh /usr/bin/pm
RUN busybox chmod +x /usr/bin/pm
COPY --from=packages / /packages
RUN busybox find /packages -name build -exec busybox chmod +x {} + && \
    busybox find /packages -name post-install -exec busybox chmod +x {} +

RUN busybox mkdir -p /kominka-root/var/db/kominka/installed \
                     /kominka-root/var/db/kominka/choices

# Install core from the repo server.
RUN KOMINKA_REPO=${REPO_URL} \
    KOMINKA_PATH=/packages \
    KOMINKA_ROOT=/kominka-root \
    KOMINKA_COMPRESS=gz \
    KOMINKA_COLOR=0 \
    KOMINKA_PROMPT=0 \
    KOMINKA_STRIP=0 \
    KOMINKA_FORCE=1 \
    KOMINKA_INSECURE=1 \
    LOGNAME=root \
    HOME=/root \
    ysh /usr/bin/pm i core

RUN busybox cp /usr/bin/pm /kominka-root/usr/bin/pm

FROM scratch

COPY --from=bootstrap /kominka-root /

ENV PATH=/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin \
    HOME=/root \
    LOGNAME=root

CMD ["/bin/sh"]
