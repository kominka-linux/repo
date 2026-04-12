#!/bin/sh
# wpa_supplicant action script.
# Called by wpa_supplicant with $1 = event, $WPA_ID = network id,
# $IFNAME = interface name.
#
# We use this to start/stop udhcpc on association events rather than
# running udhcpc unconditionally, which avoids DHCP attempts before
# the 4-way handshake completes.

IFACE="${IFNAME:-wlan0}"
PID_FILE="/run/udhcpc-${IFACE}.pid"

case "$1" in
    CONNECTED)
        # Kill any stale udhcpc for this interface.
        [ -f "$PID_FILE" ] && kill "$(cat "$PID_FILE")" 2>/dev/null || true
        # -f: run forever (retry on lease loss), -R: release on exit.
        udhcpc -b -i "$IFACE" -p "$PID_FILE" -s /etc/udhcpc.sh \
               -x hostname:"$(hostname)" 2>/dev/null || true
        ;;
    DISCONNECTED)
        [ -f "$PID_FILE" ] && kill "$(cat "$PID_FILE")" 2>/dev/null || true
        rm -f "$PID_FILE"
        ip addr flush dev "$IFACE" 2>/dev/null || true
        ;;
esac
