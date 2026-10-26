#!/usr/bin/env bash
# Signs a release's SHA256SUMS with the release key (PRD 0013):
#   sign.sh <release-key.pem> <SHA256SUMS>   writes <SHA256SUMS>.sig
# The signature is the raw 64 bytes Ed25519 makes over the file. It then
# checks the signature against packaging/release-key.pub, so a release is
# never published signed by a key Mickey does not know.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
key="$1"
sums="$2"

openssl pkeyutl -sign -inkey "$key" -rawin -in "$sums" -out "$sums.sig"
"$here/verify.sh" "$sums"
