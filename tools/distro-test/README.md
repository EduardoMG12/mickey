# Distro test VMs

Repeatable VMs for the distro matrix (#171): Mickey installed by following
only the README, on the desktops people use. QEMU/KVM, no window anywhere:
the screen is read through QMP screenshots, and typing and clicks go in
through QMP too.

| Distro | Desktop | How | SSH |
|---|---|---|---|
| Ubuntu 24.04 | GNOME, Wayland | installed unattended to a disk | `ssh -p 2221 tester@127.0.0.1` |
| Linux Mint 22.3 | Cinnamon, X11 | live session | `ssh -p 2222 mint@127.0.0.1` |
| Fedora 44 | KDE Plasma, Wayland | live session | `ssh -p 2223 liveuser@127.0.0.1` |

Needs `qemu-system-x86_64` with KVM, `bsdtar`, `socat`, `python3` and
`openssl`. Put the ISOs in `~/.cache/mickey-vms` (or `$MICKEY_VMS`) and
check them against the distro's SHA256 list. The VMs use 3 GB of RAM each;
two at once is enough for pairing.

## Ubuntu

```sh
tools/distro-test/vm.sh ubuntu install   # about 15 min; powers off when done
tools/distro-test/vm.sh ubuntu start
```

The seed gives user `tester` your `~/.ssh/id_ed25519.pub`, passwordless
sudo and automatic login.

## Mint and Fedora (live sessions)

```sh
tools/distro-test/vm.sh mint start
Q=~/.cache/mickey-vms/mint/qmp.sock
tools/distro-test/qtype.py $Q keys ctrl-alt-t        # a terminal (Konsole on Fedora)
tools/distro-test/qtype.py $Q text "mkdir -p ~/.ssh && wget -qO ~/.ssh/authorized_keys http://10.0.2.2:8001/key.pub; sudo sed -i '/cdrom:/d' /etc/apt/sources.list; sudo apt-get -qq update; sudo apt-get -qq install -y openssh-server"
```

Serve your public key first: `cp ~/.ssh/id_ed25519.pub
~/.cache/mickey-vms/ubuntu/seed/key.pub` with the seed's web server
running. On Fedora, use `curl -s ... > ~/.ssh/authorized_keys` and
`sudo systemctl start sshd` instead of apt.

A live session forgets everything when it stops. To get a fresh login as
after a reboot (the user's systemd restarted with the new `input` group),
end the user's session from SSH: `sudo loginctl terminate-user $USER; sudo
systemctl stop user@1000.service`. Then press Enter at Mint's login screen
(`qtype.py $Q keys ret`), or on Fedora run `sudo systemctl restart
display-manager`.

## Mickey in the VMs

Build the release inside the Ubuntu VM: a binary built on a newer distro
may need a newer glibc than Ubuntu 24.04 has (#220). Copy it to the other
VMs, then follow the README's install section.

**The shared LAN:** give each VM an address with `nmcli con add type
ethernet ifname <the 52:54:00:12:34:1x card> con-name lan ipv4.method
manual ipv4.addresses 192.0.2.1x/24`. Until #217 is fixed, discovery only
works on the default route's network, so also run `sudo ip route del
default via 10.0.2.2`.

**Input without logging it:** count the events that arrive on the
receiving PC's virtual device, never their codes. Find the device (the
`event*` whose name is *Mickey virtual input*), run `sudo timeout 10 cat
/dev/input/eventN | wc -c` there, send the focus with `mickey go <name>`,
and type with `qtype.py`. Every input event is 24 bytes; one key is a
press, a release and two syncs, so 4 events. QEMU's mouse does not send
relative motion while the USB tablet is attached, so test the mouse with a
uinput device made inside the source VM.

**Clicks:** `qclick.py SOCKET X Y 1280 800` clicks at screen pixels, for
example *Pair* in both pair windows.
