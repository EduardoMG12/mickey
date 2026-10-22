#!/bin/sh
# Removes what install.sh added. Your keys, pairings and config.toml stay in
# ~/.local/share/mickey and ~/.config/mickey, like SSH keys.
set -eu
units="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
systemctl --user disable --now mickey.service 2>/dev/null || true
rm -f "$units/mickey.service" "$HOME/.local/bin/mickey" "$HOME/.local/bin/mickey-ui"
systemctl --user daemon-reload
echo "Removing the udev rule (needs sudo); you stay in the input group."
sudo rm -f /etc/udev/rules.d/99-mickey-uinput.rules
echo "Mickey is uninstalled."
