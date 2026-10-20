# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Scaffolding-integrity tests for this directory's shared render helper.

Every chart test here renders through a local `_render` that shells out to
`helm template`, and every one of them has to switch the queue-policy hook Job
off first -- it is enabled by default and its broker values have no defaults, so
a bare render fails. That switch now comes from
:func:`tests.helm.conftest.disable_policy_hook` instead of being pasted into
each module.

Both halves of the hoist can be lost silently, which is why they are pinned:

- **A module that re-pastes the literal `--set`** keeps passing, so nothing
  would report the duplication creeping back.
- **A module that disables the hook AFTER its own `extra_set` overrides** also
  keeps passing, because no test here overrides `policyHook` today. helm takes
  the LAST `--set` for a key, so that ordering is what lets a future test
  override the hook at all; reversed, its override would be silently discarded
  and the test would assert against the wrong render.

The tokenless-auth baseline (:func:`tests.helm.conftest.select_tokenless_auth`)
is held to the same two rules, since the chart also fails a render that leaves
principalSource to be selected without any OpenID configuration.
"""

import ast
from pathlib import Path

import pytest

HELM_TESTS = Path(__file__).resolve().parent

# The module that owns the hook itself: it renders the hook ON, spells the
# escape hatch out as its subject matter, and is not one of the helpers.
HOOK_OWNER = "test_queue_policy_hook_template.py"

DISABLE_HELPER = "disable_policy_hook"
AUTH_HELPER = "select_tokenless_auth"
BASELINE_HELPERS = (DISABLE_HELPER, AUTH_HELPER)
ESCAPE_HATCH_LITERAL = "policyHook.enabled=false"

# The number of render modules at the time of the hoist. A floor, so a scan that
# quietly matched nothing cannot satisfy the checks below vacuously.
MINIMUM_RENDER_MODULES = 14


def _renders_the_chart(source: str) -> bool:
    # Matched on the pieces rather than on one `"helm", "template"` literal:
    # several helpers build that argv across multiple lines.
    return "subprocess.run" in source and '"helm"' in source and '"template"' in source


def _render_modules(helper: str = DISABLE_HELPER) -> list[Path]:
    """Test modules here that shell out to `helm template`.

    The hook owner renders the hook ON by design, so it is exempt from the
    policy-hook helper only; it still takes the tokenless-auth baseline.
    """
    excluded = {Path(__file__).name}
    if helper == DISABLE_HELPER:
        excluded.add(HOOK_OWNER)
    return sorted(
        path
        for path in HELM_TESTS.glob("test_*.py")
        if path.name not in excluded and _renders_the_chart(path.read_text())
    )


def _statement_index(body: list[ast.stmt], predicate) -> int | None:
    return next((i for i, stmt in enumerate(body) if predicate(stmt)), None)


def _is_call_to(helper: str):
    def predicate(stmt: ast.stmt) -> bool:
        return (
            isinstance(stmt, ast.Expr)
            and isinstance(stmt.value, ast.Call)
            and isinstance(stmt.value.func, ast.Name)
            and stmt.value.func.id == helper
        )

    return predicate


def _is_extra_set_loop(stmt: ast.stmt) -> bool:
    return (
        isinstance(stmt, ast.For)
        and isinstance(stmt.iter, ast.Name)
        and stmt.iter.id == "extra_set"
    )


class TestTheRenderBaselineStaysHoisted:
    """Shared baseline helpers, called before each caller's own overrides."""

    def test_the_scan_sees_the_render_modules(self):
        # Guards the checks below from passing for the wrong reason: a scan that
        # matched no module would report nothing wrong with any of them.
        assert len(_render_modules()) >= MINIMUM_RENDER_MODULES

    @pytest.mark.parametrize("helper", BASELINE_HELPERS)
    def test_every_render_module_uses_the_shared_helper(self, helper):
        missing = [
            path.name
            for path in _render_modules(helper)
            if helper not in path.read_text()
        ]

        assert missing == [], (
            f"{missing} render the chart without {helper}() from "
            f"tests/helm/conftest.py; a bare render fails"
        )

    def test_no_render_module_re_pastes_the_escape_hatch(self):
        pasted = [
            path.name
            for path in _render_modules()
            if ESCAPE_HATCH_LITERAL in path.read_text()
        ]

        assert pasted == [], (
            f"{pasted} spell {ESCAPE_HATCH_LITERAL!r} out again instead of "
            f"calling {DISABLE_HELPER}(); that is the duplication the hoist "
            f"removed"
        )

    @pytest.mark.parametrize("helper", BASELINE_HELPERS)
    def test_the_baseline_lands_before_the_callers_own_overrides(self, helper):
        reversed_helpers = []
        for path in _render_modules(helper):
            for node in ast.walk(ast.parse(path.read_text())):
                if not isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
                    continue
                baseline_at = _statement_index(node.body, _is_call_to(helper))
                if baseline_at is None:
                    continue
                overrides_at = _statement_index(node.body, _is_extra_set_loop)
                if overrides_at is None or baseline_at > overrides_at:
                    reversed_helpers.append(f"{path.name}::{node.name}")

        assert reversed_helpers == [], (
            f"{reversed_helpers} call {helper}() at or after applying "
            f"extra_set; helm takes the last --set, so a test's own override "
            f"of the same key would be discarded"
        )
