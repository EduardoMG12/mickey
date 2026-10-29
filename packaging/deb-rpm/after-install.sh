#!/bin/sh
# After the .deb or .rpm installs (package.sh): what install.sh does with
# sudo, for every user. Never fails the install.
udevadm control --reload 2>/dev/null || true
# uinput may be built into the kernel, where the rule only applies at boot.
if [ -e /dev/uinput ]; then
    chgrp input /dev/uinput 2>/dev/null && chmod 660 /dev/uinput 2>/dev/null || true
fi
# Starts with every user's graphical session.
systemctl --global enable mickey.service >/dev/null 2>&1 || true
# The person who ran `sudo apt install` or `sudo dnf install`: Mickey reads
# their keyboard through the input group.
person="${SUDO_USER:-}"
if [ -n "$person" ] && [ "$person" != root ] && ! id -nG "$person" | grep -qw input; then
    usermod -aG input "$person" 2>/dev/null || true
    echo "Mickey: $person was added to the input group: restart this PC once."
fi
echo "Mickey: every program run by a member of the input group can read the"
echo "keyboard, not only Mickey. Other users: sudo usermod -aG input <user>."
echo "With a firewall on, let in TCP 4242-4243 and UDP 4243-4244."
exit 0
