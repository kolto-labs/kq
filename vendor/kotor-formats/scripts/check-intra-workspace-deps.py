#!/usr/bin/env python3
"""Fail when one crate here names a sibling at a version.

These crates are not published to crates.io; consumers take them by git ref, and
inside the workspace a `path` dependency outranks any `version` beside it. So a
version requirement is never exercised here — it is exercised in the consumer,
after a release, where a stale one breaks the published tag outright:

    error: failed to select a version for the requirement `kotor-formats = "^0.1.0"`
    candidate versions found which didn't match: 0.2.0

That happened, and no test caught it, because no test could. This is the check
that would have: intra-repository dependencies carry a path and nothing else.
"""

import json
import subprocess
import sys


def main() -> int:
    meta = json.loads(
        subprocess.run(
            ["cargo", "metadata", "--format-version", "1", "--no-deps"],
            capture_output=True,
            text=True,
            check=True,
        ).stdout
    )

    siblings = {package["name"] for package in meta["packages"]}
    offences = []

    for package in meta["packages"]:
        for dependency in package["dependencies"]:
            if dependency["name"] not in siblings:
                continue
            requirement = dependency.get("req", "*")
            if requirement != "*":
                offences.append(
                    f"  {package['name']} names {dependency['name']} at "
                    f'"{requirement}", but {dependency["name"]} is at '
                    f"{next(p['version'] for p in meta['packages'] if p['name'] == dependency['name'])}"
                )

    if offences:
        print("Intra-repository dependencies must name a path and no version.\n")
        print("\n".join(offences))
        print(
            "\nDrop the version field. Nothing here is published, so it buys "
            "nothing\nand goes stale the moment one crate is released without "
            "the other."
        )
        return 1

    print(f"ok: {len(siblings)} crates, no intra-repository version requirements")
    return 0


if __name__ == "__main__":
    sys.exit(main())
