#!/bin/sh
# Builds mickey_<version>_amd64.deb and mickey-<version>-1.x86_64.rpm from
# release binaries (#176), with the distros' own tools: dpkg-deb, and
# rpmbuild (the `rpm` package on Ubuntu). Run from the repository:
#   packaging/deb-rpm/package.sh <version> <folder with mickey, mickey-ui>
# A package installs for every user of the PC, under /usr; ./install.sh
# from the tarball installs for one user, under ~/.local. Use one or the
# other, not both.
set -eu
version="$1"
bins="$2"
here=$(cd "$(dirname "$0")" && pwd)
linux="$here/../linux"
out=$(pwd)
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
root="$stage/root"

# The same files install.sh puts in ~/.local, at their system-wide places.
install -D -m 755 "$bins/mickey" "$root/usr/bin/mickey"
install -D -m 755 "$bins/mickey-ui" "$root/usr/bin/mickey-ui"
install -D -m 644 "$linux/99-mickey-uinput.rules" "$root/usr/lib/udev/rules.d/99-mickey-uinput.rules"
sed 's|^ExecStart=%h/.local/bin/mickey|ExecStart=/usr/bin/mickey|' "$linux/mickey.service" |
    install -D -m 644 /dev/stdin "$root/usr/lib/systemd/user/mickey.service"
install -D -m 644 "$linux/mickey-autostart.desktop" "$root/etc/xdg/autostart/mickey.desktop"
install -D -m 644 "$linux/mickey-settings.desktop" "$root/usr/share/applications/mickey-settings.desktop"
for window in mickey-ui:switcher mickey-history:history mickey-pair:pairing; do
    printf '[Desktop Entry]\nType=Application\nName=Mickey %s\nExec=mickey-ui\nIcon=mickey\nNoDisplay=true\n' \
        "${window#*:}" > "$root/usr/share/applications/${window%%:*}.desktop"
done
install -D -m 644 "$linux/mickey.png" "$root/usr/share/icons/hicolor/256x256/apps/mickey.png"
install -D -m 644 "$here/../../README.md" "$root/usr/share/doc/mickey/README.md"
install -D -m 644 "$here/../../LICENSE" "$root/usr/share/licenses/mickey/LICENSE"
if [ -f "$bins/THIRD-PARTY-LICENSES.txt" ]; then
    install -D -m 644 "$bins/THIRD-PARTY-LICENSES.txt" "$root/usr/share/licenses/mickey/THIRD-PARTY-LICENSES.txt"
fi

summary="One keyboard and mouse for all your computers"

# .deb
mkdir -p "$stage/deb"
cp -a "$root/." "$stage/deb/"
mkdir -p "$stage/deb/DEBIAN"
cat > "$stage/deb/DEBIAN/control" <<EOF
Package: mickey
Version: $version
Architecture: amd64
Maintainer: Eduardo Mello <102185386+EduardoMG12@users.noreply.github.com>
Depends: libc6 (>= 2.35), libpulse0
Section: utils
Priority: optional
Homepage: https://github.com/EduardoMG12/mickey
Description: $summary
 Push the cursor off the edge of one screen and it appears on the next PC;
 copy on one and paste on another. Linux and Windows.
EOF
echo "/etc/xdg/autostart/mickey.desktop" > "$stage/deb/DEBIAN/conffiles"
install -m 755 "$here/after-install.sh" "$stage/deb/DEBIAN/postinst"
install -m 755 "$here/before-remove.sh" "$stage/deb/DEBIAN/prerm"
dpkg-deb --root-owner-group --build "$stage/deb" "$out/mickey_${version}_amd64.deb"

# .rpm: rpm versions cannot hold a "-" (a test version like 0.0.0-test).
rpm_version=$(echo "$version" | tr - '~')
mkdir -p "$stage/rpm/SPECS"
{
    cat <<EOF
Name: mickey
Version: $rpm_version
Release: 1
Summary: $summary
License: GPL-3.0-or-later
URL: https://github.com/EduardoMG12/mickey
Requires: glibc >= 2.35
Requires: pulseaudio-libs
AutoReqProv: no

%description
Push the cursor off the edge of one screen and it appears on the next PC;
copy on one and paste on another. Linux and Windows.

%install
cp -a "$root/." %{buildroot}/

%post
EOF
    cat "$here/after-install.sh"
    echo
    echo "%preun"
    # rpm runs %preun with 0 on removal, 1 on upgrade.
    echo 'if [ "$1" = 0 ]; then'
    cat "$here/before-remove.sh"
    echo 'fi'
    echo
    echo "%files"
    (cd "$root" && find . -type f | sed 's|^\.||' | sort) |
        sed 's|^/etc/xdg/autostart/mickey.desktop$|%config(noreplace) &|'
} > "$stage/rpm/SPECS/mickey.spec"
rpmbuild --quiet -bb --define "_topdir $stage/rpm" "$stage/rpm/SPECS/mickey.spec"
cp "$stage"/rpm/RPMS/x86_64/*.rpm "$out/"
ls "$out"/mickey_*.deb "$out"/mickey-*.rpm
