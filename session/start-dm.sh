#!/bin/sh
# The entry of the "Memrium" session in the login screen (SDDM, greetd, ...) on NixOS. The libraries come from
# shell.nix, as in start-tty.sh; the output is kept in ~/memrium-session.log.
here=$(dirname "$(readlink -f "$0")")
root=$(dirname "$here")
export NIX_PATH="${NIX_PATH:-nixpkgs=/nix/var/nix/profiles/per-user/root/channels/nixos}"
export MEMRIUM_STATS=1
export MEMRIUM_WHY=1
export MEMRIUM_DEBUG_POINTER=1
export PATH="/run/current-system/sw/bin:$PATH"
# a look at the NVIDIA card while a game runs (utilization, clock, power; read together with the stats lines)
(nvidia-smi --query-gpu=timestamp,utilization.gpu,clocks.gr,power.draw,pstate --format=csv,noheader -l 5 >"$HOME/memrium-nvidia.log" 2>&1 &)
cd "$root" || exit 1
exec nix-shell "$root/shell.nix" --run "MEMRIUM_COMPOSITOR='$root/target/release/memrium_compositor' '$here/memrium-session'" >"$HOME/memrium-session.log" 2>&1
