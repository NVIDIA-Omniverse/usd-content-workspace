# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""The pin-bump coverage guard for publisher-relevant settings.

Issue b.8xv's root cause: the 6.3.2 -> 6.10.1 notification-common bump grew
`CommonSettings` new fields (the channel-pool overflow band, `channel_wait_ms`,
the drain window and its poll interval) whose non-`None` defaults became the
values every deployed pod ran with -- no chart knob, no render error, no signal
of any kind that a library default had quietly become a production setting.
This module makes the NEXT pin bump report that gap instead of deploying it: it
reads the field names off the settings model at import time and fails when a
publisher-relevant field is neither templated into the settings ConfigMap nor
explicitly recorded below as a deliberate omission.

RESOLVING A FAILURE from TestSettingsCoverageIsExhaustive: the named field is
new and unclassified, so decide. Either (a) template it -- add the values key
and the settings-configmap.yaml row, then add the field to `TEMPLATED` -- or
(b) leave it at the library default and add it to `DELIBERATELY_UNTEMPLATED`
with a one-line reason. The decision is the point; the test only refuses to let
it be made by default.

SCOPE LIMIT: the scan is keyed on `PREFIXES`, so it forces a decision only
for a field that joins an already-scanned family. A pin bump that introduces
a wholly new family (say a `rabbitmq_confirm_*` or `publisher_*` block) lands
outside every prefix and is invisible here -- widen `PREFIXES` when the
library grows one.

SCOPE OVERLAP: test_drain_settings_template.py pins five of the `TEMPLATED`
rows per-row (chart default, values-key wiring, emitted-once), and
test_settings_configmap_template.py pins the rendered form of
`rest_max_request_bytes`. The presence assertions here deliberately re-cover
those rows rather than carving them out, because `TEMPLATED` has to be checked
as a whole list for the exhaustiveness test below to mean anything.
"""

import pytest

from app.config.settings import Settings
from notification_common.config.common_settings import CommonSettings
from tests.helm.conftest import render, requires_helm, single_document

SETTINGS_CONFIGMAP_TEMPLATE = "templates/settings-configmap.yaml"

# `Settings` subclasses `CommonSettings`, so its model_fields is a SUPERSET of
# the library's -- scanning the subclass therefore catches library field growth
# (the b.8xv failure mode) as well as anything this service declares itself. It
# is also the model the app actually instantiates, so it is the effective one.
# TestEffectiveSettingsModel pins the subclass relationship that this reasoning
# rests on.
SETTINGS_MODEL = Settings

# Read the env-var prefix off the model rather than hardcoding "OMNI_EVENTS_",
# so a prefix change in notification-common surfaces here instead of turning
# every presence assertion below into a comparison against a stale literal.
ENV_PREFIX = SETTINGS_MODEL.model_config["env_prefix"]

# The scan is scoped to the settings families this publisher is actually
# governed by: the non-durable channel pool it publishes through, the durable
# pool it does not (kept in scope so a new durable field still forces a
# decision), the SIGTERM drain sequence, and the REST surface -- whose
# `rest_max_request_bytes` is the same hazard in miniature, a bound that
# silently becomes every pod's setting if the chart never templates it.
# Settings outside these families (auth, telemetry, the gRPC port) are covered
# by their own chart surfaces and tests.
PREFIXES = (
    "rabbitmq_channel_",
    "rabbitmq_durable_channel_",
    "shutdown_",
    "rest_",
)

# Fields this chart templates into the settings ConfigMap. Not trusted as
# written: TestTemplatedFieldsAreRendered checks every name here against the
# default render.
TEMPLATED = [
    "rabbitmq_channel_pool_size",
    "rabbitmq_channel_pool_min_size",
    "rabbitmq_channel_client_limit",
    "rabbitmq_channel_overflow_enabled",
    "rabbitmq_channel_wait_ms",
    "shutdown_drain_window_seconds",
    "shutdown_drain_poll_interval_seconds",
    # Added by the 6.10.1 -> 6.12.0 bump, and reported by the exhaustiveness
    # test below before it was templated -- the guard doing exactly the job it
    # was written for. Templated because the library ships it at 0.0 (the
    # settle phase skipped), and this chart's Service is readiness-gated, so
    # the delay is the only cover the chart has for the de-routing lag.
    "shutdown_settle_delay_seconds",
    "rest_api_enabled",
    "rest_port",
    "rest_max_request_bytes",
]

# Fields left at the library default on purpose, each with its reason. Adding a
# name here is a deliberate decision that this publisher is not subject to the
# setting -- not a way to quiet the guard.
DELIBERATELY_UNTEMPLATED = [
    # Consumer-only: prefetch is a QoS limit on unacknowledged deliveries to a
    # consumer, and this service only publishes.
    "rabbitmq_channel_prefetch_count",
    # The whole durable pool: this publisher borrows only from the non-durable
    # pool, so no durable-pool knob reaches its publish path.
    "rabbitmq_durable_channel_pool_size",
    "rabbitmq_durable_channel_pool_min_size",
    "rabbitmq_durable_channel_prefetch_count",
    "rabbitmq_durable_channel_wait_ms",
    "rabbitmq_durable_channel_client_limit",
]


def _env_name(field: str) -> str:
    return f"{ENV_PREFIX}{field.upper()}"


def _publisher_relevant_fields() -> set[str]:
    """The settings-model fields in scope, read off the model, never hardcoded."""
    return {n for n in SETTINGS_MODEL.model_fields if n.startswith(PREFIXES)}


@pytest.fixture(scope="module")
def default_data() -> dict:
    """The settings ConfigMap data of a stock render. Read-only; shared."""
    out = render(show_only=SETTINGS_CONFIGMAP_TEMPLATE)
    return single_document(out, "ConfigMap")["data"]


class TestEffectiveSettingsModel:
    """`Settings` is the effective model because it extends the library's.

    Every other test here scans `Settings`; that only catches a new
    notification-common field because the subclass inherits it. If the service
    ever stopped extending `CommonSettings`, the scan would silently go blind to
    exactly the pin-bump growth this module exists to catch.
    """

    def test_settings_extends_common_settings(self):
        assert issubclass(SETTINGS_MODEL, CommonSettings)


class TestTemplatedFieldsAreRendered:
    """Every field claimed as templated really does render a ConfigMap row.

    This is what keeps `TEMPLATED` honest: without it the list is just an
    assertion about itself, and a field could be moved into it -- silencing the
    exhaustiveness test below -- without any chart row ever being added.
    """

    # Only the render-backed classes need the helm CLI. The exhaustiveness
    # and disjointness checks below are pure model reads, and gating them on
    # helm would silently skip the pin-bump guard in the CI test job, which
    # runs on a bare python image.
    pytestmark = requires_helm

    @pytest.mark.parametrize("field", TEMPLATED)
    def test_field_renders_its_env_var(self, default_data, field):
        assert _env_name(field) in default_data


class TestUntemplatedFieldsAreAbsent:
    """No field claimed as a deliberate omission renders a ConfigMap row.

    The mirror of the class above, keeping the classification honest in the
    other direction: a knob that gets templated later must move out of
    `DELIBERATELY_UNTEMPLATED` rather than sit there contradicting the chart.
    """

    pytestmark = requires_helm

    @pytest.mark.parametrize("field", DELIBERATELY_UNTEMPLATED)
    def test_field_renders_no_env_var(self, default_data, field):
        assert _env_name(field) not in default_data


class TestSettingsCoverageIsExhaustive:
    """Every publisher-relevant model field is classified one way or the other.

    This is the guard proper. A notification-common pin bump that adds a field
    under one of the scanned prefixes lands here as a red test naming the field,
    which forces the template-it-or-record-it decision that b.8xv was deployed
    without.
    """

    def test_classification_covers_the_model_exactly(self):
        relevant = _publisher_relevant_fields()
        classified = set(TEMPLATED) | set(DELIBERATELY_UNTEMPLATED)

        assert classified == relevant, (
            "unclassified settings fields (template each one -- values key, "
            "settings-configmap.yaml row, name in TEMPLATED -- or record it in "
            f"DELIBERATELY_UNTEMPLATED with a reason): {sorted(relevant - classified)}"
            "; classified names no longer on the settings model (drop them): "
            f"{sorted(classified - relevant)}"
        )


class TestSettingsCoverageListsAgree:
    """The two classification lists do not overlap.

    A name in both would make the exhaustiveness check above pass while leaving
    the field's actual status ambiguous.
    """

    def test_lists_are_disjoint(self):
        assert not set(TEMPLATED) & set(DELIBERATELY_UNTEMPLATED)
