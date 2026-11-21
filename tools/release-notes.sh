#!/usr/bin/env bash
# Prints the text of the GitHub release for tag v<version> (#529).
#
#   tools/release-notes.sh v0.6.0 owner/repo
#
# docs/releases/<version>.md when there is one, written for the release
# page, with images; its image paths, relative to docs/releases/, become
# links to the files at that tag (a release page cannot show relative
# ones). Otherwise, that version's section of CHANGELOG.md. The release
# workflow adds the list of pull requests after it.
set -euo pipefail
tag="$1"
repo="$2"
version="${tag#v}"
here="$(cd "$(dirname "$0")/.." && pwd)"
notes="$here/docs/releases/$version.md"
if [ -f "$notes" ]; then
  sed "s#](\($version/[^)]*\))#](https://raw.githubusercontent.com/$repo/$tag/docs/releases/\1)#g" "$notes"
else
  # The lines under "## <version>", up to the next "## ".
  awk -v heading="## $version" '
    $0 == heading { inside = 1; next }
    inside && /^## / { exit }
    inside { print }
  ' "$here/CHANGELOG.md"
fi
