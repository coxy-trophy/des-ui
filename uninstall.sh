#!/bin/sh
# Removes everything install.sh put on the system.
set -eu
sudo rm -f /usr/local/bin/des-ui /usr/local/bin/des-ui-ctl /usr/local/bin/des-comp /usr/local/bin/des-music /usr/local/bin/des-portal /usr/local/bin/des-hands \
    /usr/local/bin/des-connectd /usr/local/bin/des-connect-ctl \
    /usr/local/bin/des-ui-session /usr/local/bin/des-ui-startup /usr/share/wayland-sessions/des-ui.desktop /usr/share/xdg-desktop-portal/des-ui-portals.conf \
    /usr/share/xdg-desktop-portal/portals/des-ui.portal /usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.des-ui.service
sudo rm -rf /usr/local/share/des-ui /usr/local/lib/des-ui
sudo rm -rf /usr/local/share/icons/DEs-UI
sudo rm -f /usr/local/share/icons/hicolor/scalable/apps/des-ui.svg \
    /usr/local/share/icons/hicolor/16x16/apps/des-ui.png /usr/local/share/icons/hicolor/24x24/apps/des-ui.png \
    /usr/local/share/icons/hicolor/32x32/apps/des-ui.png /usr/local/share/icons/hicolor/48x48/apps/des-ui.png \
    /usr/local/share/icons/hicolor/64x64/apps/des-ui.png /usr/local/share/icons/hicolor/128x128/apps/des-ui.png \
    /usr/local/share/icons/hicolor/256x256/apps/des-ui.png /usr/local/share/icons/hicolor/512x512/apps/des-ui.png
# What an installation under the old name, edex-rs, left behind.
sudo rm -f /usr/local/bin/edex-rs /usr/local/bin/edex-comp /usr/local/bin/edex-music /usr/local/bin/edex-rs-session \
    /usr/share/wayland-sessions/edex-rs.desktop /usr/share/xdg-desktop-portal/edex-rs-portals.conf
sudo rm -rf /usr/local/share/edex-rs /usr/local/lib/edex-rs
echo "Removed."
