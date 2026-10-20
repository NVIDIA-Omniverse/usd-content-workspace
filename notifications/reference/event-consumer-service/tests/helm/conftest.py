# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Shared scaffolding for the chart-render tests.

The queue-policy hook Job (issue b.9dd) is enabled by default and its broker
management values deliberately have no defaults, so a bare `helm template` of
this chart FAILS. Every render helper in this directory that exercises some
other template therefore has to switch the hook off first -- which was pasted
into fourteen of them before it was hoisted here.

The chart also refuses to render without an auth mode it can stand behind:
with principalSource unset and no OpenID configuration, the render fails
rather than guessing. Helpers that exercise other templates therefore select
the tokenless mode with `select_tokenless_auth`, while the auth tests in
test_settings_configmap_template.py render without it.
"""

from __future__ import annotations

# The `--set` pair that switches the queue-policy hook Job off, as `helm`
# argv. A tuple, so a helper cannot mutate the shared copy by accident.
POLICY_HOOK_DISABLE_ARGS = ("--set", "policyHook.enabled=false")


def disable_policy_hook(args: list[str]) -> list[str]:
    """Append the queue-policy hook's off switch to a `helm template` argv.

    The queue-policy hook Job is on by default and its required broker
    management values have no defaults, so a bare render fails. Tests that
    exercise unrelated templates switch the hook off with this.

    Call it BEFORE appending the caller's own `--set` overrides: helm takes the
    last `--set` for a given key, so an explicit `policyHook` setting from a
    test still wins only while this one goes on the command line first.

    `args` is extended in place and also returned, for use in a chained
    expression.
    """
    args.extend(POLICY_HOOK_DISABLE_ARGS)
    return args


# The `--set` pair that selects the tokenless auth mode, as `helm` argv.
TOKENLESS_AUTH_ARGS = ("--set", "principalSource=disabled")


def select_tokenless_auth(args: list[str]) -> list[str]:
    """Append principalSource=disabled to a `helm template` argv.

    The chart default leaves principalSource empty and fails the render when
    no OpenID configuration is present, so tests of unrelated templates pick
    the tokenless mode, which needs no other values.

    Call it BEFORE appending the caller's own `--set` overrides, for the same
    reason as `disable_policy_hook`: a test that sets principalSource (or the
    OpenID values) itself must still win.

    `args` is extended in place and also returned.
    """
    args.extend(TOKENLESS_AUTH_ARGS)
    return args
