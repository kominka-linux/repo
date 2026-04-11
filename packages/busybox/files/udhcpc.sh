#!/bin/sh
# udhcpc script — called by udhcpc on lease events.

case $1 in
    bound|renew)
        ip addr flush dev "$interface"
        ip addr add "$ip/${mask:-24}" dev "$interface"
        [ -n "$router" ] && ip route add default via "$router" dev "$interface"
        if [ -n "$dns" ]; then
            : > /etc/resolv.conf
            for ns in $dns; do
                echo "nameserver $ns" >> /etc/resolv.conf
            done
        fi
        # Signal that the network is up for dependent services (ntpd, etc.).
        mkdir -p /run
        : > /run/network-up
        ;;
    deconfig)
        ip addr flush dev "$interface"
        rm -f /run/network-up
        ;;
esac
