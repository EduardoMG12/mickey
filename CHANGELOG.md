# What's new in Mickey

What changed for you in each version, newest first. Mickey shows the part
for its own version once, the first time it starts after an update.

Writing an entry: one line per change people will notice, in plain words
("Taps on the touchpad click on the other PC"), under `## Unreleased`. The
release renames that heading to the version. Internal work (refactors, CI,
dependencies) stays out unless it fixes something people saw. Plain text
only: no arrows or emoji, which Mickey's font may not have.

## Unreleased

- The clipboard can follow the keyboard only when you ask: choose "Only
  when I press AltGr + Right Shift + C" in Settings > General, then press
  it on the PC you went to, to bring what you copied on the one you left.
- On Windows, Mickey starts at logon without a script: the way it was
  started is what Windows Defender mistook for malware on new installs.
- Pairing with a PC whose pairing is closed, or that says no, no longer
  leaves a code on one screen only: the code closes at once and a window
  says why and what to do.
- On Windows, files from your other PCs no longer make SmartScreen or
  Office warn: they are marked as from your local network, and say which
  PC they came from. Settings > General can bring the warnings back.
- Copied folders, and files of any size, now cross with copy and paste:
  they arrive checked, and Ctrl+V pastes them once they are all there.
  Over 1 GB a small window asks first whether to bring them, into
  Downloads/Mickey; where it appears, and the 1 GB, are in Settings >
  General. Links inside a folder are left out. A transfer that takes a while shows its
  progress in the same corner, with Cancel. With a firewall on, let in
  TCP 4244.
- What you copied arrives on the other PC faster: each crossing no longer
  waits about a tenth of a second for the network.
- A copied image is no longer sent again every time the keyboard goes
  back and forth, only when something new is copied, so it arrives more
  reliably. Thanks to @wendelmax, who found it.

## 0.5.0

- Taps and two-finger scrolling on a laptop's touchpad now work while it
  controls another PC, following GNOME's, KDE's or Hyprland's touchpad
  settings. Settings > This PC sets tap to click and natural scrolling by
  hand. Thanks to @bigptlg.
- Mickey speaks your language: English, Brazilian Portuguese and European
  Portuguese, following the system or picked in Settings > General.
- Screen edges work on COSMIC (Pop!_OS 24.04) in both directions, and on
  other desktops without the InputCapture portal.
- The clipboard can be kept on each PC: Settings > General > Share the
  clipboard > Never.
- Pairing says why it did not work, and a PC is remembered only once the
  other one accepts it.
- A PC that stops answering no longer holds the keyboard and mouse on the
  PC it was controlling.
- On Windows, the installer says so when Defender removes Mickey right
  after installing it.
