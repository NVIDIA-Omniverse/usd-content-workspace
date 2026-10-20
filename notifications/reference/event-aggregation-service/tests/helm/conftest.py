# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Shared scaffolding for the chart-render tests.

Every module in this directory does the same four things: locate the chart,
skip itself when the helm CLI is absent, shell out to `helm template`, and pull
a single manifest of a known kind out of a `--show-only` render. That block was
copy-pasted across three modules before it was hoisted here.
"""

from __future__ import annotations

import shutil
import subprocess
from pathlib import Path

import pytest
import yaml

CHART_PATH = Path(__file__).resolve().parents[2] / "helm" / "event-aggregation-service"
SETTINGS_CONFIGMAP_TEMPLATE = "templates/settings-configmap.yaml"

# Bound by each module as `pytestmark = requires_helm`. A bare `pytestmark` in a
# conftest does NOT propagate: pytest reads that name off the test module
# itself, so the condition is defined once here and applied per module.
requires_helm = pytest.mark.skipif(
    shutil.which("helm") is None,
    reason="helm CLI not available",
)


# The chart refuses to render without an auth configuration (principalSource
# unset and no OpenID provider configured), so a render that is not about auth
# pins the tokenless mode through this base. The auth tests pass `base=()` to
# render the chart's own selection.
TOKENLESS = ("principalSource=disabled",)


def render_raw(
    *extra_set: str,
    show_only: str | None = None,
    base: tuple[str, ...] = TOKENLESS,
    set_string: tuple[str, ...] = (),
) -> subprocess.CompletedProcess:
    """Run `helm template` and return the CompletedProcess unchecked.

    Used by the render-time guard tests, which assert on a NON-zero exit; the
    checked `render` wrapper below is for the success-path renders. `base` is
    applied before `extra_set`, so a test can override it key by key, and
    `set_string` passes values through `--set-string`, which keeps a "true"
    or "null" a string.
    """
    args = ["helm", "template", "ea", str(CHART_PATH), "--namespace", "storage-api"]
    if show_only is not None:
        args.extend(["--show-only", show_only])
    for kv in (*base, *extra_set):
        args.extend(["--set", kv])
    for kv in set_string:
        args.extend(["--set-string", kv])
    return subprocess.run(args, capture_output=True, text=True)


def render(
    *extra_set: str,
    show_only: str | None = None,
    base: tuple[str, ...] = TOKENLESS,
    set_string: tuple[str, ...] = (),
) -> str:
    """Run `helm template`, assert it succeeded, and return its stdout."""
    result = render_raw(
        *extra_set, show_only=show_only, base=base, set_string=set_string
    )
    assert result.returncode == 0, (
        f"helm template failed:\nstdout:\n{result.stdout}\n" f"stderr:\n{result.stderr}"
    )
    return result.stdout


def settings_data(*extra_set: str, base: tuple[str, ...] = TOKENLESS) -> dict:
    """Render the settings ConfigMap and return its `data` mapping."""
    output = render(*extra_set, show_only=SETTINGS_CONFIGMAP_TEMPLATE, base=base)
    return single_document(output, "ConfigMap")["data"]


def single_document(output: str, kind: str) -> dict:
    """Return the one manifest in a scoped render, asserting its kind.

    The kind assertion is what keeps a `--show-only` path typo from silently
    handing a test some other template's document.
    """
    docs = [d for d in yaml.safe_load_all(output) if d]
    assert len(docs) == 1, f"expected exactly one document, got {len(docs)}:\n{output}"
    assert docs[0]["kind"] == kind, f"unexpected kind:\n{output}"
    return docs[0]
