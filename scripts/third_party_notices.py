#!/usr/bin/env python3
"""Generates THIRD_PARTY_NOTICES.md from the crates linked into the release.

The release binary is statically linked, so every crate's notice travels with
it. MIT and Apache-2.0 both require the copyright notice and the licence text
to accompany a binary redistribution, and publishing only perfo's own MIT does
not satisfy that.

Scope is the normal dependencies of the release target, NOT everything in
Cargo.lock. Listing the lock file would claim the binary contains LGPL and MPL
code (`r-efi`, `termina`), which only build for other targets. In a licensing
document an over-broad claim is a wrong claim.

    python3 scripts/third_party_notices.py            # writes the file
    python3 scripts/third_party_notices.py --check     # exit 1 if stale
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys

TARGET = "x86_64-unknown-linux-musl"
OUTPUT = "THIRD_PARTY_NOTICES.md"
LICENSE_FILENAMES = ("LICENSE", "LICENCE", "COPYING", "NOTICE", "UNLICENSE")

# One representative crate per licence text, so the full text appears once
# rather than 46 times. Chosen by SPDX id found in the crate's own files.
TEXT_ORDER = [
    "MIT",
    "Apache-2.0",
    "BSL-1.0",
    "Zlib",
    "Unlicense",
    "Unicode-3.0",
]

# `Apache-2.0 WITH LLVM-exception` needs no separate text here: both crates
# carrying it (`rustix`, `linux-raw-sys`) also offer plain MIT and Apache-2.0,
# and the Apache text is already reproduced in full.


def run(args: list[str]) -> str:
    """Runs a command, raising with its stderr when it fails."""
    done = subprocess.run(args, check=False, capture_output=True, text=True)
    if done.returncode != 0:
        raise RuntimeError(f"{' '.join(args)} failed: {done.stderr.strip()}")
    return done.stdout


def shipped_crate_names() -> set[str]:
    """The crates linked into the release target."""
    tree = run(
        ["cargo", "tree", "--target", TARGET, "-e", "normal",
         "--prefix", "none", "--no-dedupe"]
    )
    names = set()
    for line in tree.splitlines():
        parts = line.split()
        if parts and parts[0] and parts[0] != "perfo":
            names.add(parts[0])
    if not names:
        raise RuntimeError("cargo tree listed no dependencies")
    return names


def license_files(crate_dir: str) -> list[tuple[str, str]]:
    """Every licence-looking file in a crate, as (filename, content)."""
    if not os.path.isdir(crate_dir):
        return []
    found = []
    for name in sorted(os.listdir(crate_dir)):
        if name.upper().startswith(LICENSE_FILENAMES):
            path = os.path.join(crate_dir, name)
            try:
                with open(path, encoding="utf-8", errors="replace") as handle:
                    found.append((name, handle.read()))
            except OSError:
                continue
    return found


def copyright_lines(files: list[tuple[str, str]]) -> list[str]:
    """The copyright notices a crate carries, which are the required part.

    Apache-2.0's own text contains a `Copyright [yyyy]` placeholder in its
    appendix, so a match that still has brackets is boilerplate and dropped.
    """
    seen: list[str] = []
    for _, text in files:
        for line in text.splitlines():
            stripped = line.strip()
            if not re.match(r"(?i)copyright\b", stripped):
                continue
            if "[yyyy]" in stripped or "<year>" in stripped:
                continue
            if stripped not in seen:
                seen.append(stripped)
    return seen


# How to recognise each licence text inside a crate's files. Filenames are not
# enough and the SPDX id does not appear in the text: `foldhash` ships Zlib as a
# bare `LICENSE`, and `ryu` ships BSL-1.0 as `LICENSE-BOOST`. Matching on a
# phrase from the text itself is what finds all seven.
TEXT_MARKERS = {
    "MIT": ("Permission is hereby granted, free of charge",),
    "Apache-2.0": ("Apache License",),
    "BSL-1.0": ("Boost Software License",),
    "Zlib": ("altered source versions must be plainly marked",),
    "Unlicense": ("This is free and unencumbered software",),
    "Unicode-3.0": ("UNICODE LICENSE V3",),
}


def pick_text(files: list[tuple[str, str]], spdx: str) -> str | None:
    """A crate's file holding the text for `spdx`, found by a known phrase."""
    markers = TEXT_MARKERS.get(spdx)
    if not markers:
        return None
    for _, text in files:
        lowered = text.lower()
        if any(marker.lower() in lowered for marker in markers):
            return text
    return None


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true",
                        help="exit 1 when the file on disk is out of date")
    args = parser.parse_args()

    meta = json.loads(run(["cargo", "metadata", "--format-version", "1",
                           "--locked"]))
    packages = {p["name"]: p for p in meta["packages"]}
    shipped = sorted(shipped_crate_names())

    missing = [n for n in shipped if n not in packages]
    if missing:
        raise RuntimeError(f"crates with no metadata: {missing}")

    rows = []
    texts: dict[str, str] = {}
    for name in shipped:
        pkg = packages[name]
        files = license_files(os.path.dirname(pkg["manifest_path"]))
        rows.append(
            {
                "name": name,
                "version": pkg["version"],
                "license": pkg.get("license") or "(not declared)",
                "repository": pkg.get("repository") or "",
                "copyrights": copyright_lines(files),
            }
        )
        for spdx in TEXT_ORDER:
            if spdx in texts:
                continue
            if spdx in (pkg.get("license") or ""):
                text = pick_text(files, spdx)
                if text:
                    texts[spdx] = text.strip()

    out = [
        "# Third-Party Notices",
        "",
        (
            "`bin/perfo` is statically linked, so the code of the crates "
            "below travels inside it."
        ),
        (
            "This file reproduces the copyright notices and licence texts "
            "their terms require."
        ),
        "",
        (
            "Generated by `scripts/third_party_notices.py` from the normal "
            f"dependencies of `{TARGET}`."
        ),
        (
            "Crates that build only for other targets are excluded: claiming "
            "them would describe a binary"
        ),
        "that does not exist.",
        "",
        (
            f"{len(rows)} crates, under "
            f"{len([spdx for spdx in TEXT_ORDER if spdx in texts])} distinct "
            "licences."
        ),
        "",
        "## Crates",
        "",
        "| Crate | Version | Licence |",
        "| --- | --- | --- |",
    ]
    for row in rows:
        out.append(f"| `{row['name']}` | {row['version']} | {row['license']} |")

    out += ["", "## Copyright notices", ""]
    for row in rows:
        if not row["copyrights"]:
            continue
        out.append(f"**{row['name']} {row['version']}**")
        if row["repository"]:
            out.append(f"<{row['repository']}>")
        out.append("")
        for line in row["copyrights"]:
            out.append(f"    {line}")
        out.append("")

    for spdx in TEXT_ORDER:
        if spdx not in texts:
            continue
        out += [f"## {spdx}", "", "```", texts[spdx], "```", ""]

    rendered = "\n".join(out).rstrip() + "\n"

    if args.check:
        try:
            with open(OUTPUT, encoding="utf-8") as handle:
                current = handle.read()
        except OSError:
            print(f"{OUTPUT} is missing; run scripts/third_party_notices.py",
                  file=sys.stderr)
            return 1
        if current != rendered:
            print(f"{OUTPUT} is out of date; run "
                  f"scripts/third_party_notices.py", file=sys.stderr)
            return 1
        print(f"{OUTPUT} is current ({len(rows)} crates)")
        return 0

    with open(OUTPUT, "w", encoding="utf-8") as handle:
        handle.write(rendered)
    print(f"wrote {OUTPUT}: {len(rows)} crates, {len(rendered)} bytes")
    return 0


if __name__ == "__main__":
    sys.exit(main())
