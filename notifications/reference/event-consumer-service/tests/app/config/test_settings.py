# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import importlib

import pytest
from notification_common.scaler.scaler_settings import ScalerSettings
from pydantic import ValidationError

from app.config import settings
from tests.conftest import reset_env_vars


@pytest.fixture(autouse=True)
def reset_settings():
    """
    Put the settings back to normal after tests
    """
    reset_env_vars()
    yield
    reset_env_vars()


def test_sets_settings_from_env(monkeypatch):
    monkeypatch.setenv("OMNI_EVENTS_GRPC_PORT", "gp")
    monkeypatch.setenv("OMNI_EVENTS_PERMISSIONS_ENDPOINT", "pe")
    monkeypatch.setenv("OMNI_EVENTS_PERMISSIONS_TTL_SECONDS", "10")
    importlib.reload(settings)

    assert settings.settings.grpc_port == "gp"
    assert settings.settings.permissions_ttl_seconds == 10


def test_sets_storage_permissions_endpoints_from_yaml():
    assert settings.settings.storage_permissions_endpoints == ["ep_1", "ep_2"]


def test_sets_event_permissions_from_yaml():
    event_permissions = settings.settings.event_permissions
    assert event_permissions[0].event_type == "event-type-1"
    assert event_permissions[0].action == "read"

    assert event_permissions[1].event_type == "event-type-2"
    assert event_permissions[1].action == "write"


def test_buffered_ttl_seconds_adds_2_seconds_to_non_durable_queue_ttl():
    settings.settings.non_durable_queue_ttl_seconds = 10
    assert settings.settings.buffered_ttl_seconds == 12


class TestConsumerLivenessPollSeconds:
    """
    The idle-poll interval that bounds how long a stream can stay open
    after its AMQP consumer dies (issue b.9gj).

    It replaced ``consumer_timeout_ms``, which was passed as a
    ``basic.consume`` argument the broker silently ignores. The
    acknowledgement timeout it purported to set is now applied through
    the chart's queue-policy hook instead, so a settings field by that
    name must NOT come back -- ``test_consumer_timeout_ms_is_gone``
    pins that.
    """

    def test_has_default_poll_interval(self):
        assert settings.settings.consumer_liveness_poll_seconds == 15.0

    def test_sets_poll_interval_from_env(self, monkeypatch):
        monkeypatch.setenv("OMNI_EVENTS_CONSUMER_LIVENESS_POLL_SECONDS", "2.5")
        importlib.reload(settings)

        assert settings.settings.consumer_liveness_poll_seconds == 2.5

    def test_rejects_sub_second_poll_interval(self, monkeypatch):
        # Sub-second polling spins the consume loop for no gain: the
        # conditions it detects are broker-timescale events.
        monkeypatch.setenv("OMNI_EVENTS_CONSUMER_LIVENESS_POLL_SECONDS", "0.5")

        with pytest.raises(ValidationError, match="consumer_liveness_poll_seconds"):
            importlib.reload(settings)

    def test_accepts_the_smallest_permitted_poll_interval(self, monkeypatch):
        monkeypatch.setenv("OMNI_EVENTS_CONSUMER_LIVENESS_POLL_SECONDS", "1")
        importlib.reload(settings)

        assert settings.settings.consumer_liveness_poll_seconds == 1.0

    def test_consumer_timeout_ms_is_gone(self):
        # The removed field was inert: RabbitMQ accepts the consumer
        # acknowledgement timeout only as a queue argument, a policy key,
        # or in the server config -- never as a basic.consume argument.
        assert not hasattr(settings.settings, "consumer_timeout_ms")


def test_has_default_readiness_min_available_channels():
    """
    The readiness reserve keeps its NAME (and its OMNI_EVENTS_ env var)
    by deliberate choice while its UNIT changed to non-durable STREAM
    slots: an env var with no matching field is neither read nor
    rejected by pydantic-settings, so a rename would silently ignore a
    tuned deployment's configured value - do not "correct" the name to
    match the new unit. The default is now 80 stream slots, sized against
    the 800-stream packing product this release ships (100 pool_size x 8
    client_limit): a 10% reserve leaving 720 usable slots for the scaling
    signal.

    The DECLARED default is asserted rather than the resolved value: a
    developer's local (untracked) .env commonly sets this var, and
    load_dotenv() re-seeds it on every settings reload, so a resolved
    value assertion would be green in CI and red on that machine.
    """
    field = settings.Settings.model_fields["readiness_min_available_channels"]
    assert field.default == 80


def test_sets_readiness_min_available_channels_from_env(monkeypatch):
    monkeypatch.setenv("OMNI_EVENTS_READINESS_MIN_AVAILABLE_CHANNELS", "5")
    importlib.reload(settings)

    assert settings.settings.readiness_min_available_channels == 5


class TestReadinessMinAvailableChannelsUnit:
    """
    The reserve's unit is part of its contract: it is subtracted from the
    non-durable pool's STREAM capacity, and equals a channel count only
    at channel_client_limit=1. The field's description carries that unit
    so it is assertable here (and visible to operators), rather than
    living only in a source comment.
    """

    def test_description_names_the_stream_unit(self):
        field = settings.Settings.model_fields["readiness_min_available_channels"]
        assert "stream slots" in field.description

    def test_description_disclaims_the_channel_unit(self):
        field = settings.Settings.model_fields["readiness_min_available_channels"]
        assert "not channels" in field.description


class TestReadinessMinAvailableChannelsBound:
    """
    The reserve is bounded ge=0. Zero is meaningful ("unready only at
    real exhaustion"); a negative reserve would keep a pod ready past
    exhaustion. The bound must be exercised through the ENV var, not a
    constructor kwarg: settings_customise_sources drops init_settings,
    so Settings(readiness_min_available_channels=-1) silently yields the
    default instead of raising.
    """

    def test_rejects_negative_reserve(self, monkeypatch):
        monkeypatch.setenv("OMNI_EVENTS_READINESS_MIN_AVAILABLE_CHANNELS", "-1")

        with pytest.raises(ValidationError, match="readiness_min_available_channels"):
            importlib.reload(settings)

    def test_accepts_zero_reserve(self, monkeypatch):
        monkeypatch.setenv("OMNI_EVENTS_READINESS_MIN_AVAILABLE_CHANNELS", "0")
        importlib.reload(settings)

        assert settings.settings.readiness_min_available_channels == 0


def test_has_default_dead_letter_exchange_name():
    assert settings.settings.dead_letter_exchange_name == "events.dlx"


def test_has_default_dead_letter_queue_name():
    assert settings.settings.dead_letter_queue_name == "events.dlq"


def test_has_default_dead_letter_routing_key():
    assert settings.settings.dead_letter_routing_key == "dead-letter"


def test_has_default_dead_letter_queue_message_ttl_ms():
    assert settings.settings.dead_letter_queue_message_ttl_ms == 1209600000


def test_has_default_dead_letter_queue_max_length():
    assert settings.settings.dead_letter_queue_max_length == 1000000


def test_sets_dead_letter_exchange_name_from_env(monkeypatch):
    monkeypatch.setenv("OMNI_EVENTS_DEAD_LETTER_EXCHANGE_NAME", "custom.dlx")
    importlib.reload(settings)

    assert settings.settings.dead_letter_exchange_name == "custom.dlx"


def test_sets_dead_letter_queue_name_from_env(monkeypatch):
    monkeypatch.setenv("OMNI_EVENTS_DEAD_LETTER_QUEUE_NAME", "custom.dlq")
    importlib.reload(settings)

    assert settings.settings.dead_letter_queue_name == "custom.dlq"


def test_sets_dead_letter_routing_key_from_env(monkeypatch):
    monkeypatch.setenv("OMNI_EVENTS_DEAD_LETTER_ROUTING_KEY", "custom-dl")
    importlib.reload(settings)

    assert settings.settings.dead_letter_routing_key == "custom-dl"


def test_has_default_dead_letter_browse_prefetch_count():
    assert settings.settings.dead_letter_browse_prefetch_count == 10


def test_sets_dead_letter_browse_prefetch_count_from_env(monkeypatch):
    monkeypatch.setenv("OMNI_EVENTS_DEAD_LETTER_BROWSE_PREFETCH_COUNT", "3")
    importlib.reload(settings)

    assert settings.settings.dead_letter_browse_prefetch_count == 3


class TestDeadLetterBounds:
    """The DLQ bound fields forbid the silently-destructive zero value (ge=1)."""

    def test_rejects_zero_browse_prefetch_count(self, monkeypatch):
        # A prefetch of 0 means "unlimited" to set_qos, defeating the bound.
        monkeypatch.setenv("OMNI_EVENTS_DEAD_LETTER_BROWSE_PREFETCH_COUNT", "0")
        with pytest.raises(ValueError):
            importlib.reload(settings)

    def test_rejects_zero_queue_message_ttl_ms(self, monkeypatch):
        # A zero TTL would never expire (or silently discard) dead-letters.
        monkeypatch.setenv("OMNI_EVENTS_DEAD_LETTER_QUEUE_MESSAGE_TTL_MS", "0")
        with pytest.raises(ValueError):
            importlib.reload(settings)

    def test_rejects_zero_queue_max_length(self, monkeypatch):
        # A zero max-length would silently discard every dead-letter.
        monkeypatch.setenv("OMNI_EVENTS_DEAD_LETTER_QUEUE_MAX_LENGTH", "0")
        with pytest.raises(ValueError):
            importlib.reload(settings)


def test_has_default_broker_stream_memory_budget_mib():
    """
    The DECLARED default is asserted rather than the resolved value: a
    developer's local (untracked) .env can set this var, and load_dotenv()
    re-seeds it on every settings reload, so a resolved-value assertion
    would be green in CI and red on that machine.
    """
    field = settings.Settings.model_fields["broker_stream_memory_budget_mib"]
    assert field.default == 1024


def test_sets_broker_stream_memory_budget_mib_from_env(monkeypatch):
    monkeypatch.setenv("OMNI_EVENTS_BROKER_STREAM_MEMORY_BUDGET_MIB", "2048")
    importlib.reload(settings)

    assert settings.settings.broker_stream_memory_budget_mib == 2048


def test_has_default_scaler_max_replicas():
    """
    The divisor's default is pinned against the autoscaler's OWN field
    default, not a bare literal: the two settings deliberately share the
    name and the OMNI_EVENTS_SCALER_MAX_REPLICAS env var so one helm value
    feeds both, and ScalerSettings is the source of truth for that value.
    Asserting against the library field makes a library-side change
    surface here instead of the two defaults drifting apart silently.

    Importing notification_common.scaler.scaler_settings is a TEST-only
    import; app code must not import it (the prohibition is about the
    heavy import chain on the request path, which does not apply here).
    """
    field = settings.Settings.model_fields["scaler_max_replicas"]
    assert field.default == ScalerSettings.model_fields["scaler_max_replicas"].default


def test_sets_scaler_max_replicas_from_env(monkeypatch):
    monkeypatch.setenv("OMNI_EVENTS_SCALER_MAX_REPLICAS", "4")
    importlib.reload(settings)

    assert settings.settings.scaler_max_replicas == 4


def test_has_default_max_non_durable_streams_per_pod():
    """
    None is the "unset" sentinel and means "compute the cap from the
    budget and the configured prefetch". Pinning it guards the computed
    path: a future non-sentinel default would silently disable the
    computation for every deployment that never set this var.

    The DECLARED default is read from model_fields rather than from
    settings.settings, both for the .env reason above and because the
    autouse cap neutralizer in tests/app/conftest.py patches this field
    on the settings singleton.
    """
    field = settings.Settings.model_fields["max_non_durable_streams_per_pod"]
    assert field.default is None


def test_sets_max_non_durable_streams_per_pod_from_env(monkeypatch):
    monkeypatch.setenv("OMNI_EVENTS_MAX_NON_DURABLE_STREAMS_PER_POD", "250")
    importlib.reload(settings)

    assert settings.settings.max_non_durable_streams_per_pod == 250


class TestMemorySafeStreamCapUnits:
    """
    Each memory-safe cap setting states its unit in its description, so
    the unit is assertable here (and visible to operators in the helm
    values / README that Epic 3 generates) rather than living only in a
    source comment. The three units are deliberately different - MiB for
    the fleet-wide budget, pods for the divisor, streams for the per-pod
    cap - and confusing them silently mis-sizes the backstop.
    """

    def test_budget_description_names_the_mib_unit(self):
        field = settings.Settings.model_fields["broker_stream_memory_budget_mib"]
        assert "mebibytes (MiB)" in field.description

    def test_budget_description_says_it_is_fleet_wide(self):
        field = settings.Settings.model_fields["broker_stream_memory_budget_mib"]
        assert "Fleet-wide, not per pod" in field.description

    def test_divisor_description_names_the_pod_unit(self):
        field = settings.Settings.model_fields["scaler_max_replicas"]
        assert "in pods" in field.description

    def test_override_description_names_the_stream_unit(self):
        field = settings.Settings.model_fields["max_non_durable_streams_per_pod"]
        assert "streams per pod" in field.description


class TestMemorySafeStreamCapBounds:
    """
    All three cap settings are bounded ge=1. The bounds must be exercised
    through the ENV var, not a constructor kwarg: settings_customise_sources
    drops init_settings, so Settings(scaler_max_replicas=0) silently
    yields the default instead of raising.
    """

    def test_rejects_zero_budget(self, monkeypatch):
        # A zero budget computes a per-pod cap of zero, rejecting every stream.
        monkeypatch.setenv("OMNI_EVENTS_BROKER_STREAM_MEMORY_BUDGET_MIB", "0")

        with pytest.raises(ValidationError, match="broker_stream_memory_budget_mib"):
            importlib.reload(settings)

    def test_accepts_smallest_budget(self, monkeypatch):
        monkeypatch.setenv("OMNI_EVENTS_BROKER_STREAM_MEMORY_BUDGET_MIB", "1")
        importlib.reload(settings)

        assert settings.settings.broker_stream_memory_budget_mib == 1

    def test_rejects_zero_scaler_max_replicas(self, monkeypatch):
        # The divisor must never be zero: a zero divisor is a
        # ZeroDivisionError at startup or, worse, a swallowed fallback.
        monkeypatch.setenv("OMNI_EVENTS_SCALER_MAX_REPLICAS", "0")

        with pytest.raises(ValidationError, match="scaler_max_replicas"):
            importlib.reload(settings)

    def test_accepts_smallest_scaler_max_replicas(self, monkeypatch):
        # A single-pod fleet is a legitimate deployment.
        monkeypatch.setenv("OMNI_EVENTS_SCALER_MAX_REPLICAS", "1")
        importlib.reload(settings)

        assert settings.settings.scaler_max_replicas == 1

    def test_rejects_zero_max_non_durable_streams_per_pod(self, monkeypatch):
        # None, not 0, is the "unset" sentinel: 0 must stay invalid so it
        # cannot become a second sentinel, and because a real cap of 0
        # would reject every stream.
        monkeypatch.setenv("OMNI_EVENTS_MAX_NON_DURABLE_STREAMS_PER_POD", "0")

        with pytest.raises(ValidationError, match="max_non_durable_streams_per_pod"):
            importlib.reload(settings)

    def test_accepts_smallest_max_non_durable_streams_per_pod(self, monkeypatch):
        monkeypatch.setenv("OMNI_EVENTS_MAX_NON_DURABLE_STREAMS_PER_POD", "1")
        importlib.reload(settings)

        assert settings.settings.max_non_durable_streams_per_pod == 1


class TestReRoutableShedThreshold:
    """
    The SR-3.7 shed threshold, given the same settings-level pins its three
    sibling capacity knobs above already carry: declared default, env
    parsing, unit in the description, and the ``ge=1`` bound.

    It is a separate class rather than four more rows in the cap classes
    because it is a different tier: the cap is the HARD bound that refuses
    every class, while this one refuses re-routable streams only, inside
    the overflow band. The two are read together
    (``re_routable_shed_threshold()`` clamps this value into the band the
    cap tops out), so they must not silently converge into one setting.

    The ``ge=1`` bound is load-bearing beyond the field itself. The helm
    settings configmap renders any non-null value verbatim -- including 0 --
    precisely so pydantic rejects it and the pod crashes loudly at startup
    rather than degrading into the computed midpoint (see
    ``tests/helm/test_settings_configmap_template.py``). That template's
    contract is only true while this bound holds, and nothing else asserts
    it.
    """

    def test_has_default_max_re_routable_streams_per_pod(self):
        """
        None is the "unset" sentinel and means "sit at the midpoint of the
        band". Pinning it guards the midpoint path: a non-sentinel default
        would silently fix the shed line for every deployment that never
        set this var, and the clamp would then move it with the band
        instead of the operator's intent.

        The DECLARED default is read from ``model_fields`` rather than
        from ``settings.settings``, for the same .env reason the cap's
        default test records.
        """
        field = settings.Settings.model_fields["max_re_routable_streams_per_pod"]
        assert field.default is None

    def test_sets_max_re_routable_streams_per_pod_from_env(self, monkeypatch):
        monkeypatch.setenv("OMNI_EVENTS_MAX_RE_ROUTABLE_STREAMS_PER_POD", "250")
        importlib.reload(settings)

        assert settings.settings.max_re_routable_streams_per_pod == 250

    def test_description_names_the_stream_unit(self):
        # The same unit as the cap, deliberately: the two knobs read as one
        # scale, and an operator comparing them must not have to convert.
        field = settings.Settings.model_fields["max_re_routable_streams_per_pod"]
        assert "Per-pod stream count" in field.description

    def test_description_names_the_unset_sentinel_meaning(self):
        field = settings.Settings.model_fields["max_re_routable_streams_per_pod"]
        assert "Unset means the midpoint" in field.description

    def test_description_names_the_width_1_band_caveat(self):
        """
        The floor-division midpoint degenerates on a width-1 band: soft +
        (hard - soft) // 2 = soft + 0 = soft, so the unset shed line sits at
        the soft threshold and re-routable traffic sheds at the packing
        product. That operationally load-bearing caveat -- the same one the
        capacity_guard docstring carries -- must be visible in the operator
        description, not just the source comment.
        """
        field = settings.Settings.model_fields["max_re_routable_streams_per_pod"]
        assert "width-1 band the midpoint equals the soft threshold" in (
            field.description
        )

    def test_rejects_zero_max_re_routable_streams_per_pod(self, monkeypatch):
        # None, not 0, is the "unset" sentinel: 0 must stay invalid so it
        # cannot become a second sentinel. It is also the value the helm
        # template deliberately renders through rather than suppressing.
        monkeypatch.setenv("OMNI_EVENTS_MAX_RE_ROUTABLE_STREAMS_PER_POD", "0")

        with pytest.raises(ValidationError, match="max_re_routable_streams_per_pod"):
            importlib.reload(settings)

    def test_accepts_smallest_max_re_routable_streams_per_pod(self, monkeypatch):
        # 1 is accepted here and clamped UP to the soft threshold at
        # admission; the clamp is the guard's business, not this field's.
        monkeypatch.setenv("OMNI_EVENTS_MAX_RE_ROUTABLE_STREAMS_PER_POD", "1")
        importlib.reload(settings)

        assert settings.settings.max_re_routable_streams_per_pod == 1

    def test_field_name_exists(self):
        """
        The NAME is a cross-Task contract, on exactly the grounds
        ``TestMemorySafeStreamCapNameContract`` records: helm templates
        ``OMNI_EVENTS_MAX_RE_ROUTABLE_STREAMS_PER_POD`` into the settings
        configmap, and the env source iterates ``model_fields`` only -- so
        a rename leaves an operator's tuned shed line SILENTLY ignored
        while the computed midpoint takes effect.
        """
        assert "max_re_routable_streams_per_pod" in settings.Settings.model_fields


class TestMemorySafeStreamCapNameContract:
    """
    These field NAMES are a cross-Epic contract: helm templates them into
    settings-configmap.yaml / values.yaml and the README documents them,
    so the names an operator types are these names.

    A rename is not a safe refactor. The pydantic-settings env source
    iterates settings_cls.model_fields only, so an OMNI_EVENTS_-prefixed
    variable with no matching field is neither read nor rejected -
    extra='forbid' never sees it. Rename one of these and an operator's
    tuned cap is SILENTLY IGNORED while the computed default takes
    effect. Changing a name here must therefore be a deliberate migration
    (helm keys, README, and a documented upgrade note), not a rename.
    """

    def test_memory_safe_cap_field_names_exist(self):
        expected = {
            "broker_stream_memory_budget_mib",
            "scaler_max_replicas",
            "max_non_durable_streams_per_pod",
        }
        assert expected <= set(settings.Settings.model_fields)

    def test_divisor_field_name_matches_the_autoscalers(self):
        """
        The divisor is the same NAME on both sides on purpose, so helm can
        template channelPoolAutoscaling.maxReplicas into one env var that
        both the service and the autoscaler read. If either side renames
        its field, the shared var stops reaching that side silently.
        """
        assert "scaler_max_replicas" in ScalerSettings.model_fields

    def test_divisor_shares_the_autoscalers_env_prefix(self):
        """
        Same name only means the same env var while the two settings
        classes share an env_prefix; a prefix change on either side splits
        OMNI_EVENTS_SCALER_MAX_REPLICAS into two variables and an operator
        would have to set the number twice to keep them aligned.
        """
        assert (
            settings.Settings.model_config["env_prefix"]
            == ScalerSettings.model_config["env_prefix"]
        )


class TestRetiredSharedChannelSetting:
    """
    ``max_streams_per_shared_channel`` is retired with the
    ConnectionChannelManager (SDD SR-4.2). It was solely the CCM's
    per-channel sharing limit; packing density is now the pool's
    ``channel_client_limit``, so the field has no successor here.

    The absence is PINNED, not just deleted, for the same
    pydantic-settings reason ``TestMemorySafeStreamCapNameContract``
    records in the other direction: the env source iterates
    ``model_fields`` only, so an ``OMNI_EVENTS_``-prefixed variable with
    no matching field is silently ignored -- ``extra='forbid'`` never
    sees it. A future rebase that re-introduces the field would draw no
    signal, so this test is the signal.
    """

    def test_field_is_absent_from_the_schema(self):
        assert "max_streams_per_shared_channel" not in settings.Settings.model_fields


class TestRedisEnabled:
    def test_defaults_to_false(self):
        assert settings.settings.redis_enabled is False

    def test_sets_redis_enabled_from_env(self, monkeypatch):
        monkeypatch.setenv("OMNI_EVENTS_REDIS_ENABLED", "True")
        monkeypatch.setenv("OMNI_EVENTS_REDIS_URI", "test-uri")
        importlib.reload(settings)

        assert settings.settings.redis_enabled is True


class TestValidatePermissionsSettings:
    """
    Covers the permission-service gate of the two-gate
    validate_auth_settings contract. The gate is now independent:
    permissions_enabled=true requires permissions_endpoint specifically
    (storage endpoints no longer satisfy it), and a disabled gate leaves
    its endpoint inert. The removed OR-semantics ("permissions_enabled is
    true but neither ... nor storage_permissions_endpoints") is gone.

    Tests that need no YAML-provided endpoints must clear the YAML
    file path, because YAML has higher priority than env vars in the
    settings source order.
    """

    def test_raises_when_permissions_enabled_but_no_endpoint(self, monkeypatch):
        # Storage endpoints (YAML) no longer satisfy this gate, so clearing
        # only permissions_endpoint (and the YAML path) must still raise.
        monkeypatch.setenv("OMNI_EVENTS_PERMISSIONS_ENABLED", "true")
        monkeypatch.setenv("OMNI_EVENTS_PERMISSIONS_ENDPOINT", "")
        monkeypatch.setenv("OMNI_EVENTS_PERMISSIONS_YAML_FILE_PATH", "")

        with pytest.raises(
            Exception,
            match="permissions_enabled is true but permissions_endpoint",
        ):
            importlib.reload(settings)

    def test_passes_when_permissions_enabled_with_permissions_endpoint(
        self, monkeypatch
    ):
        # One-gate-on/other-off: the permission-service gate is satisfied by
        # permissions_endpoint alone and does NOT require storage endpoints.
        # Clear the YAML path so no storage endpoints leak in.
        monkeypatch.setenv("OMNI_EVENTS_PERMISSIONS_ENABLED", "true")
        monkeypatch.setenv(
            "OMNI_EVENTS_PERMISSIONS_ENDPOINT",
            "https://permissions.example.com",
        )
        monkeypatch.setenv("OMNI_EVENTS_PERMISSIONS_YAML_FILE_PATH", "")
        importlib.reload(settings)

        assert settings.settings.permissions_enabled is True
        assert settings.settings.storage_permissions_enabled is False
        assert settings.settings.storage_permissions_endpoints == []

    def test_passes_when_permissions_disabled_with_endpoints_set(self, monkeypatch):
        # A disabled permission-service gate leaves its endpoint inert: no
        # ServerError even with permissions_endpoint (env) and
        # storage_permissions_endpoints (YAML fixture) both populated.
        monkeypatch.setenv("OMNI_EVENTS_PERMISSIONS_ENABLED", "false")
        monkeypatch.setenv(
            "OMNI_EVENTS_PERMISSIONS_ENDPOINT",
            "https://permissions.example.com",
        )
        importlib.reload(settings)

        assert settings.settings.permissions_enabled is False
        assert settings.settings.permissions_endpoint == (
            "https://permissions.example.com"
        )
        assert settings.settings.storage_permissions_endpoints == ["ep_1", "ep_2"]


class TestValidateStoragePermissionsSettings:
    """
    Covers the storage gate of the two-gate validate_auth_settings
    contract, symmetric to the permission-service gate:
    storage_permissions_enabled=true requires storage_permissions_endpoints
    specifically (permissions_endpoint no longer satisfies it), and a
    disabled gate leaves its endpoints inert.

    storage_permissions_endpoints is sourced from the YAML fixture
    (test_event_permissions.yml), so tests that need zero storage
    endpoints must clear the YAML file path.
    """

    def test_storage_permissions_enabled_defaults_to_false(self):
        assert settings.settings.storage_permissions_enabled is False

    def test_raises_when_storage_enabled_but_no_endpoints(self, monkeypatch):
        # Clear the YAML path (the storage endpoints' source); reset_env_vars
        # already deletes the env override, so the storage gate has zero
        # endpoints to validate. (An empty-string env value would fail JSON
        # parsing for the list[str] field before validation, so delenv it.)
        monkeypatch.setenv("OMNI_EVENTS_STORAGE_PERMISSIONS_ENABLED", "true")
        monkeypatch.delenv("OMNI_EVENTS_STORAGE_PERMISSIONS_ENDPOINTS", raising=False)
        monkeypatch.setenv("OMNI_EVENTS_PERMISSIONS_YAML_FILE_PATH", "")

        with pytest.raises(
            Exception,
            match=(
                "storage_permissions_enabled is true but "
                "storage_permissions_endpoints"
            ),
        ):
            importlib.reload(settings)

    def test_passes_when_storage_enabled_with_storage_endpoints(self, monkeypatch):
        # One-gate-on/other-off: the storage gate is satisfied by the
        # YAML-provided storage endpoints alone and does NOT require
        # permissions_endpoint. Clear permissions_endpoint to prove that.
        monkeypatch.setenv("OMNI_EVENTS_STORAGE_PERMISSIONS_ENABLED", "true")
        monkeypatch.setenv("OMNI_EVENTS_PERMISSIONS_ENDPOINT", "")
        importlib.reload(settings)

        assert settings.settings.storage_permissions_enabled is True
        assert settings.settings.permissions_enabled is False
        assert settings.settings.storage_permissions_endpoints == ["ep_1", "ep_2"]

    def test_passes_when_storage_disabled_with_endpoints_set(self, monkeypatch):
        # A disabled storage gate leaves its endpoints inert: no ServerError
        # even with the YAML-provided storage endpoints populated.
        monkeypatch.setenv("OMNI_EVENTS_STORAGE_PERMISSIONS_ENABLED", "false")
        importlib.reload(settings)

        assert settings.settings.storage_permissions_enabled is False
        assert settings.settings.storage_permissions_endpoints == ["ep_1", "ep_2"]


def test_default_deploy_posture_loads_without_crash_loop(monkeypatch):
    # Crash-loop regression guard: the DEFAULT-deploy posture is BOTH gates
    # off (permissions_enabled and storage_permissions_enabled unset/false)
    # with principal_source at its default and no endpoints configured.
    # Clearing the YAML path and the env overrides that reset_env_vars seeds
    # reproduces that posture; settings must reload cleanly with no startup
    # ServerError and no endpoints required.
    monkeypatch.delenv("OMNI_EVENTS_PERMISSIONS_ENABLED", raising=False)
    monkeypatch.delenv("OMNI_EVENTS_STORAGE_PERMISSIONS_ENABLED", raising=False)
    monkeypatch.delenv("OMNI_EVENTS_PRINCIPAL_SOURCE", raising=False)
    monkeypatch.setenv("OMNI_EVENTS_PERMISSIONS_ENDPOINT", "")
    monkeypatch.delenv("OMNI_EVENTS_STORAGE_PERMISSIONS_ENDPOINTS", raising=False)
    monkeypatch.setenv("OMNI_EVENTS_PERMISSIONS_YAML_FILE_PATH", "")

    importlib.reload(settings)

    assert settings.settings.permissions_enabled is False
    assert settings.settings.storage_permissions_enabled is False
    assert settings.settings.storage_permissions_endpoints == []


class TestValidateRedisSettings:
    def test_raises_error_when_redis_enabled_but_uri_empty(self, monkeypatch):
        monkeypatch.setenv("OMNI_EVENTS_REDIS_ENABLED", "True")
        monkeypatch.setenv("OMNI_EVENTS_REDIS_URI", "")

        with pytest.raises(Exception, match="redis_uri must be provided"):
            importlib.reload(settings)

    def test_does_not_raise_when_redis_enabled_and_uri_provided(self, monkeypatch):
        monkeypatch.setenv("OMNI_EVENTS_REDIS_ENABLED", "True")
        monkeypatch.setenv("OMNI_EVENTS_REDIS_URI", "test-uri")
        importlib.reload(settings)

        assert settings.settings.redis_enabled is True

    def test_does_not_raise_when_redis_disabled_and_uri_empty(self, monkeypatch):
        monkeypatch.setenv("OMNI_EVENTS_REDIS_ENABLED", "False")
        monkeypatch.setenv("OMNI_EVENTS_REDIS_URI", "")
        importlib.reload(settings)

        assert settings.settings.redis_enabled is False
