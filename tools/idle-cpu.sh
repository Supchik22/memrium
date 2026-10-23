#!/usr/bin/env bash
cd "$(dirname "$0")/.."
# Idle CPU of the nested compositor: tools/idle-cpu.sh [seconds]  (THEME=windows11 to try another template).
# Uses an empty config in $MEMRIUM_IDLE_CONFIG so your own settings do not change the result.
mkdir -p "${MEMRIUM_IDLE_CONFIG:-/tmp/memrium-idle-config}/memrium-de"
XDG_CONFIG_HOME=${MEMRIUM_IDLE_CONFIG:-/tmp/memrium-idle-config} MEMRIUM_THEME=${THEME:-macos} ./target/release/memrium_compositor > /tmp/memrium-idle.log 2>&1 &
PID=$!
sleep 6
t1=$(awk '{print $14+$15}' /proc/$PID/stat); w1=$(date +%s.%N)
sleep ${1:-8}
t2=$(awk '{print $14+$15}' /proc/$PID/stat); w2=$(date +%s.%N)
awk -v a=$t1 -v b=$t2 -v c=$w1 -v d=$w2 -v tck=$(getconf CLK_TCK) 'BEGIN{printf "CPU: %.1f %% of one core over %.1fs\n", (b-a)/tck/(d-c)*100, d-c}'
kill $PID
