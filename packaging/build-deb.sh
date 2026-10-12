#!/bin/sh
# Builds the Debian packages: des-ui (the desktop) and des-ui-music (the Spotify
# player, kept apart so that it can be left out). They land in target/deb/.
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
    (cd "$root" && find usr -type f -exec md5sum {} + >DEBIAN/md5sums)
    dpkg-deb --root-owner-group --build "$root" "$out/${1}_${version}_${arch}.deb" >/dev/null
    rm -rf "$root"
}

root="$out/des-ui"
install -Dm755 target/release/des-ui "$root/usr/bin/des-ui"
install -Dm755 target/release/des-comp "$root/usr/bin/des-comp"
install -Dm755 session/des-ui-session "$root/usr/bin/des-ui-session"
install -Dm644 session/des-ui.desktop "$root/usr/share/wayland-sessions/des-ui.desktop"
sed -i 's|/usr/local/bin/|/usr/bin/|' "$root/usr/share/wayland-sessions/des-ui.desktop"
install -Dm644 session/des-ui-portals.conf "$root/usr/share/xdg-desktop-portal/des-ui-portals.conf"
install -d "$root/usr/share/des-ui/themes"
install -m644 themes/*.json "$root/usr/share/des-ui/themes/"
install -Dm644 README.md "$root/usr/share/doc/des-ui/README.md"

# Wayland and the graphics libraries are loaded while running, so nothing finds them
# by looking at the programs: they are named here.
pack des-ui \
    "$(linked target/release/des-ui target/release/des-comp), libegl1, libwayland-client0, libwayland-cursor0, libwayland-egl1, fontconfig" \
    "libgl1-mesa-dri, xwayland, cage, xdg-desktop-portal-gtk, wireplumber, pipewire-bin, network-manager, bluez, iproute2, sudo, curl, wl-clipboard, sound-theme-freedesktop, des-ui-music" \
    "ffmpeg, greetd, plymouth-themes" \
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

ls -1 "$out"/*.deb
