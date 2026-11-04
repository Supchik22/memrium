#!/bin/sh
# Try a change in the real session without a nixos-rebuild (which builds everything from nothing, ~12 minutes).
#
#   tools/dev-install.sh        build here (incremental) and put the programs in ~/.local/share/memrium-dev
#   tools/dev-install.sh --off  go back to the installed system version
#
# After it, log out and in again for a new compositor; programs started from the shell (Terminal, Files, ...) use the new
# build at once. The session picks the dev build when ~/.local/share/memrium-dev/active exists (see session/memrium-session,
# which needs one `sudo ./install.sh` to learn that). Each program gets the wrapper of the installed system version, so its
# libraries and tools are found exactly as in a real install.
set -eu

here=$(cd "$(dirname "$0")/.." && pwd)
dev=${MEMRIUM_DEV:-$HOME/.local/share/memrium-dev}
shell_dir=${MEMRIUM_DEVSHELL:-$HOME/.local/share/memrium-devshell}

if [ "${1:-}" = "--off" ]; then
    rm -f "$dev/active"
    echo "dev build off: the next login starts the installed version"
    exit 0
fi

# never fill the disk again: a release build with its dependencies needs room
free_gb=$(df -BG --output=avail "$here" | tail -1 | tr -dc 0-9)
if [ "$free_gb" -lt "${MEMRIUM_MIN_FREE_GB:-25}" ]; then
    echo "only ${free_gb} GB free (need ${MEMRIUM_MIN_FREE_GB:-25}): clean up first" >&2
    exit 1
fi

# the wrapper the installed version has: it sets LD_LIBRARY_PATH, PATH and so on, then runs the real program
template=$(command -v memrium_compositor || true)
if [ -z "$template" ] || ! head -c 2 "$template" | grep -q '#!'; then
    echo "memrium_compositor is not installed on this system: run 'sudo ./install.sh' once first" >&2
    exit 1
fi

# the dev shell lives outside the project: inside it, `nix develop` copies the whole folder (target/ too) to the store every time
if [ ! -f "$shell_dir/flake.nix" ]; then
    echo "missing $shell_dir/flake.nix (the dev shell flake)" >&2
    exit 1
fi

cd "$here"
before=$free_gb
CARGO_INCREMENTAL=1 nix --extra-experimental-features "nix-command flakes" develop --impure "$shell_dir" --command cargo build --release --bins

mkdir -p "$dev/bin"
n=0
for f in target/release/memrium_*; do
    [ -f "$f" ] && [ -x "$f" ] || continue
    case "$f" in *.d) continue ;; esac
    name=$(basename "$f")
    # the real program next to its wrapper, as in the Nix package (the shell finds its programs beside itself)
    ln -f "$f" "$dev/bin/.$name-wrapped" 2>/dev/null || cp -f "$f" "$dev/bin/.$name-wrapped"
    sed "s|/nix/store/[^\"' ]*/\.memrium_compositor-wrapped|$dev/bin/.$name-wrapped|" "$template" > "$dev/bin/$name.new"
    chmod +x "$dev/bin/$name.new"
    mv -f "$dev/bin/$name.new" "$dev/bin/$name"
    n=$((n + 1))
done
touch "$dev/active"

after=$(df -BG --output=avail "$here" | tail -1 | tr -dc 0-9)
echo "installed $n programs in $dev/bin ($((before - after)) GB of disk used by this run)"
echo "log out and in to start the new compositor; programs started from the shell use it already"
