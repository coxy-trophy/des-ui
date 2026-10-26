#!/bin/sh
# Builds des-ui and registers it as a session on the login screen.
set -eu
cd "$(dirname "$0")"

if ! command -v cage >/dev/null; then
    echo "Note: cage is not installed, so the session has no fallback if des-comp" >&2
    echo "cannot drive the display. To add one: sudo apt install cage" >&2
fi

command -v python3 >/dev/null || { echo "python3 is required by the session startup watchdog" >&2; exit 1; }

# Direct terminals remain available without tmux; live reconnection needs 3.3+.
if command -v tmux >/dev/null && tmux -V | awk '{ split($2, v, "."); exit !(v[1] > 3 || (v[1] == 3 && v[2] + 0 >= 3)) }'; then
    echo "tmux 3.3+ detected: terminal preservation across shell restarts is available."
else
    echo "Note: tmux 3.3+ is required to keep terminals across shell restarts." >&2
    echo "Without it, turn off Keep terminals across shell restarts in Settings for direct terminals." >&2
fi

if ! python3 -c 'import websocket' >/dev/null 2>&1; then
    echo "Note: companion voice requires python3-websocket (websocket-client). Text remains available." >&2
fi

if ! /usr/bin/python3 -c 'import gi; gi.require_version("Secret", "1"); from gi.repository import Secret' >/dev/null 2>&1 || ! command -v gnome-keyring-daemon >/dev/null; then
    echo "Note: remembering companion keys requires: sudo apt install python3-gi gir1.2-secret-1 gnome-keyring dbus-user-session" >&2
    echo "Secret Service starts through D-Bus on demand. Unlock the login keyring when saving a key." >&2
fi

cargo build --release

# Legacy clients load these images instead of the cursor-shape protocol. Export
# with the same rasterizer as des-comp, before changing the installed programs.
rm -rf target/cursor-theme
target/release/des-comp --write-cursor-theme target/cursor-theme

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
sudo install -Dm755 target/release/des-portal /usr/local/bin/des-portal
sudo install -Dm755 target/release/des-hands /usr/local/bin/des-hands
sudo install -Dm755 session/des-ui-session /usr/local/bin/des-ui-session
sudo install -Dm755 session/des-ui-startup /usr/local/bin/des-ui-startup
sudo install -Dm644 session/des-ui.desktop /usr/share/wayland-sessions/des-ui.desktop
sudo install -Dm644 session/des-ui-portals.conf /usr/share/xdg-desktop-portal/des-ui-portals.conf
sudo install -Dm644 session/des-ui.portal /usr/share/xdg-desktop-portal/portals/des-ui.portal
sudo install -Dm644 session/org.freedesktop.impl.portal.desktop.des-ui.service /usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.des-ui.service
sudo install -d /usr/local/share/des-ui/companion
sudo install -m644 companion/service.py companion/voice.py companion/credentials.py /usr/local/share/des-ui/companion/
sudo install -d /usr/local/share/des-ui/themes
sudo install -m644 themes/*.json /usr/local/share/des-ui/themes/
sudo install -d /usr/local/share/icons/DEs-UI/cursors
sudo install -m644 target/cursor-theme/index.theme /usr/local/share/icons/DEs-UI/index.theme
sudo install -m644 target/cursor-theme/cursors/* /usr/local/share/icons/DEs-UI/cursors/

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
    echo "Installed. Super+Shift+R restarts the shell on the new build; managed terminals reconnect;"
    echo "a new compositor takes effect the next time you log in."
    echo "For frame geometry updates, log out and in so shell and compositor switch together."
else
    echo "Installed. Log out and pick 'DEs-UI' from the session menu on the login screen."
fi
