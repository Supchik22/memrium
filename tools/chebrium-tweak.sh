#!/bin/sh
# Sliders for the glass of Qt popups, live: every change is written to ~/.config/memrium/glass.toml and every running Qt
# program that uses the chebrium style repaints at once. Keep a menu open in another program to see it change there.
#   tools/chebrium-tweak.sh
root=$(cd "$(dirname "$0")/.." && pwd)
q="$root/qtstyle"
[ -x "$q/build/chebrium-tweak" ] || nix-shell "$q/shell.nix" --run "cmake -S '$q' -B '$q/build' -G Ninja >/dev/null && cmake --build '$q/build'"
exec nix-shell "$q/shell.nix" --run "QT_PLUGIN_PATH='$q/build':\$QT_PLUGIN_PATH QT_STYLE_OVERRIDE=chebrium QT_QPA_PLATFORM='wayland;xcb' '$q/build/chebrium-tweak'"
