#!/bin/sh
# Registers the Memrium portal backend (file dialogs) for this user, pointing at a cargo build of memrium-files:
#   tools/install-portal.sh [BIN_DIR]        (default: ../memrium-files/target/release)
# The bus then starts memrium_portal the first time a program opens a file dialog. Log out and in (or restart
# xdg-desktop-portal) afterwards. On NixOS the package and the module do this system-wide instead.
set -e
root=$(cd "$(dirname "$0")/.." && pwd)
bin=$(cd "${1:-$root/../memrium-files/target/release}" && pwd)
[ -x "$bin/memrium_portal" ] || { echo "build first: cargo build --release (in memrium-files) -> $bin/memrium_portal" >&2; exit 1; }
data=${XDG_DATA_HOME:-$HOME/.local/share}
mkdir -p "$data/xdg-desktop-portal/portals" "$data/dbus-1/services"
cp "$root/session/portal/memrium.portal" "$data/xdg-desktop-portal/portals/memrium.portal"
sed "s|@BIN@|$bin|" "$root/session/portal/org.freedesktop.impl.portal.desktop.memrium.service.in" > "$data/dbus-1/services/org.freedesktop.impl.portal.desktop.memrium.service"
echo "installed: $data/xdg-desktop-portal/portals/memrium.portal and the D-Bus service for $bin"
