#!/bin/sh
# Installs Mickey for the current user. Run it from the unpacked release:
#   ./install.sh
# It asks for your password once, to let Mickey read and create input
# devices (the input group and a udev rule); everything else stays in your
# home directory.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
bin="$HOME/.local/bin"
units="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"

echo "Installing mickey and mickey-ui to $bin"
mkdir -p "$bin"
install -m 755 "$here/mickey" "$here/mickey-ui" "$bin/"

echo "Allowing the input group to read and create input devices (needs sudo)"
echo "  Note: every program you run as a member of the input group can read"
echo "  your keyboard, not only Mickey. Install Mickey only on your own PCs."
sudo install -m 644 "$here/99-mickey-uinput.rules" /etc/udev/rules.d/99-mickey-uinput.rules
sudo udevadm control --reload
# uinput may be built into the kernel, where the rule only applies at boot.
sudo chgrp input /dev/uinput && sudo chmod 660 /dev/uinput
if ! id -nG "$USER" | grep -qw input; then
    sudo usermod -aG input "$USER"
    relogin=1
fi

echo "Adding 'Mickey settings' to your applications menu"
apps="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
mkdir -p "$apps"
sed "s|^Exec=mickey-ui|Exec=$bin/mickey-ui|" "$here/mickey-settings.desktop" > "$apps/mickey-settings.desktop"
# Ratão as the icon. On Wayland a taskbar finds a window's icon through a
# .desktop file named after the window's app id: one per window, hidden
# from the menu, so the switcher, history and pairing windows show it too.
install -D -m 644 "$here/mickey.png" "${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor/256x256/apps/mickey.png"
for window in mickey-ui:switcher mickey-history:history mickey-pair:pairing; do
    printf '[Desktop Entry]\nType=Application\nName=Mickey %s\nExec=%s/mickey-ui\nIcon=mickey\nNoDisplay=true\n' \
        "${window#*:}" "$bin" > "$apps/${window%%:*}.desktop"
done

# Other PCs connect to this one on TCP 4242 and find it on UDP 4243. A
# firewall that is on (firewalld on openSUSE and Fedora, ufw on Ubuntu
# when enabled) would refuse them; Windows' installer opens the same two.
if command -v firewall-cmd >/dev/null && sudo firewall-cmd --state >/dev/null 2>&1; then
    echo "Letting other PCs reach Mickey through firewalld (TCP 4242, UDP 4243)"
    for port in 4242/tcp 4243/udp; do
        sudo firewall-cmd -q --permanent --add-port=$port
        sudo firewall-cmd -q --add-port=$port
    done
elif command -v ufw >/dev/null && sudo ufw status 2>/dev/null | grep -q "Status: active"; then
    echo "Letting other PCs reach Mickey through ufw (TCP 4242, UDP 4243)"
    sudo ufw allow 4242/tcp comment Mickey >/dev/null
    sudo ufw allow 4243/udp comment Mickey >/dev/null
fi

echo "Starting Mickey with your graphical session"
mkdir -p "$units"
install -m 644 "$here/mickey.service" "$units/mickey.service"
systemctl --user daemon-reload
systemctl --user enable mickey.service
# Some desktops (Cinnamon, MATE, XFCE) never start graphical-session.target,
# so the service above would not start: this login entry starts it there.
autostart="${XDG_CONFIG_HOME:-$HOME/.config}/autostart"
mkdir -p "$autostart"
install -m 644 "$here/mickey-autostart.desktop" "$autostart/mickey.desktop"

if [ "${relogin:-0}" = 1 ]; then
    echo
    # Logging out is often not enough: the user's systemd outlives the
    # session and keeps the old groups.
    echo "Done. Restart this PC once (you were added to the input group);"
    echo "Mickey then starts by itself."
else
    systemctl --user restart mickey.service
    echo
    echo "Done. Mickey is running: systemctl --user status mickey"
fi
echo "Then open 'Mickey settings' from your applications to name this PC"
echo "and connect your other PCs (PCs tab)."
