#!/usr/bin/env bash
# Checks <SHA256SUMS>.sig against the public key in packaging/release-key.pub,
# the way Mickey does before installing an update (PRD 0013):
#   verify.sh <SHA256SUMS>
# Anyone can run it on a downloaded release, then `sha256sum -c SHA256SUMS`.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
sums="$1"
public="$(grep -v '^#' "$here/../../packaging/release-key.pub" | tr -d ' \n')"
if [ "${#public}" -ne 64 ]; then
    echo "packaging/release-key.pub has no key yet: run tools/release-key/new-key.sh" >&2
    exit 1
fi

# An Ed25519 public key in DER: a fixed 12-byte header, then the 32 bytes.
pem="$(mktemp)"
trap 'rm -f "$pem"' EXIT
{
    echo "-----BEGIN PUBLIC KEY-----"
    printf '%b' "$(echo "302a300506032b6570032100$public" | sed 's/../\\x&/g')" | base64
    echo "-----END PUBLIC KEY-----"
} > "$pem"
openssl pkeyutl -verify -pubin -inkey "$pem" -rawin -in "$sums" -sigfile "$sums.sig"
