#!/bin/sh
# Run from a live system to install the prepared compositor into an offline root.
set -eu

root=$(readlink -f "${1:?Usage: sudo sh scripts/repair-mounted-session.sh /mnt/desui-system}")
project=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
artifact="$project/target/recovery"

[ "$(id -u)" -eq 0 ] || { echo "Run this script with sudo." >&2; exit 1; }
[ "$root" != / ] && mountpoint -q "$root" || {
    echo "Expected a separately mounted offline system root." >&2; exit 1;
}
[ -f "$root/etc/os-release" ] && [ -x "$root/usr/local/bin/des-comp" ] || {
    echo "The mounted system has no /usr/local/bin/des-comp to repair." >&2; exit 1;
}
(cd "$artifact" && sha256sum -c SHA256SUMS)

was_readonly=false
case ",$(findmnt -n -o OPTIONS --target "$root")," in
    *,ro,*) was_readonly=true; mount -o remount,rw "$root" ;;
esac
temporary="$root/usr/local/bin/.des-comp-repair-$$"
cleanup() {
    rm -f "$temporary"
    if [ "$was_readonly" = true ]; then
        mount -o remount,ro "$root"
    fi
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

backup="$root/usr/local/lib/des-ui/recovery-$(date -u +%Y%m%dT%H%M%SZ)-$$"
install -d -m755 "$backup"
cp -p "$root/usr/local/bin/des-comp" "$backup/des-comp"
install -m755 "$artifact/des-comp" "$temporary"
mv -f "$temporary" "$root/usr/local/bin/des-comp"
sync -f "$root/usr/local/bin/des-comp"
echo "Installed the repaired desktop compositor."
echo "Previous binary: $backup/des-comp"
echo "You can reboot into the installed system and select DEs-UI."
