#!/bin/sh
# The entry of the "Memrium" session in the login screen (SDDM, greetd, ...) on NixOS. The libraries come from
# shell.nix, as in start-tty.sh; the output is kept in ~/memrium-session.log.
here=$(dirname "$(readlink -f "$0")")
root=$(dirname "$here")
export NIX_PATH="${NIX_PATH:-nixpkgs=/nix/var/nix/profiles/per-user/root/channels/nixos}"
export MEMRIUM_STATS=1
export PATH="/run/current-system/sw/bin:$PATH"
cd "$root" || exit 1
exec nix-shell "$root/shell.nix" --run "MEMRIUM_COMPOSITOR='$root/target/release/memrium_compositor' '$here/memrium-session'" >"$HOME/memrium-session.log" 2>&1
