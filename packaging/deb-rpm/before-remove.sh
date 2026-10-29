#!/bin/sh
# Before the .deb or .rpm is removed (package.sh): stop starting Mickey
# with new sessions. Never fails the removal.
systemctl --global disable mickey.service >/dev/null 2>&1 || true
exit 0
