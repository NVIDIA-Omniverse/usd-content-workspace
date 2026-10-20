# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

# Regression pin for OMNI_EVENTS_SCALER_MAX_REPLICAS on the SERVICE side, at
# templates/settings-configmap.yaml:65-73. That number is the divisor the
# service pods use to apportion the fleet-wide broker memory budget into a
# per-pod non-durable stream cap (stream_memory_cap.py:140), and the service
# Deployment picks it up through envFrom.configMapRef
# (templates/deployment.yaml:55-57). Two failure modes, both silent:
#
# 1. The row is never emitted. pydantic-settings raises nothing for a
#    variable that is simply unset, so Settings.scaler_max_replicas quietly
#    falls back to its code default of 10 (app/config/settings.py:136-147).
#    A fleet deployed with a different ceiling then divides the budget by a
#    number nobody chose, and every pod admits the wrong number of streams.
#    Nothing errors -- the cap is just wrong.
# 2. The branch reads the wrong values key. The ceiling is owned by the
#    autoscaler when channelPoolAutoscaling.enabled is true and by
#    replicaCount when it is false, so the row is a conditional. Swap the two
#    arms and the render still succeeds and still produces a plausible
#    integer; only the apportioning is wrong, and only under load.
#
# The point of the whole item is that ONE helm value states the fleet ceiling
# once and feeds both consumers -- the autoscaler's own ceiling and the
# service pods' divisor. app/config/settings.py:126-135 states that contract
# in prose; the cross-surface tests below are what actually hold it.
#
# Renders are scoped with --show-only on purpose. A whole-chart render also
# contains the autoscaler Deployment, whose inline env carries a
# `- name: OMNI_EVENTS_SCALER_MAX_REPLICAS` line: any unscoped raw-text check
# would match that row too and pass for the wrong reason.

import shutil
import subprocess
from pathlib import Path

import pytest
import yaml

from tests.helm.conftest import disable_policy_hook, select_tokenless_auth

CHART_PATH = Path(__file__).resolve().parents[2] / "helm" / "event-consumer-service"

SETTINGS_CONFIGMAP_TEMPLATE = "templates/settings-configmap.yaml"
AUTOSCALER_TEMPLATE = "templates/autoscaler-deployment.yaml"
DEPLOYMENT_TEMPLATE = "templates/deployment.yaml"

ENV_MAX_REPLICAS = "OMNI_EVENTS_SCALER_MAX_REPLICAS"

# Chart defaults: channelPoolAutoscaling.enabled/maxReplicas at
# values.yaml:595-612, replicaCount at values.yaml:8-19. These are the CHART's
# numbers, not the code defaults -- the chart value is what actually ships to
# a cluster, so it is what these tests pin.
DEFAULT_MAX_REPLICAS = "10"
DEFAULT_REPLICA_COUNT = "1"

# Both differ from every default in play (1, 3, 10, 20, 30, 75, 100), so a
# template that ignored its values key could not render either by accident,
# and they differ from each other so a branch-selection mistake is visible.
OVERRIDE_MAX_REPLICAS = "25"
OVERRIDE_REPLICA_COUNT = "7"

AUTOSCALING_DISABLED = "channelPoolAutoscaling.enabled=false"
SET_MAX_REPLICAS = f"channelPoolAutoscaling.maxReplicas={OVERRIDE_MAX_REPLICAS}"
SET_REPLICA_COUNT = f"replicaCount={OVERRIDE_REPLICA_COUNT}"

pytestmark = pytest.mark.skipif(
    shutil.which("helm") is None,
    reason="helm CLI not available",
)


def _render(*extra_set: str, show_only: str | None = None) -> str:
    args = ["helm", "template", "ec", str(CHART_PATH), "--namespace", "storage-api"]
    if show_only is not None:
        args.extend(["--show-only", show_only])
    # Before the caller's own overrides: helm takes the last --set, so an
    # explicit policyHook setting in extra_set still wins.
    disable_policy_hook(args)
    select_tokenless_auth(args)
    for kv in extra_set:
        args.extend(["--set", kv])
    result = subprocess.run(args, capture_output=True, text=True)
    assert result.returncode == 0, (
        f"helm template failed:\nstdout:\n{result.stdout}\n" f"stderr:\n{result.stderr}"
    )
    return result.stdout


def _settings_configmap_data(*extra_set: str) -> dict:
    """Return the settings ConfigMap's `data` mapping from a scoped render.

    Parsed structurally rather than scanned as a substring: a scan for
    "OMNI_EVENTS_SCALER_MAX_REPLICAS:" cannot tell this key apart from a
    future OMNI_EVENTS_SCALER_MAX_REPLICAS_FLOOR, cannot tell a data row from
    the autoscaler's `- name:` env entry, and cannot see the YAML type of the
    rendered value.
    """
    out = _render(*extra_set, show_only=SETTINGS_CONFIGMAP_TEMPLATE)
    docs = [d for d in yaml.safe_load_all(out) if d]
    assert len(docs) == 1, f"expected exactly one document, got {len(docs)}:\n{out}"
    assert docs[0]["kind"] == "ConfigMap", f"unexpected kind:\n{out}"
    return docs[0]["data"]


def _autoscaler_env(*extra_set: str) -> dict[str, str]:
    """Flatten the autoscaler container's env list into {name: value}.

    Only valid on the enabled branch: the autoscaler Deployment is gated by
    `{{- if .Values.channelPoolAutoscaling.enabled }}`
    (autoscaler-deployment.yaml:4), so with autoscaling disabled this render
    still exits 0 but emits zero documents and the single-document assertion
    below would fire. Entries sourced with `valueFrom` carry no `value` key,
    so they are skipped rather than raising KeyError.
    """
    out = _render(*extra_set, show_only=AUTOSCALER_TEMPLATE)
    docs = [d for d in yaml.safe_load_all(out) if d]
    assert len(docs) == 1, f"expected exactly one document, got {len(docs)}:\n{out}"
    assert docs[0]["kind"] == "Deployment", f"unexpected kind:\n{out}"

    container = docs[0]["spec"]["template"]["spec"]["containers"][0]
    return {e["name"]: e["value"] for e in container["env"] if "value" in e}


def _service_deployment(*extra_set: str) -> dict:
    out = _render(*extra_set, show_only=DEPLOYMENT_TEMPLATE)
    docs = [d for d in yaml.safe_load_all(out) if d]
    assert len(docs) == 1, f"expected exactly one document, got {len(docs)}:\n{out}"
    assert docs[0]["kind"] == "Deployment", f"unexpected kind:\n{out}"
    return docs[0]


@pytest.fixture(scope="module")
def default_data() -> dict:
    """The settings ConfigMap data of a stock render. Read-only; shared."""
    return _settings_configmap_data()


class TestDivisorOnTheEnabledBranch:
    """With autoscaling on, the divisor is the autoscaler's own ceiling.

    Absence is asserted apart from value because absence IS the primary bug:
    an unset env var produces no error, just the code default of 10, which
    happens to equal today's chart default and would therefore hide the
    regression on a default render forever.
    """

    def test_key_is_present(self, default_data):
        assert ENV_MAX_REPLICAS in default_data

    def test_renders_the_chart_default_max_replicas(self, default_data):
        assert default_data[ENV_MAX_REPLICAS] == DEFAULT_MAX_REPLICAS

    def test_follows_the_max_replicas_values_key(self):
        # Proves the row reads channelPoolAutoscaling.maxReplicas rather than
        # a literal that merely matches the default.
        data = _settings_configmap_data(SET_MAX_REPLICAS)

        assert data[ENV_MAX_REPLICAS] == OVERRIDE_MAX_REPLICAS

    def test_ignores_replica_count_when_both_are_set(self):
        # Branch-selection proof: with the two source keys set to distinct
        # values, only a correct enabled arm can produce maxReplicas.
        data = _settings_configmap_data(SET_MAX_REPLICAS, SET_REPLICA_COUNT)

        assert data[ENV_MAX_REPLICAS] == OVERRIDE_MAX_REPLICAS


class TestDivisorOnTheDisabledBranch:
    """With autoscaling off, replicaCount IS the fleet ceiling.

    There is no autoscaler pod to own the ceiling in this mode, so the
    Deployment's static replica count is the only honest divisor.
    """

    def test_key_is_present(self):
        data = _settings_configmap_data(AUTOSCALING_DISABLED)

        assert ENV_MAX_REPLICAS in data

    def test_renders_the_chart_default_replica_count(self):
        data = _settings_configmap_data(AUTOSCALING_DISABLED)

        assert data[ENV_MAX_REPLICAS] == DEFAULT_REPLICA_COUNT

    def test_follows_the_replica_count_values_key(self):
        data = _settings_configmap_data(AUTOSCALING_DISABLED, SET_REPLICA_COUNT)

        assert data[ENV_MAX_REPLICAS] == OVERRIDE_REPLICA_COUNT

    def test_ignores_max_replicas_when_both_are_set(self):
        data = _settings_configmap_data(
            AUTOSCALING_DISABLED,
            SET_MAX_REPLICAS,
            SET_REPLICA_COUNT,
        )

        assert data[ENV_MAX_REPLICAS] == OVERRIDE_REPLICA_COUNT


class TestDivisorAgreesWithTheAutoscalerCeiling:
    """One helm value must reach both consumers with the same number.

    The autoscaler pod reads its ceiling from an inline env entry on its own
    Deployment; the service pods read the divisor from the settings ConfigMap.
    If those two ever drift, the fleet can scale past the replica count the
    per-pod cap was apportioned for and the aggregate admission overshoots the
    broker memory budget -- the exact overshoot this cap exists to prevent.
    Enabled branch only: the autoscaler Deployment is not rendered at all when
    autoscaling is disabled.
    """

    def test_agree_in_the_default_render(self, default_data):
        assert default_data[ENV_MAX_REPLICAS] == _autoscaler_env()[ENV_MAX_REPLICAS]

    def test_agree_under_an_overridden_ceiling(self):
        data = _settings_configmap_data(SET_MAX_REPLICAS)
        env = _autoscaler_env(SET_MAX_REPLICAS)

        assert data[ENV_MAX_REPLICAS] == env[ENV_MAX_REPLICAS]


class TestDivisorAgreesWithTheStaticReplicaCount:
    """Disabled branch: the divisor must match the pods that exist.

    With autoscaling off the service Deployment templates spec.replicas
    (templates/deployment.yaml:12-14). Apportioning the fleet budget across a
    different number than the number of pods running either wastes budget or
    oversubscribes the broker.
    """

    def test_agree_in_the_default_render(self):
        data = _settings_configmap_data(AUTOSCALING_DISABLED)
        doc = _service_deployment(AUTOSCALING_DISABLED)

        assert data[ENV_MAX_REPLICAS] == str(doc["spec"]["replicas"])

    def test_agree_under_an_overridden_replica_count(self):
        data = _settings_configmap_data(AUTOSCALING_DISABLED, SET_REPLICA_COUNT)
        doc = _service_deployment(AUTOSCALING_DISABLED, SET_REPLICA_COUNT)

        assert data[ENV_MAX_REPLICAS] == str(doc["spec"]["replicas"])


class TestChangingTheCeilingRollsTheServicePods:
    """Retuning the fleet ceiling must restart the pods that read the divisor.

    Before this row existed, channelPoolAutoscaling.maxReplicas touched only
    the autoscaler Deployment, so an operator retuning it restarted one pod.
    Now it also changes the settings ConfigMap, and the service Deployment's
    checksum/config annotation (templates/deployment.yaml:22) hashes that
    ConfigMap -- which is what makes the new value actually reach running
    pods. Without a rolling restart the ConfigMap would update while every
    live pod kept apportioning the budget by the OLD divisor, silently, until
    something unrelated happened to restart them. values.yaml:605-611 states
    this consequence in prose; this test is what holds it.
    """

    @staticmethod
    def _config_checksum(*extra_set: str) -> str:
        doc = _service_deployment(*extra_set)
        annotations = doc["spec"]["template"]["metadata"]["annotations"]
        return annotations["checksum/config"]

    def test_ceiling_change_changes_the_config_checksum(self):
        assert self._config_checksum() != self._config_checksum(SET_MAX_REPLICAS)

    def test_disabled_branch_replica_count_change_changes_the_checksum(self):
        # The disabled branch feeds the divisor from replicaCount, so a
        # replicaCount change must roll the pods there for the same reason.
        before = self._config_checksum(AUTOSCALING_DISABLED)
        after = self._config_checksum(AUTOSCALING_DISABLED, SET_REPLICA_COUNT)

        assert before != after


class TestDivisorRendersAsAString:
    """A ConfigMap `data` value must be a string.

    If the row lands unquoted, helm template still emits it happily and YAML
    parses the number as an int; the API server rejects the manifest only at
    apply time, in the cluster, which is far too late to be useful. Same
    rationale as test_autoscaler_tuning_template.py:193-205.
    """

    def test_enabled_branch_value_is_a_string(self, default_data):
        assert isinstance(default_data[ENV_MAX_REPLICAS], str)

    def test_disabled_branch_value_is_a_string(self):
        data = _settings_configmap_data(AUTOSCALING_DISABLED)

        assert isinstance(data[ENV_MAX_REPLICAS], str)


class TestDivisorIsEmittedExactlyOnce:
    """The key must never be emitted twice in the ConfigMap.

    yaml.safe_load_all silently collapses duplicate mapping keys to the last
    occurrence, so every structural test above would keep passing if a second
    row landed -- while the effective value became whichever row happened to
    come last. Only a raw-text count can see that. The render is scoped to the
    ConfigMap template, so the autoscaler Deployment's `- name:` env row is
    not in scope, and the comment above the row is deliberately worded without
    the env-var name so a bare-name match counts data rows only.
    """

    @staticmethod
    def _occurrences(*extra_set: str) -> int:
        out = _render(*extra_set, show_only=SETTINGS_CONFIGMAP_TEMPLATE)
        return sum(1 for line in out.splitlines() if ENV_MAX_REPLICAS in line)

    def test_enabled_branch_emits_one_line(self):
        assert self._occurrences() == 1

    def test_disabled_branch_emits_one_line(self):
        assert self._occurrences(AUTOSCALING_DISABLED) == 1
