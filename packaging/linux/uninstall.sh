#!/bin/sh
# Removes what install.sh added. Your keys, pairings and config.toml stay in
# ~/.local/share/mickey and ~/.config/mickey, like SSH keys.
set -eu
units="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
systemctl --user disable --now mickey.service 2>/dev/null || true
rm -f "$units/mickey.service" "$HOME/.local/bin/mickey" "$HOME/.local/bin/mickey-ui" \
    "${XDG_DATA_HOME:-$HOME/.local/share}/applications/mickey-settings.desktop" \
    "${XDG_DATA_HOME:-$HOME/.local/share}/applications/mickey-ui.desktop" \
    "${XDG_DATA_HOME:-$HOME/.local/share}/applications/mickey-history.desktop" \
    "${XDG_DATA_HOME:-$HOME/.local/share}/applications/mickey-pair.desktop" \
    "${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/256x256/apps/mickey.png" \
    "${XDG_CONFIG_HOME:-$HOME/.config}/autostart/mickey.desktop"
systemctl --user daemon-reload
echo "Removing the udev rule (needs sudo); you stay in the input group."
sudo rm -f /etc/udev/rules.d/99-mickey-uinput.rules
# Closes what install.sh opened in the firewall, if it is on.
if command -v firewall-cmd >/dev/null && sudo firewall-cmd --state >/dev/null 2>&1; then
    for port in 4242/tcp 4243/tcp 4243/udp 4244/udp; do
        sudo firewall-cmd -q --permanent --remove-port=$port
        sudo firewall-cmd -q --remove-port=$port
    done
elif command -v ufw >/dev/null && sudo ufw status 2>/dev/null | grep -q "Status: active"; then
    for port in 4242/tcp 4243/tcp 4243/udp 4244/udp; do
        sudo ufw delete allow $port >/dev/null
    done
fi
echo "Mickey is uninstalled."
