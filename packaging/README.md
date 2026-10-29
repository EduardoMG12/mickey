# Packaging

| Folder | What | Built or sent by |
|---|---|---|
| `linux/` | `install.sh` and the files it installs to `~/.local`, in the release tarball | `release.yml` |
| `windows/` | The NSIS installer, `Mickey-Setup-<version>.exe` | `release.yml` |
| `deb-rpm/` | The `.deb` and `.rpm`, the same files under `/usr` | `release.yml` |
| `aur/` | `mickey-bin` for Arch's AUR | the maintainer, by hand (below) |
| `winget/` | Manifests for `winget install EduardoMG12.Mickey` | the maintainer, by hand (below) |

## Each release, by hand

- **AUR:** in `aur/`, set `pkgver` and the tarball's line of `SHA256SUMS` in
  `sha256sums`, run `makepkg --printsrcinfo > .SRCINFO`, and push
  `PKGBUILD`, `.SRCINFO` and `mickey-bin.install` to
  `ssh://aur@aur.archlinux.org/mickey-bin.git` (an AUR account with your SSH
  key).
- **winget:** in `winget/`, set `PackageVersion`, `InstallerUrl` and
  `InstallerSha256` in the three files, check with
  `winget validate --manifest winget`, and open a pull request to
  microsoft/winget-pkgs with them in `manifests/e/EduardoMG12/Mickey/<version>/`.

## Left out, and why (#176)

- **Scoop** installs portable programs into the user's folder, without
  administrator rights. Mickey on Windows needs them: the logon task that
  runs it as administrator (so it can drive administrator windows) and the
  firewall rule. A Scoop package would either run the installer, which
  Scoop's buckets do not accept, or ship a Mickey that cannot do either.
  winget runs the installer as it is.
- **Flatpak** runs a program in a sandbox that cannot open `/dev/input` or
  `/dev/uinput` and cannot add a udev rule or the user to the `input` group.
  Mickey needs all of them to read the keyboard and make its virtual one.
  Only the portals (InputCapture, RemoteDesktop) would work from inside,
  and only on GNOME and KDE, so a Flatpak would be a smaller Mickey.
