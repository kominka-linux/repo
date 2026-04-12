#!/usr/local/bin/ysh
# wpa_supplicant action script.
# Called by wpa_supplicant with $1 = event, $IFNAME = interface name.
#
# Drives udhcpc on association events rather than running it
# unconditionally — avoids DHCP before the 4-way handshake completes.

var iface = ENV => get("IFNAME", "wlan0")
var pid_file = "/run/udhcpc-${iface}.pid"

case ($1) {
    CONNECTED {
        # Kill any stale udhcpc for this interface.
        if test -f $pid_file { kill $(cat $pid_file) 2>/dev/null || true }
        # -f: run forever (retry on lease loss).
        udhcpc -b -i $iface -p $pid_file -s /etc/udhcpc.sh \
               -x "hostname:$(hostname)" 2>/dev/null || true
    }
    DISCONNECTED {
        if test -f $pid_file { kill $(cat $pid_file) 2>/dev/null || true }
        rm -f $pid_file
        ip addr flush dev $iface 2>/dev/null || true
    }
}
