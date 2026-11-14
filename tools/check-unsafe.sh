#!/usr/bin/env bash
# Hard rule 6 (CLAUDE.md, CONTRIBUTING.md): `unsafe` lives only in
# crates/mickey/src/platform/. Fails, naming each line, if it appears
# anywhere else. A comment that only mentions the word counts too: reword it.
set -euo pipefail

found=$(grep -rnw --include='*.rs' unsafe crates | grep -v '^crates/mickey/src/platform/' || true)
if [ -n "$found" ]; then
    echo "unsafe outside crates/mickey/src/platform/:" >&2
    echo "$found" >&2
    exit 1
fi
echo "unsafe only in crates/mickey/src/platform/"
