#!/bin/sh
# Build the `chebrium` Qt style and put it where the session and Flatpak programs can find it:
#   ~/.local/share/memrium/qt-plugins/styles/libchebrium.so      (no RPATH: it uses the Qt of the program that loads it)
# Flatpak programs named on the command line get an override: the plugin directory and the shell's look (appearance.toml)
# are visible in the sandbox, and the style is switched on. `--undo APP...` takes that away again.
#
#   tools/install-qtstyle.sh                                  build and install
#   tools/install-qtstyle.sh org.prismlauncher.PrismLauncher  ... and set the override for that program
#   tools/install-qtstyle.sh --undo org.prismlauncher.PrismLauncher
set -e
root=$(cd "$(dirname "$0")/.." && pwd)
dest="${XDG_DATA_HOME:-$HOME/.local/share}/memrium-de/qt-plugins"
# what a KDE runtime sets itself: keep it, ours goes first
runtime_path=/app/lib/plugins:/usr/share/runtime/lib/plugins

if [ "$1" = "--undo" ]; then
    shift
    for app in "$@"; do
        flatpak override --user --reset "$app" && echo "override of $app removed"
    done
    exit 0
fi

nix-shell "$root/qtstyle/shell.nix" --run "cmake -S '$root/qtstyle' -B '$root/qtstyle/build' -G Ninja >/dev/null && cmake --build '$root/qtstyle/build' --target chebrium"
mkdir -p "$dest/styles"
rm -f "$dest/styles/libchebrium.so"
cp "$root/qtstyle/build/styles/libchebrium.so" "$dest/styles/libchebrium.so"
chmod u+w "$dest/styles/libchebrium.so"
nix-shell -p patchelf --run "patchelf --remove-rpath '$dest/styles/libchebrium.so'"
echo "installed $dest/styles/libchebrium.so"

for app in "$@"; do
    flatpak override --user "$app" \
        --filesystem="$dest:ro" --filesystem=xdg-config/memrium-de:ro \
        --env=QT_PLUGIN_PATH="$dest:$runtime_path" --env=QT_STYLE_OVERRIDE=chebrium
    echo "override set for $app"
done
