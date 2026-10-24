#!/usr/bin/env python3
"""Writes the licenses of every crate built into Mickey for one target.

MIT, Apache-2.0, BSL-1.0 and the fonts' OFL ask that their notices travel
with binary copies, so the release puts this file next to the programs.

    python3 tools/licenses/third_party_licenses.py <target-triple> > THIRD-PARTY-LICENSES.txt

It reads `cargo metadata` (only crates used on that target, without
dev-dependencies) and each crate's own license files from cargo's source
cache. A crate that ships no license file is listed with its license name
and authors, and counted on stderr.
"""

import json
import os
import subprocess
import sys

# Files that hold a crate's license or notices, by the start of their name.
NAMES = ("license", "licence", "copying", "notice", "copyright", "ofl", "ufl")


# For crates that ship no license file. Apache-2.0's full text is above,
# in the many crates that ship LICENSE-APACHE.
APPENDIX = """
--- MIT

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

--- Apache-2.0

See any LICENSE-APACHE above (the same text in every crate).

--- Unlicense

This is free and unencumbered software released into the public domain
(https://unlicense.org).
"""


def license_files(folder):
    """The crate's license files: at its root, and one folder down (where
    a font crate keeps its fonts' licenses)."""
    found = []
    for root, dirs, files in os.walk(folder):
        if root.count(os.sep) - folder.count(os.sep) > 1:
            dirs[:] = []
            continue
        dirs[:] = sorted(d for d in dirs if d not in ("target", ".git", "tests", "benches"))
        for name in sorted(files):
            if name.lower().startswith(NAMES):
                found.append(os.path.join(root, name))
    return found


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    target = sys.argv[1]
    metadata = json.loads(
        subprocess.check_output(
            ["cargo", "metadata", "--format-version", "1", "--filter-platform", target, "--locked"]
        )
    )
    packages = {package["id"]: package for package in metadata["packages"]}
    workspace = set(metadata["workspace_members"])
    # Walk the resolved graph from the workspace, skipping dev-dependencies.
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    used, todo = set(), list(workspace)
    while todo:
        id = todo.pop()
        if id in used:
            continue
        used.add(id)
        for dep in nodes[id]["deps"]:
            if any(kind["kind"] != "dev" for kind in dep["dep_kinds"]):
                todo.append(dep["pkg"])
    crates = sorted((packages[id] for id in used - workspace), key=lambda p: (p["name"], p["version"]))

    out = sys.stdout
    out.write("Mickey is GPL-3.0-or-later (see LICENSE). It is built with the\n")
    out.write(f"following {len(crates)} crates, under their own licenses.\n")
    without = 0
    for crate in crates:
        out.write("\n" + "=" * 72 + "\n")
        out.write(f"{crate['name']} {crate['version']}: {crate.get('license') or 'see below'}\n")
        if crate.get("repository"):
            out.write(f"{crate['repository']}\n")
        files = license_files(os.path.dirname(crate["manifest_path"]))
        if not files:
            without += 1
            authors = ", ".join(crate.get("authors") or []) or "its authors"
            out.write(f"(no license file in the crate) Copyright {authors}.\n")
        for path in files:
            out.write(f"\n--- {os.path.relpath(path, os.path.dirname(crate['manifest_path']))}\n\n")
            with open(path, encoding="utf-8", errors="replace") as text:
                out.write(text.read().strip() + "\n")
    if without:
        out.write("\n" + "=" * 72 + "\n")
        out.write("The licenses named by crates that ship no license file:\n")
        out.write(APPENDIX)
    print(f"{len(crates)} crates, {without} without a license file", file=sys.stderr)


if __name__ == "__main__":
    main()
