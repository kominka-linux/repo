#!/usr/local/bin/ysh
# Kominka Linux boot script.
# Called by busybox init as the sysinit action.

source /usr/lib/init/rc.lib

log "Welcome to Kominka!"

log "Mounting pseudo filesystems..." {
    mnt nosuid,noexec,nodev    proc     proc /proc
    mnt nosuid,noexec,nodev    sysfs    sys  /sys
    mnt mode=0755,nosuid,nodev tmpfs    run  /run
    mnt mode=0755,nosuid       devtmpfs dev  /dev

    mkdir -p /run/runit /run/user /run/lock \
             /run/log   /dev/pts  /dev/shm

    mnt mode=0620,gid=5,nosuid,noexec devpts devpts /dev/pts
    mnt mode=1777,nosuid,nodev        tmpfs  shm    /dev/shm

    ln -s /proc/self/fd /dev/fd     2>/dev/null || true
    ln -s fd/0          /dev/stdin  2>/dev/null || true
    ln -s fd/1          /dev/stdout 2>/dev/null || true
    ln -s fd/2          /dev/stderr 2>/dev/null || true
}

log "Loading rc.conf settings..." {
    if test -f /etc/rc.conf { source /etc/rc.conf }
}

log "Starting device manager..." {
    if command -v udevd >/dev/null 2>&1 {
        udevd -d
        udevadm trigger -c add -t subsystems
        udevadm trigger -c add -t devices
        udevadm settle
    } elif command -v mdevd >/dev/null 2>&1 {
        fork { mdevd }
        var pid_mdevd = $!
        mdevd-coldplug
    } elif command -v mdev >/dev/null 2>&1 {
        mdev -s
        fork { mdev -df }
        var pid_mdev = $!
    }
}

log "Remounting rootfs as read-only..." {
    mount -o remount,ro / || sos
}

log "Checking filesystems..." {
    if command -v fsck.ext4 >/dev/null 2>&1 {
        fsck -pATat noopts=_netdev
        if ($? > 1) { sos }
    }
}

log "Mounting rootfs as read-write..." {
    mount -o remount,rw / || sos
}

log "Mounting all local filesystems..." {
    mount -a || sos
}

log "Enabling swap..." {
    swapon -a || true
}

log "Seeding random..." {
    random_seed load
}

log "Setting up loopback..." {
    ip link set up dev lo
}

log "Setting hostname..." {
    if test -f /etc/hostname {
        read --line < /etc/hostname
        echo $_reply > /proc/sys/kernel/hostname
    }
} 2>/dev/null || true

log "Loading sysctl settings..." {
    var seen = ''
    for conf in @[glob('/run/sysctl.d/*.conf')] \
                @[glob('/etc/sysctl.d/*.conf')] \
                @[glob('/usr/lib/sysctl.d/*.conf')] {
        if ! test -f $conf { continue }
        var base = $(basename $conf)
        if (seen ~ " $base ") { continue }
        setvar seen = "$seen $base "
        sysctl -p $conf
    }
    if test -f /etc/sysctl.conf {
        var base = 'sysctl.conf'
        if not (seen ~ " $base ") { sysctl -p /etc/sysctl.conf }
    }
}

log "Killing device manager to make way for services..." {
    if command -v udevd >/dev/null 2>&1 {
        udevadm control --exit
    } elif test -n ${pid_mdevd:-} {
        kill $pid_mdevd
    } elif test -n ${pid_mdev:-} {
        kill $pid_mdev
        command -v mdev > /proc/sys/kernel/hotplug
    }
} 2>/dev/null || true

log "Running boot hooks..." {
    run_hook boot
}

read --line < /proc/uptime
var boot_time = _reply.split('.')[0]
log "Boot stage completed in ${boot_time}s..."
