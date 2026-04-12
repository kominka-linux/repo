#!/usr/local/bin/ysh
# Update the CA certificate bundle from curl.haxx.se.

if ! test -w /etc/ssl {
    echo "${0##*/}: root required to update cert." >&2
    exit 1
}

cd /etc/ssl
curl -LO https://curl.haxx.se/ca/cacert.pem
mv -f cacert.pem cert.pem
echo "${0##*/}: updated cert.pem"
