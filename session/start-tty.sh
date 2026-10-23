#!/bin/sh
# Start Memrium from a text console (VT) on NixOS: log in on a free VT (Ctrl+Alt+F3) and run
#   /path/to/memrium-de/session/start-tty.sh
# The libraries the compositor needs come from shell.nix; output is also saved to ~/memrium-session.log.
here=$(dirname "$(readlink -f "$0")")
root=$(dirname "$here")
cd "$root" || exit 1
[ -x "$root/target/release/memrium_compositor" ] || { echo "build first: nix-shell --run 'cargo build --release'"; exit 1; }
exec nix-shell "$root/shell.nix" --run "MEMRIUM_COMPOSITOR='$root/target/release/memrium_compositor' '$here/memrium-session' 2>&1 | tee \"\$HOME/memrium-session.log\""
