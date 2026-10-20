# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Self-contained, in-repo Apache-2.0 header lint.

Fails closed when a published NVIDIA-authored Python source file is missing the
wrapped 3-physical-line ``Apache-2.0`` header written by the Epic 2 license
swap, so the Apache posture cannot silently regress. This module is published
in the GitHub mirror and is under the package coverage gate, so its core scan
is written as a pure, injectable function (no process exit / print) that the
tests can drive against a temporary fixture tree.

The expected header is the 2-statement SPDX form with the copyright statement
physically wrapped across two comment lines (a single-line copyright is 98
characters and would violate the repo's flake8 ``max-line-length = 88``)::

    # SPDX-FileCopyrightText: Copyright (c) <year> NVIDIA CORPORATION &
    # AFFILIATES. All rights reserved.
    # SPDX-License-Identifier: Apache-2.0

The copyright year varies and is matched as a 4-digit wildcard; every other
character of the three lines is matched exactly. An optional leading shebang
line and an optional PEP 263 encoding line (which protoc writes on line 1 of
every ``*_pb2.py``) are tolerated.

The generated ``nvidia/`` stubs are in scope: ``buf generate`` emits them with
no header, so ``uv run nc-update-proto`` stamps each one with :func:`stamp_header`,
and a regeneration that skips the stamp fails this lint.
"""

import re
import sys
from pathlib import Path

# The published NVIDIA-authored Python source roots, relative to the repo root.
# Kept as an explicit, documented list so the alignment with Epic 1's published
# set is auditable. These three trees carry the header: the authored
# ``notification_common/`` and ``tests/`` trees, and the generated ``nvidia/``
# PEP-420 namespace tree (stamped by ``update_proto``). Every ``.publishignore``
# path (``docs/``, root ``scripts/``, ``.vscode/``, ``.cursorrules``,
# ``.bees/``, ``.idea/``, ...) and the ``tools/`` YAML configs are intentionally
# out of scope.
SOURCE_ROOTS = ("notification_common", "tests", "nvidia")

# Source file globs checked under each root: Python modules and the generated
# ``.pyi`` type stubs.
SOURCE_GLOBS = ("*.py", "*.pyi")

# Directory names pruned during the walk: byte-compiled caches.
EXCLUDED_DIR_NAMES = frozenset({"__pycache__"})

# The three expected header lines. The copyright year is a 4-digit wildcard;
# everything else is matched literally. Lines are compared after stripping
# trailing whitespace / line endings so CRLF checkouts are tolerated.
_EXPECTED_HEADER_LINES = (
    re.compile(
        r"^# SPDX-FileCopyrightText: Copyright \(c\) \d{4} " r"NVIDIA CORPORATION &$"
    ),
    re.compile(r"^# AFFILIATES\. All rights reserved\.$"),
    re.compile(r"^# SPDX-License-Identifier: Apache-2\.0$"),
)

# The header :func:`stamp_header` writes, in the exact form the lint expects.
_HEADER_TEMPLATE = (
    "# SPDX-FileCopyrightText: Copyright (c) {year} NVIDIA CORPORATION &\n"
    "# AFFILIATES. All rights reserved.\n"
    "# SPDX-License-Identifier: Apache-2.0\n"
)

# A PEP 263 encoding declaration. It only takes effect on line 1 or 2, so the
# header goes after it rather than before.
_CODING_RE = re.compile(r"^[ \t\f]*#.*?coding[:=]")


def repo_root() -> Path:
    """Return the repo root, resolved relative to this module's location.

    Walking up from ``notification_common/scripts/header_lint.py`` yields the
    repo root so ``uv run nc-lint`` works from any current working directory.
    """
    return Path(__file__).resolve().parent.parent.parent


def _preamble_length(lines: list[str]) -> int:
    """Return how many leading lines must stay above the header.

    That is an optional shebang (``#!...``) followed by an optional PEP 263
    encoding declaration.
    """
    count = 0
    if count < len(lines) and lines[count].startswith("#!"):
        count += 1
    if count < len(lines) and _CODING_RE.match(lines[count]):
        count += 1
    return count


def _has_expected_header(path: Path) -> bool:
    """Return ``True`` when ``path`` opens with the expected wrapped header.

    The header must appear as the leading comment lines, after an optional
    shebang line and an optional encoding declaration.
    """
    try:
        with path.open("r", encoding="utf-8") as handle:
            # Read only the small window we need: an optional shebang and
            # encoding line plus the three header lines.
            lines = [handle.readline() for _ in range(len(_EXPECTED_HEADER_LINES) + 2)]
    except (OSError, UnicodeDecodeError):
        return False

    stripped = [line.rstrip("\r\n") for line in lines]
    del stripped[: _preamble_length(stripped)]

    if len(stripped) < len(_EXPECTED_HEADER_LINES):
        return False

    return all(
        pattern.match(line) for pattern, line in zip(_EXPECTED_HEADER_LINES, stripped)
    )


def stamp_header(path: Path, year: int) -> bool:
    """Stamp the expected Apache-2.0 header onto ``path`` if it has none.

    The header goes after any shebang or encoding line and is followed by a
    blank line. A file that already carries any SPDX header is left alone, so the
    stamp is idempotent and never stacks a second header on a wrong one (the
    lint still flags that file).

    Returns:
        Whether ``path`` changed.
    """
    text = path.read_text(encoding="utf-8")
    if "SPDX-License-Identifier" in text:
        return False
    lines = text.splitlines(keepends=True)
    split = _preamble_length(lines)
    header = _HEADER_TEMPLATE.format(year=year) + "\n"
    path.write_text("".join([*lines[:split], header, *lines[split:]]), "utf-8")
    return True


def find_missing_headers(root: Path | str | None = None) -> list[Path]:
    """Return the source files under the source roots missing the header.

    Args:
        root: Scan root. Defaults to the repo root resolved relative to
            ``__file__`` so the check runs from any CWD; tests inject a
            temporary fixture tree here.

    Returns:
        The sorted list of source files that do not carry the expected wrapped
        Apache-2.0 header. Pure: no printing and no process exit.
    """
    base = repo_root() if root is None else Path(root)

    violations: list[Path] = []
    for source_root in SOURCE_ROOTS:
        search_dir = base / source_root
        if not search_dir.is_dir():
            continue
        source_files = (
            path for glob in SOURCE_GLOBS for path in search_dir.rglob(glob)
        )
        for py_file in source_files:
            # Match exclude-dir names against the path relative to the scanned
            # source root so only directories inside the tree are pruned. Using
            # the absolute ``py_file.parts`` would spuriously match ancestor
            # directories of the checkout (e.g. a parent literally named
            # ``__pycache__``) and vacuously exclude every file.
            if EXCLUDED_DIR_NAMES.intersection(py_file.relative_to(search_dir).parts):
                continue
            if not _has_expected_header(py_file):
                violations.append(py_file)

    return sorted(violations)


def check_headers() -> None:
    """CLI runner: scan the default root and exit non-zero on any violation.

    Prints the offending file paths and raises ``SystemExit(1)`` so the
    enclosing ``nc-lint`` command fails. Returns cleanly when all files comply.
    """
    violations = find_missing_headers()
    if violations:
        print(
            "Header lint failed: the following published NVIDIA-authored "
            "source files are missing the wrapped Apache-2.0 header:",
            file=sys.stderr,
        )
        for path in violations:
            print(f"  {path}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    check_headers()
