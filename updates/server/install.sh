#!/bin/sh
# Installs the Beans server updater on a Linux host, as root, from a checkout of this repository:
# the script, the Beans public key, and the systemd service and timer. It never writes the
# config: copy updates/server/server-updater.example.json to /etc/beans/server-updater.json,
# name your relay and Runners, and keep it root-owned with mode 0600. It never enables the timer:
# run 'systemctl enable --now beans-server-updater.timer' yourself once the config is right.
set -eu

if [ "$(id -u)" -ne 0 ]; then
    echo "install.sh: run as root" >&2
    exit 1
fi
command -v python3 >/dev/null || { echo "install.sh: python3 is required" >&2; exit 1; }
command -v openssl >/dev/null || { echo "install.sh: openssl is required" >&2; exit 1; }
command -v systemctl >/dev/null || { echo "install.sh: systemd is required" >&2; exit 1; }

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$here/../.." && pwd)

# Reject symlink/writable destination ancestry before root installs executable code.
check_directories() {
    directory=$1
    while :; do
        if [ -L "$directory" ] || [ ! -d "$directory" ] ||
           [ "$(stat -c %u -- "$directory")" != 0 ]; then
            echo "install.sh: $directory must be a real root-owned directory" >&2
            exit 1
        fi
        mode="0$(stat -c %a -- "$directory")"
        if [ "$((mode & 022))" -ne 0 ]; then
            echo "install.sh: $directory must not be writable by group or others" >&2
            exit 1
        fi
        [ "$directory" != / ] || break
        directory=${directory%/*}
        [ -n "$directory" ] || directory=/
    done
}

check_file() {
    file=$1
    mask=$2
    if [ -e "$file" ] || [ -L "$file" ]; then
        if [ -L "$file" ] || [ ! -f "$file" ] || [ "$(stat -c %u -- "$file")" != 0 ]; then
            echo "install.sh: $file must be a regular root-owned file, without symlinks" >&2
            exit 1
        fi
        mode="0$(stat -c %a -- "$file")"
        if [ "$((mode & mask))" -ne 0 ]; then
            echo "install.sh: unsafe permissions on $file; repair them explicitly" >&2
            exit 1
        fi
    fi
}

check_directories /usr/local/lib
check_directories /etc/systemd/system
for directory in /usr/local/lib/beans /etc/beans; do
    if [ -e "$directory" ] || [ -L "$directory" ]; then
        check_directories "$directory"
    fi
done
check_file /usr/local/lib/beans/server-updater.py 022
check_file /etc/beans/update-public-key.txt 022
check_file /etc/beans/server-updater.json 077
check_file /etc/systemd/system/beans-server-updater.service 022
check_file /etc/systemd/system/beans-server-updater.timer 022

install -d -o root -g root -m 0755 /usr/local/lib/beans
install -o root -g root -m 0555 "$root/scripts/server-updater.py" /usr/local/lib/beans/server-updater.py
install -d -o root -g root -m 0755 /etc/beans
install -o root -g root -m 0644 "$root/updates/public-key.txt" /etc/beans/update-public-key.txt
install -o root -g root -m 0644 "$here/beans-server-updater.service" /etc/systemd/system/beans-server-updater.service
install -o root -g root -m 0644 "$here/beans-server-updater.timer" /etc/systemd/system/beans-server-updater.timer
systemctl daemon-reload

echo "install.sh: installed; write /etc/beans/server-updater.json (mode 0600), try 'systemctl start beans-server-updater.service', then run 'systemctl enable --now beans-server-updater.timer'"
