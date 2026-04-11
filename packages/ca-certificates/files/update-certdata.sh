#!/bin/sh -e
# Update CA certificate bundle from curl.haxx.se (Mozilla-derived).
# Requires working curl with existing certs (or KOMINKA_INSECURE=1).

[ -w /etc/ssl ] || {
    printf '%s\n' "${0##*/}: root required" >&2
    exit 1
}

curl -sfLo /etc/ssl/certs/ca-certificates.crt \
    https://curl.haxx.se/ca/cacert.pem

printf '%s\n' "${0##*/}: updated /etc/ssl/certs/ca-certificates.crt"
