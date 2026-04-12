#!/usr/local/bin/ysh
# Update CA certificate bundle from curl.haxx.se (Mozilla-derived).
# Requires working curl with existing certs (or KOMINKA_INSECURE=1).

if ! test -w /etc/ssl {
    echo "${0##*/}: root required" >&2
    exit 1
}

curl -sfLo /etc/ssl/certs/ca-certificates.crt \
    https://curl.haxx.se/ca/cacert.pem

echo "${0##*/}: updated /etc/ssl/certs/ca-certificates.crt"
