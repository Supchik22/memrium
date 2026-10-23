#!/bin/sh
# Frame cost while a window is dragged across the desktop (headless, so no display is needed).
#   tools/bench-drag.sh [extra compositor arguments]     e.g.  tools/bench-drag.sh --files ~/Pictures
# Prints the GPU time of the window pass and the shell pass, the CPU time of building a frame, and the frame times.
# MEMRIUM_SKIP=layer turns the desktop-layer cache off (compare with and without), MEMRIUM_SKIP=frame / surf / mip / shell
# leave one kind of work out to see what it costs. Needs a release build: cargo build --release.
here=$(dirname "$(readlink -f "$0")")
bin="${MEMRIUM_COMPOSITOR:-$here/../target/release/memrium_compositor}"
# press on the title bar at 2.55 s, then 80 moves 25 ms apart, 6 px right and 4 down each
script=$(awk 'BEGIN { printf "2500:move 700,120;2550:down"; t = 2600; x = 700; y = 120; for (i = 0; i < 80; i++) { x += 6; y += 4; printf ";%d:move %d,%d", t, x, y; t += 25 } printf ";%d:up", t }')
shot=$(mktemp --suffix=.png)
MEMRIUM_BENCH=1 MEMRIUM_BENCH_FROM=2600 MEMRIUM_TEST_INPUT="$script" "$bin" --headless "$@" --shot "$shot" --after-ms 5200 2>&1 | grep '^bench'
rm -f "$shot"
