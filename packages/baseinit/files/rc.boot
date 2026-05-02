#!/usr/local/bin/ysh
# Kominka Linux boot script.
# Called by seed init as the sysinit action.

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
        mdev -s || true
        fork { mdev -df }
        var pid_mdev = $!
    }
}

log "Remounting rootfs as read-only..." {
    var _root_dev = $(awk '$2 == "/" {print $1; exit}' /proc/mounts)
    mount $_root_dev / -t ext4 -o remount,ro || sos
}


log "Mounting rootfs as read-write..." {
    var _root_dev = $(awk '$2 == "/" {print $1; exit}' /proc/mounts)
    mount $_root_dev / -t ext4 -o remount,rw || sos
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
    ifconfig lo up
}

log "Setting hostname..." {
    if test -f /etc/hostname {
        builtin read --raw-line < /etc/hostname
        echo $_reply > /proc/sys/kernel/hostname 2>/dev/null || true
    }
}

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
        udevadm control --exit 2>/dev/null || true
    } elif builtin test -n "${pid_mdevd:-}" {
        kill $pid_mdevd 2>/dev/null || true
    } elif builtin test -n "${pid_mdev:-}" {
        kill $pid_mdev 2>/dev/null || true
        command -v mdev > /proc/sys/kernel/hotplug 2>/dev/null || true
    }
}

log "Running boot hooks..." {
    run_hook boot
}

builtin read --raw-line < /proc/uptime
var boot_time = _reply.split('.')[0]
log "Boot stage completed in ${boot_time}s..."
