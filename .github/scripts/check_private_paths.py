#!/usr/bin/env python3
# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
"""Keep private paths out of the public repository.

`.ovplc/`, `docs/plc/`, and every `evals/` directory stay in the development
repository. Without arguments, check that no other tracked Markdown, reST, or
text file links into them, and that no tracked file names the private repository.
With `--tree DIR`, check that a tree prepared for publishing contains none of them.
"""

import argparse
import re
import subprocess
import sys
from pathlib import Path, PurePosixPath

ALLOWED_LINKS = {
    ("AGENTS.md", "docs/plc/MAINTAINING.md"),
    ("CONTRIBUTING.md", "docs/plc/MAINTAINING.md"),
}
LINK = re.compile(r"\]\(([^)\s]+)\)|<([^>\s]+)>`_|\{src\}`[^`<]*<([^>`]+)>`")
TEXT_SUFFIXES = {".md", ".rst", ".txt"}
PRIVATE_REPO = re.compile("usd-content-workspace" + "-internal")


def is_private(path: PurePosixPath) -> bool:
    parts = path.parts
    return parts[:1] == (".ovplc",) or parts[:2] == ("docs", "plc") or "evals" in parts


def tracked_files(root: Path) -> list[PurePosixPath]:
    out = subprocess.run(
        ["git", "ls-files", "-z"], cwd=root, check=True, capture_output=True
    ).stdout
    return [PurePosixPath(p) for p in out.decode().split("\0") if p]


def resolve(source: PurePosixPath, target: str, from_root: bool) -> PurePosixPath | None:
    target = target.split("#", 1)[0]
    if not target or re.match(r"^[a-z][a-z0-9+.-]*:", target):
        return None
    if from_root or (source.suffix == ".rst" and target.startswith("/")):
        base = PurePosixPath()
    else:
        base = source.parent
    parts: list[str] = []
    for part in (base / target.lstrip("/")).parts:
        if part == "..":
            if parts:
                parts.pop()
        elif part != ".":
            parts.append(part)
    return PurePosixPath(*parts)


def check_links(root: Path) -> list[str]:
    errors = []
    for path in tracked_files(root):
        if is_private(path) or path.suffix not in TEXT_SUFFIXES:
            continue
        text = (root / path).read_text(encoding="utf-8", errors="replace")
        for match in LINK.finditer(text):
            markdown, rst, src = match.groups()
            target = resolve(path, markdown or rst or src, from_root=bool(src))
            if target and is_private(target) and (str(path), str(target)) not in ALLOWED_LINKS:
                line = text.count("\n", 0, match.start()) + 1
                errors.append(f"{path}:{line}: links to private path {target}")
    return errors


def check_names(root: Path) -> list[str]:
    errors = []
    for path in tracked_files(root):
        full = root / path
        if is_private(path) or full.is_symlink() or not full.is_file():
            continue
        data = full.read_bytes()
        if b"\0" in data[:8192]:
            continue
        text = data.decode("utf-8", errors="replace")
        for match in PRIVATE_REPO.finditer(text):
            line = text.count("\n", 0, match.start()) + 1
            errors.append(f"{path}:{line}: names the private repository")
    return errors


def check_tree(tree: Path) -> list[str]:
    return [
        f"{p.relative_to(tree)}: private path in publish tree"
        for p in tree.rglob("*")
        if is_private(PurePosixPath(p.relative_to(tree).as_posix()))
        and not is_private(PurePosixPath(p.parent.relative_to(tree).as_posix()))
    ]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tree", type=Path, help="tree prepared for publishing")
    args = parser.parse_args()
    if args.tree:
        errors = check_tree(args.tree.resolve())
    else:
        root = Path(
            subprocess.run(
                ["git", "rev-parse", "--show-toplevel"], check=True, capture_output=True, text=True
            ).stdout.strip()
        )
        errors = check_links(root) + check_names(root)
    for error in errors:
        print(error, file=sys.stderr)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
