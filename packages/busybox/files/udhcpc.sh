#!/usr/local/bin/ysh
# udhcpc script — called by udhcpc on lease events.
# $1 = event (bound, renew, deconfig), env vars set by udhcpc.

var event = $1
var iface  = ENV => get("interface", "")
var ip_    = ENV => get("ip", "")
var mask   = ENV => get("mask", "24")
var router = ENV => get("router", "")
var dns_   = ENV => get("dns", "")

case $event {
    bound|renew {
        ip addr flush dev $iface
        ip addr add "${ip_}/${mask}" dev $iface
        if (router !== '') { ip route add default via $router dev $iface }
        if (dns_ !== '') {
            true > /etc/resolv.conf
            for ns in @[split(dns_)] {
                echo "nameserver $ns" >> /etc/resolv.conf
            }
        }
        # Signal that the network is up for dependent services (ntpd etc.).
        mkdir -p /run
        true > /run/network-up
    }
    deconfig {
        ip addr flush dev $iface
        rm -f /run/network-up
    }
}
