#!/bin/sh
# Registers the Memrium portal backend (file dialogs, screen sharing) for this user, pointing at a cargo build of this checkout:
#   tools/install-portal.sh [release|debug]
# The bus then starts memrium_portal the first time a program opens a file dialog. Log out and in (or restart
# xdg-desktop-portal) afterwards. On NixOS the package and the module do this system-wide instead.
set -e
root=$(cd "$(dirname "$0")/.." && pwd)
profile=${1:-release}
bin=$root/target/$profile
[ -x "$bin/memrium_portal" ] || { echo "build first: cargo build --profile $profile --bin memrium_portal --bin memrium_files --bin memrium_share" >&2; exit 1; }
data=${XDG_DATA_HOME:-$HOME/.local/share}
mkdir -p "$data/xdg-desktop-portal/portals" "$data/dbus-1/services"
cp "$root/session/portal/memrium.portal" "$data/xdg-desktop-portal/portals/memrium.portal"
sed "s|@BIN@|$bin|" "$root/session/portal/org.freedesktop.impl.portal.desktop.memrium.service.in" > "$data/dbus-1/services/org.freedesktop.impl.portal.desktop.memrium.service"
echo "installed: $data/xdg-desktop-portal/portals/memrium.portal and the D-Bus service for $bin"
