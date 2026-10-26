# 0013 — Updates

> Status: **draft** · Depends on: [0011](0011-shared-clipboard.md) (the data link) · Vision: [0000](0000-vision.md) (changes *no update checks*)

## Goals

Most releases fix security or speed, so a PC left on an old Mickey is a
real risk. Decided with the owner on 2026-10-26: updating must be
**safe**, must **work**, must be **easy**, and it must be **obvious** that
an update is waiting.

- **Every PC learns of a new release within a day**, from GitHub, and
  from any paired PC that already runs it.
- **It is hard to miss**: the tray's menu starts with *Update to 0.2.2*
  and the tray tooltip says so; the settings window shows a banner; the
  PCs tab shows each PC's version, so a PC left behind stands out.
- **One click updates**: download, check the signature, install,
  restart. On Windows the system asks for an administrator once (the
  installer needs it); on Linux nothing is asked.
- **Nothing unverified ever runs.** A release is installed only when its
  checksums carry a valid signature from the release key built into
  Mickey, and the downloaded file matches them.

## Approach

- **Releases are signed.** `release.yml` signs `SHA256SUMS` with an
  Ed25519 key held as the repository secret `RELEASE_SIGNING_KEY`
  (`openssl pkeyutl -sign -rawin`) and publishes `SHA256SUMS.sig`. The
  public half is `packaging/release-key.pub` (32 bytes, hex), built into
  Mickey with `include_str!`. HTTPS proves the file came from GitHub; the
  signature proves it came from the owner, even if the GitHub account or
  a release were tampered with.
- **The check is one HTTPS request**, at start (after a minute) and
  every 24 h: `GET https://api.github.com/repos/EduardoMG12/mickey/releases/latest`,
  reading only `tag_name`. Newer means a higher semantic version than
  this build's `CARGO_PKG_VERSION`; never a downgrade.
- **Downloads go through the system's `curl`**, which every Linux
  desktop and Windows 10 (1803) or newer ships: no HTTP or TLS crate in
  Mickey. Files land in Mickey's data folder, `updates/<version>/`.
- **Verification in Rust**: `ed25519-dalek` checks `SHA256SUMS.sig`
  against the built-in key (`verify_strict`), then `sha2` hashes the
  downloaded file and compares it with its line in `SHA256SUMS`.
- **Installing**:
  - **Linux**: unpack the tarball with `tar` and run its `install.sh
    --update`, which replaces the binaries in `~/.local/bin` and restarts
    the user service, without `sudo` (the udev rule, the `input` group
    and the firewall were set up at the first install).
  - **Windows**: run the verified `Mickey-Setup-<version>.exe /S` through
    `ShellExecute` with `runas` (one UAC prompt). The installer already
    stops Mickey, replaces it and starts it again.
- **PCs tell each other their version** over the data link (payload kind
  23, a version string), when the link opens. A PC that hears a newer
  version from a paired PC shows the update even without internet, and
  the PCs tab lists every PC's version.
- **Off switch**: `[updates] check = false` in `config.toml`, and a
  checkbox in *General*. Off, Mickey never contacts the internet.
- **CLI**: `mickey update check` and `mickey update apply`, which the
  tray, the settings window and the MCP server (PRD 0014) all use.

## Requirements

- A tampered `SHA256SUMS`, signature or download is refused, and the
  owner is told why; nothing is run.
- A failed update leaves the running Mickey as it was.
- The check sends nothing about the owner: no ids, no config, no PC
  names; only what any HTTPS request sends (an IP and a user agent,
  `mickey/<version>`).
- Updating never leaves the owner locked out (hard rule 8): the
  installers stop Mickey the normal way, which releases every grab.

## Non-Goals

- Downloading a release from a paired PC instead of GitHub (one PC with
  internet feeding the others). Worth it later; not now.
- Updating automatically without a click.
- Rolling back to an older release (install it by hand).

## Risks

- **The signing key is the trust root.** If it is lost, older Mickeys
  cannot verify newer releases and must be updated by hand once; if it
  leaks, someone could sign a release. The owner keeps an offline copy;
  `tools/release-key/` explains how it was made and how to replace it.
- **No code signing on Windows yet** (#157): Defender may still flag the
  installer. The update path is the same as installing by hand.

## Discoveries

- The README promised "no update checks". With the owner's decision this
  becomes: one request to GitHub's releases, which can be turned off.
