#!/usr/bin/env bash
# Makes the Ed25519 key that signs Mickey's releases (PRD 0013), once.
#
# - The private key goes to ~/.config/mickey-release/release-key.pem (only
#   you can read it) and to the repository secret RELEASE_SIGNING_KEY,
#   which release.yml signs with.
# - The public key goes to packaging/release-key.pub, which is built into
#   Mickey: commit it.
#
# Keep an offline copy of the .pem (a password manager, a USB stick). Lost,
# released Mickeys can no longer verify a new release and must be updated
# by hand once; leaked, someone could sign a release.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
dir="$HOME/.config/mickey-release"
key="$dir/release-key.pem"

if [ -e "$key" ]; then
    echo "A key already exists: $key. Move it away first to make a new one." >&2
    exit 1
fi
mkdir -p "$dir"
chmod 700 "$dir"
(umask 077 && openssl genpkey -algorithm ed25519 -out "$key")

# The raw 32-byte public key is the last 32 bytes of its DER form.
public="$(openssl pkey -in "$key" -pubout -outform DER | tail -c 32 | od -An -tx1 | tr -d ' \n')"
{
    echo "# The Ed25519 public key that signs Mickey's releases (PRD 0013),"
    echo "# made by tools/release-key/new-key.sh. Mickey installs an update only"
    echo "# when its SHA256SUMS carries a valid signature from this key."
    echo "$public"
} > "$repo/packaging/release-key.pub"

gh secret set RELEASE_SIGNING_KEY --repo EduardoMG12/mickey < "$key"

echo "Private key: $key (back it up offline now)"
echo "Public key:  $public (in packaging/release-key.pub; commit it)"
