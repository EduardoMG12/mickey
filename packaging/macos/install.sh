#!/bin/sh
# Installs Mickey as ~/Applications/Mickey.app on Apple Silicon macOS (arm64),
# symlinks the CLI into ~/.local/bin, and registers the user LaunchAgent.
#
# Run from the unpacked release directory (or from this repo after `cargo build`):
#   ./install.sh
#   ./install.sh --update   (used by `mickey update apply`)
set -eu

update=0
[ "${1:-}" = "--update" ] && update=1

if [ "$(uname -s)" != "Darwin" ] || [ "$(uname -m)" != "arm64" ]; then
    echo "Mickey for macOS supports Apple Silicon (arm64) only." >&2
    exit 1
fi

here=$(cd "$(dirname "$0")" && pwd)
src="$here"
if [ ! -x "$src/mickey" ]; then
    if [ -x "$here/../../target/release/mickey" ]; then
        src=$(cd "$here/../../target/release" && pwd)
    elif [ -x "$here/../../target/debug/mickey" ]; then
        src=$(cd "$here/../../target/debug" && pwd)
    fi
fi

if [ ! -x "$src/mickey" ] || [ ! -x "$src/mickey-ui" ]; then
    echo "Cannot find mickey and mickey-ui binaries in $src" >&2
    exit 1
fi

version=$("$src/mickey" --version 2>/dev/null | awk '{print $2}')
if [ -z "$version" ]; then
    echo "Mickey cannot start here:" >&2
    "$src/mickey" --version 2>&1 | head -1 >&2
    exit 1
fi

app="$HOME/Applications/Mickey.app"
contents="$app/Contents"
macos_dir="$contents/MacOS"
res_dir="$contents/Resources"
bin="$HOME/.local/bin"

echo "Installing Mickey $version to $app"
mkdir -p "$macos_dir" "$res_dir" "$bin"
install -m 755 "$src/mickey" "$src/mickey-ui" "$macos_dir/"

sed "s|__VERSION__|$version|g" "$here/Info.plist" > "$contents/Info.plist"

# Build Mickey.icns from the bundled PNG using macOS's built-in sips + iconutil.
icon_png=""
for candidate in "$here/mickey.png" "$here/../../assets/mascot/ratao.png"; do
    if [ -f "$candidate" ]; then
        icon_png="$candidate"
        break
    fi
done
if [ -n "$icon_png" ] && command -v sips >/dev/null && command -v iconutil >/dev/null; then
    iconset=$(mktemp -d "${TMPDIR:-/tmp}/mickey-icon.XXXXXX")/Mickey.iconset
    mkdir -p "$iconset"
    for size in 16 32 128 256 512; do
        sips -z "$size" "$size" "$icon_png" --out "$iconset/icon_${size}x${size}.png" >/dev/null 2>&1 || true
        size2=$((size * 2))
        sips -z "$size2" "$size2" "$icon_png" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null 2>&1 || true
    done
    iconutil -c icns "$iconset" -o "$res_dir/Mickey.icns" 2>/dev/null || true
    rm -rf "$(dirname "$iconset")"
fi

if command -v codesign >/dev/null; then
    codesign --force --deep --sign - "$app" >/dev/null 2>&1 || true
fi

ln -sf "$macos_dir/mickey" "$bin/mickey"
ln -sf "$macos_dir/mickey-ui" "$bin/mickey-ui"

echo "Configuring and starting the Mickey LaunchAgent"
"$macos_dir/mickey" autostart on

uid=$(/usr/bin/id -u)
label="io.github.eduardomg12.mickey"
domain="gui/$uid"
service="$domain/$label"
plist="$HOME/Library/LaunchAgents/$label.plist"

if [ "$update" = 1 ] && /bin/launchctl print "$service" >/dev/null 2>&1; then
    # Restart in place so launchd atomically swaps to the newly installed binary
    # even when `install.sh --update` was spawned by the running agent.
    /bin/launchctl kickstart -k "$service"
else
    # `bootout` tears down a running job asynchronously; wait briefly for it to
    # finish unloading before bootstrapping the updated plist.
    if /bin/launchctl bootout "$service" 2>/dev/null; then
        i=0
        while [ "$i" -lt 30 ]; do
            if ! /bin/launchctl print "$service" >/dev/null 2>&1; then
                break
            fi
            sleep 0.1
            i=$((i + 1))
        done
    fi
    attempt=0
    while [ "$attempt" -lt 5 ]; do
        if /bin/launchctl bootstrap "$domain" "$plist"; then
            break
        fi
        attempt=$((attempt + 1))
        if [ "$attempt" -eq 5 ]; then
            echo "Cannot bootstrap $plist via launchctl." >&2
            exit 1
        fi
        sleep 0.2
    done
fi

if [ "$update" = 0 ]; then
    echo
    echo "Done. Mickey is installed at $app (CLI linked in $bin)."
    echo "Grant Mickey.app permission in System Settings > Privacy & Security:"
    echo "  - Accessibility (required to control and be controlled)"
    echo "  - Input Monitoring (required to control other PCs)"
    echo "  - Screen & System Audio Recording (optional, for shared audio)"
    case ":${PATH:-}:" in
        *":$bin:"*) ;;
        *)
            echo
            echo "Tip: add $bin to your PATH to run 'mickey' from any terminal."
            ;;
    esac
fi
