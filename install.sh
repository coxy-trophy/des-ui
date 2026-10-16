#!/bin/sh
# Builds des-ui and registers it as a session on the login screen.
set -eu
cd "$(dirname "$0")"

if ! command -v cage >/dev/null; then
    echo "Note: cage is not installed, so the session has no fallback if des-comp" >&2
    echo "cannot drive the display. To add one: sudo apt install cage" >&2
fi

cargo build --release

# Keep the build being replaced: the session falls back to it if the new one cannot start.
for program in des-ui des-comp; do
    if [ -x "/usr/local/bin/$program" ]; then
        sudo install -Dm755 "/usr/local/bin/$program" "/usr/local/lib/des-ui/previous/$program"
    fi
done
# The first build under this name falls back to the last one under the old name.
if [ ! -x /usr/local/bin/des-ui ] && [ -x /usr/local/bin/edex-rs ] && [ -x /usr/local/bin/edex-comp ]; then
    sudo install -Dm755 /usr/local/bin/edex-rs /usr/local/lib/des-ui/previous/des-ui
    sudo install -Dm755 /usr/local/bin/edex-comp /usr/local/lib/des-ui/previous/des-comp
fi
sudo install -Dm755 target/release/des-ui /usr/local/bin/des-ui
sudo install -Dm755 target/release/des-ui-ctl /usr/local/bin/des-ui-ctl
sudo install -Dm755 target/release/des-comp /usr/local/bin/des-comp
sudo install -Dm755 target/release/des-music /usr/local/bin/des-music
sudo install -Dm755 session/des-ui-session /usr/local/bin/des-ui-session
sudo install -Dm644 session/des-ui.desktop /usr/share/wayland-sessions/des-ui.desktop
sudo install -Dm644 session/des-ui-portals.conf /usr/share/xdg-desktop-portal/des-ui-portals.conf
sudo install -d /usr/local/share/des-ui/themes
sudo install -m644 themes/*.json /usr/local/share/des-ui/themes/

# This desktop was called edex-rs before. Whatever still starts a session under that
# name gets this one; the rest of the old installation goes once nothing of it runs.
if [ -e /usr/local/bin/edex-rs-session ] || [ -L /usr/local/bin/edex-rs-session ]; then
    sudo ln -sfn des-ui-session /usr/local/bin/edex-rs-session
    sudo rm -f /usr/share/wayland-sessions/edex-rs.desktop
fi
if [ -e /usr/local/bin/edex-comp ]; then
    if [ -f /usr/local/bin/edex-rs-greeter ] && [ ! -L /usr/local/bin/edex-rs-greeter ]; then
        echo "The login screen still runs under the old name, edex-rs. Run ./install-login.sh"
        echo "to bring it over; the old programs are left in place until then."
    elif pgrep -x edex-comp >/dev/null; then
        echo "The session running now is the one installed under the old name, edex-rs. It is"
        echo "left in place; run ./install.sh once more after logging in again to clear it away."
    else
        sudo rm -f /usr/local/bin/edex-rs /usr/local/bin/edex-comp /usr/local/bin/edex-music \
            /usr/local/bin/edex-rs-session /usr/share/xdg-desktop-portal/edex-rs-portals.conf
        sudo rm -rf /usr/local/share/edex-rs /usr/local/lib/edex-rs
    fi
fi

if [ "${XDG_CURRENT_DESKTOP:-}" = des-ui ]; then
    echo "Installed. Super+Shift+R restarts the shell on the new build (terminal tabs close);"
    echo "a new compositor takes effect the next time you log in."
else
    echo "Installed. Log out and pick 'DEs-UI' from the session menu on the login screen."
fi
