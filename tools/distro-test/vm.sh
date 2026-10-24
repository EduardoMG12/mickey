#!/bin/sh
# Test VMs for the distro matrix (#171): QEMU/KVM with no window.
#
#   tools/distro-test/vm.sh <distro> install   Ubuntu only: unattended install to a disk
#   tools/distro-test/vm.sh <distro> start     boot (Ubuntu: the installed disk; Mint and
#                                              Fedora: their live session from the ISO)
#   tools/distro-test/vm.sh <distro> shot F    save the screen as a PNG
#   tools/distro-test/vm.sh <distro> stop      power off at once
#
# distros: ubuntu (24.04, GNOME, Wayland), mint (22.3, Cinnamon, X11),
# fedora (44, KDE Plasma, Wayland). ISOs, disks and sockets live in
# $MICKEY_VMS (default ~/.cache/mickey-vms); download the ISOs named
# below there and check their SHA256 first.
#
# Each VM has two network cards: NAT to the internet with SSH forwarded to
# 127.0.0.1:<port>, and a LAN shared by all running test VMs (QEMU socket
# multicast, no root) where they find and pair with each other; give it
# 192.0.2.x/24 inside the VM.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=${MICKEY_VMS:-$HOME/.cache/mickey-vms}
distro=${1:?distro}
action=${2:?action}
dir="$root/$distro"
mkdir -p "$dir"
cd "$dir"

case $distro in
ubuntu) iso=ubuntu-24.04.5.1-desktop-amd64.iso; n=1 ;;
mint) iso=linuxmint-22.3-cinnamon-64bit.iso; n=2 ;;
fedora) iso=Fedora-KDE-Desktop-Live-44-1.7.x86_64.iso; n=3 ;;
*) echo "unknown distro $distro" >&2; exit 1 ;;
esac

qmp() { printf '{"execute":"qmp_capabilities"}\n%s\n' "$1" | socat - UNIX-CONNECT:qmp.sock >/dev/null; }

case $action in
shot) qmp "{\"execute\":\"screendump\",\"arguments\":{\"filename\":\"$(realpath -m "$3")\",\"format\":\"png\"}}"; exit ;;
stop) qmp '{"execute":"quit"}' || true; exit ;;
install | start) ;;
*) echo "unknown action $action" >&2; exit 1 ;;
esac

boot=""
case $distro-$action in
ubuntu-install)
    # The installer's kernel, an empty 32 GB disk (sparse), and the
    # autoinstall seed served to the VM at 10.0.2.2:8001.
    [ -f vmlinuz ] || bsdtar -xf "../$iso" -s ',casper/,,' casper/vmlinuz casper/initrd
    [ -f disk.raw ] || truncate -s 32G disk.raw
    mkdir -p seed
    key=$(cat "$HOME/.ssh/id_ed25519.pub")
    password=$(openssl passwd -6 tester)
    sed -e "s|@KEY@|$key|" -e "s|@PASSWORD@|$password|" "$here/ubuntu-user-data" > seed/user-data
    printf 'instance-id: mickey-ubuntu\n' > seed/meta-data
    : > seed/vendor-data
    (cd seed && setsid python3 -m http.server 8001 --bind 127.0.0.1 >/dev/null 2>&1 &)
    boot="-drive file=disk.raw,format=raw,if=virtio -cdrom ../$iso -kernel vmlinuz -initrd initrd"
    set -- -append "boot=casper autoinstall ds=nocloud-net;s=http://10.0.2.2:8001/ ---"
    ;;
ubuntu-start) boot="-drive file=disk.raw,format=raw,if=virtio"; set -- ;;
mint-start) boot="-cdrom ../$iso -boot d"; set -- ;;
fedora-start)
    # Straight into the live session: the ISO's menu would check the media first.
    [ -f linux ] || bsdtar -xf "../$iso" -s ',boot/x86_64/loader/,,' boot/x86_64/loader/linux boot/x86_64/loader/initrd
    boot="-cdrom ../$iso -kernel linux -initrd initrd"
    set -- -append "root=live:CDLABEL=Fedora-KDE-Live-44 rd.live.image quiet"
    ;;
*) echo "$distro has no $action" >&2; exit 1 ;;
esac

# shellcheck disable=SC2086 # $boot is a list of arguments
exec qemu-system-x86_64 -enable-kvm -machine q35 -cpu host -m 3072 -smp 4 $boot "$@" \
    -netdev "user,id=n,hostfwd=tcp:127.0.0.1:222$n-:22" -device virtio-net-pci,netdev=n \
    -netdev socket,id=lan,mcast=230.0.0.1:12345 -device "virtio-net-pci,netdev=lan,mac=52:54:00:12:34:1$n" \
    -device VGA,vgamem_mb=64 -device qemu-xhci -device usb-tablet \
    -display none -vnc "127.0.0.1:$n" -qmp unix:qmp.sock,server,nowait -daemonize -pidfile vm.pid
