#!/usr/bin/env python3
# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
"""Keep pre-release versions out of the skills under `skills/`.

Skills track released versions only: plain `X.Y.Z`, or a pre-release
identifier that NVIDIA actually publishes (see `PUBLISHED`). Unreleased
source is described as "unreleased" next to the last released version, never
by its rc, alpha, beta, or development build number.
"""

import re
import sys
from pathlib import Path

# Pre-release versions under which NGC API resources are published
# (skills/usd-content-workspace-expert/references/versions/released-inventory.md).
PUBLISHED = {
    "1.0.0-alpha.1",
    "1.0.0-beta.1",
    "1.0.0-beta.2",
    "1.0.0-beta.3",
    "1.0.0-beta.4",
}
PUBLISHED_SHORT = {"alpha.1", "beta.1", "beta.2", "beta.3", "beta.4"}

FULL = re.compile(
    r"\b\d+\.\d+\.\d+"
    r"(?:-(?:rc|alpha|beta|pre|dev)(?:\.\d+)*"
    r"|(?:a|b|rc)\d+(?:\.dev\d+)?(?:\+[0-9a-z.]+)?"
    r"|\.dev\d+(?:\+[0-9a-z.]+)?)",
    re.IGNORECASE,
)
SHORT = re.compile(r"\b(?:rc|alpha|beta)\.\d+\b|\bdev\d+\+g[0-9a-f]+\b", re.IGNORECASE)


ROOT = Path(__file__).resolve().parents[2]


def check(path: Path) -> list[str]:
    errors = []
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        for match in FULL.finditer(line):
            if match.group(0) not in PUBLISHED:
                errors.append(f"{path.relative_to(ROOT)}:{number}: unreleased version {match.group(0)}")
        for match in SHORT.finditer(FULL.sub("", line)):
            if match.group(0).lower() not in PUBLISHED_SHORT:
                errors.append(f"{path.relative_to(ROOT)}:{number}: unreleased version {match.group(0)}")
    return errors


def main() -> int:
    errors = [e for p in sorted((ROOT / "skills").rglob("*.md")) for e in check(p)]
    for error in errors:
        print(error)
    if errors:
        print(
            "Describe unreleased source as 'unreleased' beside the last released "
            "version; add a published pre-release to PUBLISHED only if NVIDIA "
            "publishes an artifact under it."
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
