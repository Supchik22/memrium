#!/bin/sh
# Put back ~/.config/kdeglobals as it was before a Memrium session wrote its colours into it (the shell does this itself when it
# ends; this is for after a crash). Does nothing when there is no saved copy.
set -e
orig="${XDG_CONFIG_HOME:-$HOME/.config}/memrium/kdeglobals.orig"
if [ -f "$orig" ]; then
    cp "$orig" "${XDG_CONFIG_HOME:-$HOME/.config}/kdeglobals" && rm "$orig" && echo "kdeglobals restored"
else
    echo "no saved copy: nothing to restore"
fi
