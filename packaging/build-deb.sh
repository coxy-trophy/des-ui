#!/bin/sh
# Builds the Debian packages into target/deb/: des-ui (the desktop), des-ui-music
# (the Spotify player), des-ui-greeter (the login screen) and des-ui-plymouth (the
# boot splash). The last three are apart so that each can be left out.
#
#     packaging/build-deb.sh              build the programs, then the packages
#     packaging/build-deb.sh --no-build   package what target/release already holds
#
# A package only suits the release of Ubuntu or Debian it was built on: the libraries
# it asks for are the ones found here.
set -eu
cd "$(dirname "$0")/.."

[ "${1:-}" = --no-build ] || cargo build --release

version=$(sed -n 's/^version *= *"\(.*\)"/\1/p' shell/Cargo.toml | head -n 1)
arch=$(dpkg --print-architecture)
maintainer="${DEB_MAINTAINER:-$(git config user.name) <$(git config user.email)>}"
out=target/deb
rm -rf "$out"
mkdir -p "$out"

# The packages holding the libraries these programs are linked against.
linked() {
    work=$(mktemp -d)
    mkdir "$work/debian"
    printf 'Source: des-ui\n\nPackage: des-ui\nArchitecture: any\n' >"$work/debian/control"
    for program; do set -- "$@" "-e$PWD/$program"; shift; done
    (cd "$work" && dpkg-shlibdeps -O "$@" 2>/dev/null) | sed 's/^shlibs:Depends=//'
    rm -rf "$work"
}

# pack <name> <depends> <recommends> <suggests> <summary>; the rest of the
# description is read from standard input.
pack() {
    root="$out/$1"
    # Whatever the builder's umask: nothing writable by a group, bar the sticky folder.
    find "$root" ! -perm -1000 -exec chmod go-w {} +
    install -Dm644 LICENSE "$root/usr/share/doc/$1/copyright"
    mkdir -p "$root/DEBIAN"
    {
        echo "Package: $1"
        echo "Version: $version"
        echo "Architecture: $arch"
        echo "Maintainer: $maintainer"
        echo "Installed-Size: $(du -sk --exclude=DEBIAN "$root" | cut -f1)"
        echo "Depends: $2"
        [ -z "$3" ] || echo "Recommends: $3"
        [ -z "$4" ] || echo "Suggests: $4"
        echo "Section: x11"
        echo "Priority: optional"
        echo "Homepage: https://github.com/coxy-trophy/des-ui"
        echo "Description: $5"
        sed 's/^$/./; s/^/ /'
    } >"$root/DEBIAN/control"
    # What to run when the package goes on or comes off, where there is something.
    for script in "packaging/$1"/*; do
        [ -f "$script" ] && install -m755 "$script" "$root/DEBIAN/"
    done
    (cd "$root" && find usr -type f -exec md5sum {} + >DEBIAN/md5sums)
    dpkg-deb --root-owner-group --build "$root" "$out/${1}_${version}_${arch}.deb" >/dev/null
    rm -rf "$root"
}

root="$out/des-ui"
install -Dm755 target/release/des-ui "$root/usr/bin/des-ui"
install -Dm755 target/release/des-ui-ctl "$root/usr/bin/des-ui-ctl"
install -Dm755 target/release/des-comp "$root/usr/bin/des-comp"
install -Dm755 target/release/des-portal "$root/usr/bin/des-portal"
install -Dm755 session/des-ui-session "$root/usr/bin/des-ui-session"
install -Dm755 session/des-ui-startup "$root/usr/bin/des-ui-startup"
install -Dm644 session/des-ui.desktop "$root/usr/share/wayland-sessions/des-ui.desktop"
sed -i 's|/usr/local/bin/|/usr/bin/|' "$root/usr/share/wayland-sessions/des-ui.desktop"
install -Dm644 session/des-ui-portals.conf "$root/usr/share/xdg-desktop-portal/des-ui-portals.conf"
install -Dm644 session/des-ui.portal "$root/usr/share/xdg-desktop-portal/portals/des-ui.portal"
install -Dm644 session/org.freedesktop.impl.portal.desktop.des-ui.service "$root/usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.des-ui.service"
sed -i 's|/usr/local/bin/|/usr/bin/|' "$root/usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.des-ui.service"
install -d "$root/usr/share/des-ui/themes"
install -m644 themes/*.json "$root/usr/share/des-ui/themes/"
install -Dm644 README.md "$root/usr/share/doc/des-ui/README.md"
# Pictures for the lock and login screens: each user can add their own, none can
# remove another's.
install -d -m 1777 "$root/var/lib/des-ui/avatars"

# Wayland and the graphics libraries are loaded while running, so nothing finds them
# by looking at the programs: they are named here.
pack des-ui \
    "$(linked target/release/des-ui target/release/des-ui-ctl target/release/des-comp target/release/des-portal), python3, tmux (>= 3.3), libegl1, libwayland-client0, libwayland-cursor0, libwayland-egl1, fontconfig" \
    "libgl1-mesa-dri, xwayland, cage, xdg-desktop-portal-gtk, wireplumber, pipewire-bin, network-manager, bluez, iproute2, sudo, curl, wl-clipboard, sound-theme-freedesktop, des-ui-music" \
    "ffmpeg, des-ui-greeter, des-ui-plymouth" \
    "sci-fi terminal desktop" <<'TEXT'
DEs-UI is a desktop in the style of a film's computer screens: a terminal in
the middle of a frame of live readings, with other applications opening
inside the frame. It is its own Wayland compositor, with settings, a lock
screen, notifications, screenshots and workspaces.

After installing, log out and choose DEs-UI from the session menu of the
login screen.
TEXT

root="$out/des-ui-music"
install -Dm755 target/release/des-music "$root/usr/bin/des-music"
pack des-ui-music \
    "$(linked target/release/des-music), des-ui (= $version), pipewire-bin" \
    "" "" \
    "Spotify player for the DEs-UI desktop" <<'TEXT'
Plays music from Spotify inside DEs-UI, and shows and controls what is
playing on your other devices. It needs a Spotify Premium account.

This is not an official Spotify program: it is built on librespot, an
independent client.
TEXT

root="$out/des-ui-greeter"
install -Dm755 session/des-ui-greeter "$root/usr/bin/des-ui-greeter"
install -Dm755 packaging/des-ui-login "$root/usr/bin/des-ui-login"
pack des-ui-greeter \
    "des-ui (= $version), greetd" \
    "" "" \
    "login screen of the DEs-UI desktop" <<'TEXT'
The DEs-UI login screen, in place of the one the system came with. It is run
by greetd, a small login manager.

Installing this changes nothing by itself. To make it the login screen from
the next boot, run "sudo des-ui-login enable"; "sudo des-ui-login disable"
puts back what was there before, and so does removing the package.
TEXT

root="$out/des-ui-plymouth"
install -d "$root/usr/share/plymouth/themes/des-ui"
# The pictures are drawn by the shell, here in the colours of the default theme:
# an empty configuration folder keeps the theme of whoever builds out of it.
empty=$(mktemp -d)
XDG_CONFIG_HOME="$empty" target/release/des-ui --write-boot-splash "$root/usr/share/plymouth/themes/des-ui" >/dev/null
rmdir "$empty" 2>/dev/null || rm -rf "$empty"
install -m644 session/boot/des-ui.plymouth session/boot/des-ui.script "$root/usr/share/plymouth/themes/des-ui/"
pack des-ui-plymouth \
    "plymouth" \
    "" "des-ui" \
    "boot splash of the DEs-UI desktop" <<'TEXT'
Shows the DEs-UI splash while the machine starts and shuts down: the name,
which glitches now and then, over a bar that fills. It is a theme for
Plymouth, and becomes the one in use unless you have chosen another.

Installing and removing it rebuild the boot image, which takes a minute.
TEXT

ls -1 "$out"/*.deb
