#!/bin/sh
# Removes ~/Applications/Mickey.app, CLI symlinks, and the LaunchAgent.
# Your keys, pairings and config.toml stay in ~/.local/share/mickey and
# ~/.config/mickey, like SSH keys.
set -eu

app="$HOME/Applications/Mickey.app"
bin="$HOME/.local/bin"
label="io.github.eduardomg12.mickey"
plist="$HOME/Library/LaunchAgents/$label.plist"
uid=$(/usr/bin/id -u)

if [ -x "$app/Contents/MacOS/mickey" ]; then
    "$app/Contents/MacOS/mickey" autostart off 2>/dev/null || true
elif [ -x "$bin/mickey" ]; then
    "$bin/mickey" autostart off 2>/dev/null || true
fi

/bin/launchctl bootout "gui/$uid/$label" 2>/dev/null || true
rm -f "$plist"

rm -rf "$app"
rm -f "$bin/mickey" "$bin/mickey-ui"
echo "Mickey is uninstalled."
