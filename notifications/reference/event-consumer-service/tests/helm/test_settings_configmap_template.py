# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import json
import os
import shutil
import subprocess
from pathlib import Path

import pytest
import yaml
from notification_common.config import auth_settings as auth_settings_module
from notification_common.config.auth_settings import AuthSettings
from notification_common.config.common_settings import CommonSettings

from app.config.settings import Settings
from tests.helm.conftest import disable_policy_hook, select_tokenless_auth

CHART_PATH = Path(__file__).resolve().parents[2] / "helm" / "event-consumer-service"

pytestmark = pytest.mark.skipif(
    shutil.which("helm") is None,
    reason="helm CLI not available",
)

ENV_PRINCIPAL_SOURCE = "OMNI_EVENTS_PRINCIPAL_SOURCE"
ENV_ALLOW_UNVERIFIED_JWT = "OMNI_EVENTS_ALLOW_UNVERIFIED_JWT"
ENV_OPENID_CONFIGURATION_URI = "OMNI_EVENTS_OPENID_CONFIGURATION_URI"
ENV_JWT_AUDIENCES = "OMNI_EVENTS_JWT_AUDIENCES"
ENV_PRINCIPAL_ID_CLAIM = "OMNI_EVENTS_PRINCIPAL_ID_CLAIM"
ENV_CORS_ALLOW_ORIGINS = "OMNI_EVENTS_CORS_ALLOW_ORIGINS"
ENV_PERMISSIONS_ENABLED = "OMNI_EVENTS_PERMISSIONS_ENABLED"

OPENID_URI = "https://login.example.com/tenant/v2.0/.well-known/openid-configuration"
CLIENT_ID = "the-client-id"

# The OpenID configuration that makes the chart select jwt when principalSource
# is left empty: a discovery URI and a "default" client registration.
OPENID_CONFIGURED = (
    f"openId.openIdConfigurationUri={OPENID_URI}",
    "openId.clientRegistrations[0].name=default",
    f"openId.clientRegistrations[0].clientId={CLIENT_ID}",
)

# The auth values of the dev and performance deployments: their shared OpenID
# overlay adds the Envoy auth extension to OPENID_CONFIGURED, the service
# values enable the HTTPProxy and both permission gates, and neither sets
# principalSource.
DEPLOYMENT_AUTH_VALUES = OPENID_CONFIGURED + (
    "openId.enabled=true",
    "openId.principalIdClaim=oid",
    "httpProxy.enabled=true",
    "httpProxy.authExtension.enabled=true",
    "permissions.enabled=true",
    "permissions.storage_permissions.enabled=true",
)

# The opening of each render-failure message, enough to tell them apart.
NO_OPENID_CONFIGURATION = "principalSource is unset and OpenID is not fully configured"
UNVERIFIED_NEEDS_OPT_IN = "principalSource jwtUnverified trusts any token"
OPT_IN_NEEDS_UNVERIFIED = "allowUnverifiedJwt applies only to principalSource"
AUDIENCE_KIND = "openId.additionalJwtAudience must be a YAML list"


def _render_result(*extra_set: str) -> subprocess.CompletedProcess:
    """Render with the tokenless baseline, for the rows unrelated to auth."""
    args = ["helm", "template", "ec", str(CHART_PATH), "--namespace", "storage-api"]
    # Before the caller's own overrides: helm takes the last --set, so an
    # explicit policyHook or principalSource setting in extra_set still wins.
    disable_policy_hook(args)
    select_tokenless_auth(args)
    for kv in extra_set:
        args.extend(["--set", kv])
    return subprocess.run(args, capture_output=True, text=True)


def _auth_render_result(
    *extra_set: str, set_string: tuple[str, ...] = ()
) -> subprocess.CompletedProcess:
    """Render with no auth baseline, so the chart's own auth defaults apply.

    `set_string` entries go through --set-string, which renders a value the
    way a quoted YAML scalar would.
    """
    args = ["helm", "template", "ec", str(CHART_PATH), "--namespace", "storage-api"]
    disable_policy_hook(args)
    for kv in extra_set:
        args.extend(["--set", kv])
    for kv in set_string:
        args.extend(["--set-string", kv])
    return subprocess.run(args, capture_output=True, text=True)


def _stdout(result: subprocess.CompletedProcess) -> str:
    assert result.returncode == 0, (
        f"helm template failed:\nstdout:\n{result.stdout}\n" f"stderr:\n{result.stderr}"
    )
    return result.stdout


def _render_error(result: subprocess.CompletedProcess) -> str:
    assert result.returncode != 0, f"expected the render to fail:\n{result.stdout}"
    return result.stderr


def _render(*extra_set: str) -> str:
    return _stdout(_render_result(*extra_set))


def _configmap_value(output: str, key: str) -> str:
    needle = f"{key}:"
    for line in output.splitlines():
        if needle in line:
            return line.split(":", 1)[1].strip().strip('"')
    raise AssertionError(f"{key} not found in rendered output")


def _parse_settings_data(output: str) -> dict[str, str]:
    """Return the settings ConfigMap's `data` block from a whole-chart render.

    The settings ConfigMap is selected by name, since the chart also renders
    a granular-permissions ConfigMap.
    """
    configmaps = [
        doc
        for doc in yaml.safe_load_all(output)
        if doc
        and doc.get("kind") == "ConfigMap"
        and doc["metadata"]["name"].endswith("-settings")
    ]
    assert len(configmaps) == 1, f"expected one settings ConfigMap:\n{output}"
    return configmaps[0]["data"]


def _auth_settings_data(*extra_set: str) -> dict[str, str]:
    return _parse_settings_data(_stdout(_auth_render_result(*extra_set)))


class TestJwtAudiences:
    """OMNI_EVENTS_JWT_AUDIENCES renders the chart's jwtAudiences helper."""

    def test_default_client_clientid_is_an_audience(self):
        data = _auth_settings_data(*OPENID_CONFIGURED)
        assert data[ENV_JWT_AUDIENCES] == CLIENT_ID

    def test_additional_audiences_without_default_client(self):
        # Audiences come solely from additionalJwtAudience; no default client
        # exists, so no clientId is added.
        data = _auth_settings_data(
            f"openId.openIdConfigurationUri={OPENID_URI}",
            "openId.additionalJwtAudience={api://my-resource,https://other.aud}",
        )
        assert data[ENV_JWT_AUDIENCES] == "api://my-resource,https://other.aud"

    def test_additional_audiences_unioned_with_default_client(self):
        data = _auth_settings_data(
            *OPENID_CONFIGURED, "openId.additionalJwtAudience={api://my-resource}"
        )
        assert data[ENV_JWT_AUDIENCES] == f"api://my-resource,{CLIENT_ID}"

    def test_tokenless_render_omits_the_audiences(self):
        data = _auth_settings_data(*OPENID_CONFIGURED, "principalSource=disabled")
        assert data[ENV_JWT_AUDIENCES] == ""

    def test_second_default_client_fails_the_render(self):
        # Only one "default" registration can supply the audience.
        result = _auth_render_result(
            *OPENID_CONFIGURED,
            "openId.clientRegistrations[1].name=default",
            "openId.clientRegistrations[1].clientId=other-client",
        )
        assert 'more than one entry named "default"' in _render_error(result)

    def test_absent_additional_audience_adds_nothing(self):
        # --set key=null removes the key, so this covers an absent value.
        data = _auth_settings_data(
            *OPENID_CONFIGURED, "openId.additionalJwtAudience=null"
        )
        assert data[ENV_JWT_AUDIENCES] == CLIENT_ID

    def test_empty_string_additional_audience_adds_nothing(self):
        data = _auth_settings_data(*OPENID_CONFIGURED, "openId.additionalJwtAudience=")
        assert data[ENV_JWT_AUDIENCES] == CLIENT_ID

    def test_plain_string_additional_audience_fails_the_render(self):
        result = _auth_render_result(
            *OPENID_CONFIGURED, "openId.additionalJwtAudience=api://my-resource"
        )
        assert AUDIENCE_KIND in _render_error(result)

    def test_plain_string_additional_audience_fails_the_render_when_disabled(self):
        # The audiences are not rendered for disabled, but the mistyped value
        # still fails rather than lying in wait for a later mode change.
        result = _auth_render_result(
            "principalSource=disabled", "openId.additionalJwtAudience=api://my-resource"
        )
        assert AUDIENCE_KIND in _render_error(result)


class TestPrincipalIdClaim:
    def test_openid_selected_render_emits_the_claim(self):
        # The claim is the durable-queue identity: if a token-reading mode
        # dropped it, the code default would extract a different principal and
        # orphan every durable queue created under the configured claim.
        data = _auth_settings_data(*DEPLOYMENT_AUTH_VALUES)
        assert data[ENV_PRINCIPAL_ID_CLAIM] == "oid"

    def test_disabled_principal_source_omits_claim(self):
        # principalSource=disabled is the only mode that drops the claim,
        # locking the gating contract both ways.
        assert _configmap_value(_render(), ENV_PRINCIPAL_ID_CLAIM) == ""


class TestPermissionsEnabled:
    def test_default_render_permissions_enabled_reflects_default(self):
        # The chart default permissions.enabled is false.
        output = _render()
        assert _configmap_value(output, "OMNI_EVENTS_PERMISSIONS_ENABLED") == "false"

    def test_default_render_storage_permissions_enabled_reflects_default(self):
        # The chart default permissions.storage_permissions.enabled is false.
        # This is the second of the two independent permission gates.
        output = _render()
        assert (
            _configmap_value(output, "OMNI_EVENTS_STORAGE_PERMISSIONS_ENABLED")
            == "false"
        )

    def test_storage_permissions_enabled_set_true_renders_true(self):
        # The storage-permission gate maps to its own env var, independent of
        # OMNI_EVENTS_PERMISSIONS_ENABLED.
        output = _render("permissions.storage_permissions.enabled=true")
        assert (
            _configmap_value(output, "OMNI_EVENTS_STORAGE_PERMISSIONS_ENABLED")
            == "true"
        )

    def test_permissions_enabled_set_true_renders_true(self):
        data = _auth_settings_data(*OPENID_CONFIGURED, "permissions.enabled=true")
        assert data["OMNI_EVENTS_PERMISSIONS_ENABLED"] == "true"


class TestStoragePermissionsEndpoints:
    # The storage-service endpoints are relocated out of permissions.content and
    # rendered into the permissions YAML file (granular-permissions-configmap)
    # under a `storage_permissions_endpoints:` list, one `- <url>` item per
    # entry. These tests lock that rendered contract.
    def test_default_render_emits_default_storage_endpoint(self):
        output = _render()
        assert "storage_permissions_endpoints:" in output
        assert "- http://storage-service:8012" in output

    def test_custom_endpoint_renders_under_storage_permissions_endpoints(self):
        output = _render(
            "permissions.storage_permissions.endpoints[0]=http://custom-storage:9999"
        )
        assert "storage_permissions_endpoints:" in output
        assert "- http://custom-storage:9999" in output
        assert "- http://storage-service:8012" not in output


ENV_MAX_RE_ROUTABLE = "OMNI_EVENTS_MAX_RE_ROUTABLE_STREAMS_PER_POD"
VALUES_MAX_RE_ROUTABLE = "rabbitmq.max_re_routable_streams_per_pod"

# Differs from every other number the settings ConfigMap renders (0, 1, 8, 10,
# 15, 75, 80, 100, 500, 600, 1024, 1800, 8000, 50051), so a key that picked up
# an unrelated value could not pass by coincidence.
OVERRIDE_MAX_RE_ROUTABLE = "250"


def _settings_data(*extra_set: str) -> dict[str, str]:
    """Parse the settings ConfigMap's `data` block into an ordered mapping.

    Parsed structurally rather than scanned with `_configmap_value`: a
    substring scan cannot distinguish a key that is absent from one that
    rendered empty, cannot see the YAML type of the value, and would match
    the key name inside the template's explanatory comment. Python dicts
    preserve insertion order, so the returned mapping also carries the
    rendered key ORDER, which the whitespace-regression test compares.

    The whole chart is rendered (rather than --show-only) to reuse the
    module's shared `_render` helper.
    """
    return _parse_settings_data(_render(*extra_set))


class TestMaxReRoutableStreamsPerPod:
    """The shed threshold is suppressed by null alone, never by a value.

    `max_re_routable_streams_per_pod` is `int | None` in the service
    settings: leaving it unset is meaningful (the service derives the
    midpoint of the soft-threshold/cap band itself). Because pydantic
    parses an empty-string env var as an invalid int rather than as None,
    an unconditionally rendered `OMNI_EVENTS_MAX_RE_ROUTABLE_STREAMS_PER_POD:
    ""` would fail validation and crash the pod at startup. The chart
    therefore guards the key with an explicit nil test,
    `{{- if not (kindIs "invalid" ...) }}`, rather than `{{- with }}`:
    null (or omitted) is the ONLY input that omits the key, and every
    non-null value -- including the falsy `0` -- renders as supplied and
    is left for pydantic to accept or reject at startup. These tests pin
    both halves of that contract.
    """

    def test_set_value_renders_the_quoted_value(self):
        data = _settings_data(f"{VALUES_MAX_RE_ROUTABLE}={OVERRIDE_MAX_RE_ROUTABLE}")

        assert data[ENV_MAX_RE_ROUTABLE] == OVERRIDE_MAX_RE_ROUTABLE

    def test_set_value_renders_as_a_yaml_string(self):
        # ConfigMap data values must be strings; an unquoted `{{ . }}` would
        # parse as an int and be rejected at apply time, in the cluster.
        data = _settings_data(f"{VALUES_MAX_RE_ROUTABLE}={OVERRIDE_MAX_RE_ROUTABLE}")

        assert isinstance(data[ENV_MAX_RE_ROUTABLE], str)

    def test_default_render_omits_the_key_entirely(self):
        # Not "renders empty" -- absent. An empty-valued key fails the
        # pydantic `int | None` parse and crash-loops the pod at startup.
        assert ENV_MAX_RE_ROUTABLE not in _settings_data()

    def test_zero_renders_rather_than_being_swallowed(self):
        # The contract the explicit nil test exists for: an operator-supplied
        # 0 must SURFACE as the rendered value so the service fails loudly at
        # startup on the field's `ge=1`, never silently degrade into the
        # computed midpoint. A `{{- with }}` guard treats 0 as absent and
        # would swallow it -- so this is the test that catches a revert to
        # `with`, which every other test in this class passes under.
        data = _settings_data(f"{VALUES_MAX_RE_ROUTABLE}=0")

        assert data[ENV_MAX_RE_ROUTABLE] == "0"

    def test_conditional_block_leaves_every_other_key_untouched(self):
        # The `{{- if }}` chomp markers that omit the key on the null branch
        # are the classic way to eat the newline of an adjacent line: the
        # neighbouring keys (SCALER_MAX_REPLICAS above,
        # DURABLE_CONSUMER_PREFETCH_COUNT below -- the new SR-7.1 durable and
        # cap-surface rows now sit between the shed key and REDIS_ENABLED) must
        # survive both renders with identical values AND identical positions.
        default_data = _settings_data()
        set_data = _settings_data(
            f"{VALUES_MAX_RE_ROUTABLE}={OVERRIDE_MAX_RE_ROUTABLE}"
        )
        set_data_without_new_key = {
            key: value for key, value in set_data.items() if key != ENV_MAX_RE_ROUTABLE
        }

        assert list(set_data_without_new_key.items()) == list(default_data.items())


ENV_CHANNEL_OVERFLOW = "OMNI_EVENTS_RABBITMQ_CHANNEL_OVERFLOW_ENABLED"
VALUES_CHANNEL_OVERFLOW = "rabbitmq.channel_overflow_enabled"
ENV_DURABLE_CHANNEL_OVERFLOW = "OMNI_EVENTS_RABBITMQ_DURABLE_CHANNEL_OVERFLOW_ENABLED"


class TestChannelOverflowEnabled:
    """The burst-overflow switch is non-durable only and defaults on.

    Unlike the shed threshold above, this is a plain bool with a non-null
    default, so the row is unconditional and renders on every branch. An
    un-templated setting would be silently stuck at its code default with
    no operator-reachable knob and no error anywhere, so presence on the
    default render is pinned first.
    """

    def test_default_render_emits_the_key(self):
        # Absence IS the bug: no knob, no error, no way to turn overflow off.
        assert ENV_CHANNEL_OVERFLOW in _render()

    def test_default_render_is_true(self):
        # Matches the library setting's decided default (overflow on).
        assert _configmap_value(_render(), ENV_CHANNEL_OVERFLOW) == "true"

    def test_set_false_renders_false(self):
        # Proves the row follows its values key rather than a hardcoded
        # literal, and that no `| default true` filter eats the false.
        output = _render(f"{VALUES_CHANNEL_OVERFLOW}=false")

        assert _configmap_value(output, ENV_CHANNEL_OVERFLOW) == "false"

    def test_no_durable_overflow_twin_is_rendered(self):
        # The durable pool is strict by construction: there is deliberately
        # no durable switch, so no durable twin key may reach the ConfigMap.
        assert ENV_DURABLE_CHANNEL_OVERFLOW not in _render()


ENV_HEALTH_CHECKING = "OMNI_EVENTS_GRPC_HEALTH_CHECKING_ENABLED"
VALUES_HEALTH_CHECKING = "service.grpc.healthCheckingEnabled"


class TestGrpcHealthCheckingEnabled:
    """The gRPC health-checking switch templates from values, defaults on.

    Previously this row was hardcoded `"True"` in the template; b.tz2 wired it
    to `service.grpc.healthCheckingEnabled` (a YAML bool), so the default now
    renders as lowercase `true`. `grpc_health_checking_enabled` is inherited
    from the notification_common CommonSettings base, not declared in this
    service's Settings directly. It is a plain bool with a non-null default, so
    the row is unconditional and renders on every branch. An un-templated
    setting would be silently stuck at its code default with no operator knob
    and no error anywhere, so presence on the default render is pinned first.
    """

    def test_default_render_emits_the_key(self):
        # Absence IS the bug: no knob, no error, no way to turn it off.
        assert ENV_HEALTH_CHECKING in _render()

    def test_default_render_is_true(self):
        # Values-driven default is the YAML bool `true`, rendered lowercase --
        # a change from the former hardcoded `"True"`, and correct.
        assert _configmap_value(_render(), ENV_HEALTH_CHECKING) == "true"

    def test_set_false_renders_false(self):
        # Proves the row follows its values key rather than a hardcoded literal,
        # and that no `| default true` filter eats the false.
        output = _render(f"{VALUES_HEALTH_CHECKING}=false")

        assert _configmap_value(output, ENV_HEALTH_CHECKING) == "false"

    def test_set_value_renders_as_a_yaml_string(self):
        # ConfigMap data values must be strings; an unquoted `{{ . }}` would
        # parse as a YAML bool and be rejected at apply time.
        assert isinstance(_settings_data()[ENV_HEALTH_CHECKING], str)


@pytest.fixture(scope="module")
def deployment_settings_data() -> dict[str, str]:
    """The settings ConfigMap rendered from DEPLOYMENT_AUTH_VALUES, read-only."""
    return _auth_settings_data(*DEPLOYMENT_AUTH_VALUES)


class TestPrincipalSource:
    """The effective principalSource: explicit, else jwt with OpenID, else fail."""

    def test_chart_defaults_fail_the_render(self):
        assert NO_OPENID_CONFIGURATION in _render_error(_auth_render_result())

    def test_deployment_values_select_jwt(self, deployment_settings_data):
        data = deployment_settings_data
        assert data[ENV_PRINCIPAL_SOURCE] == "jwt"

    def test_deployment_values_render_the_openid_configuration_uri(
        self, deployment_settings_data
    ):
        data = deployment_settings_data
        assert data[ENV_OPENID_CONFIGURATION_URI] == OPENID_URI

    def test_deployment_values_render_the_audience(self, deployment_settings_data):
        data = deployment_settings_data
        assert data[ENV_JWT_AUDIENCES] == CLIENT_ID

    def test_deployment_values_do_not_allow_unverified_jwt(
        self, deployment_settings_data
    ):
        data = deployment_settings_data
        assert data[ENV_ALLOW_UNVERIFIED_JWT] == "false"

    def test_additional_audience_counts_as_an_audience(self):
        data = _auth_settings_data(
            f"openId.openIdConfigurationUri={OPENID_URI}",
            "openId.additionalJwtAudience={api://my-resource}",
        )
        assert data[ENV_PRINCIPAL_SOURCE] == "jwt"

    def test_openid_uri_without_an_audience_fails_the_render(self):
        result = _auth_render_result(f"openId.openIdConfigurationUri={OPENID_URI}")
        assert NO_OPENID_CONFIGURATION in _render_error(result)

    def test_audience_without_an_openid_uri_fails_the_render(self):
        result = _auth_render_result(*OPENID_CONFIGURED[1:])
        assert NO_OPENID_CONFIGURATION in _render_error(result)

    def test_failure_names_the_missing_audience(self):
        result = _auth_render_result(f"openId.openIdConfigurationUri={OPENID_URI}")
        assert "openIdConfigurationUri: set; JWT audience: missing" in (
            _render_error(result)
        )

    def test_failure_names_the_missing_openid_uri(self):
        result = _auth_render_result(*OPENID_CONFIGURED[1:])
        assert "openIdConfigurationUri: missing; JWT audience: set" in (
            _render_error(result)
        )

    def test_auth_extension_alone_does_not_select_jwt_unverified(self):
        # The auth extension verifies only HTTPProxy traffic; in-cluster
        # callers reach the Service without it, so it is no reason to trust
        # unverified tokens.
        result = _auth_render_result(
            "httpProxy.enabled=true",
            "openId.enabled=true",
            "httpProxy.authExtension.enabled=true",
        )
        assert NO_OPENID_CONFIGURATION in _render_error(result)

    def test_whitespace_audience_is_not_an_audience(self):
        # The library strips each audience, so " " configures none.
        result = _auth_render_result(
            f"openId.openIdConfigurationUri={OPENID_URI}",
            "openId.additionalJwtAudience={ }",
        )
        assert NO_OPENID_CONFIGURATION in _render_error(result)

    def test_explicit_value_wins_over_openid_configuration(self):
        data = _auth_settings_data(*DEPLOYMENT_AUTH_VALUES, "principalSource=opaque")
        assert data[ENV_PRINCIPAL_SOURCE] == "opaque"


@pytest.fixture(scope="module")
def opt_in_settings_data() -> dict[str, str]:
    """The settings ConfigMap of an explicit jwtUnverified opt-in, read-only."""
    return _auth_settings_data(
        "principalSource=jwtUnverified", "allowUnverifiedJwt=true"
    )


class TestAllowUnverifiedJwt:
    """jwtUnverified renders only as an explicit pair with allowUnverifiedJwt."""

    def test_explicit_opt_in_renders_jwt_unverified(self, opt_in_settings_data):
        assert opt_in_settings_data[ENV_PRINCIPAL_SOURCE] == "jwtUnverified"

    def test_explicit_opt_in_renders_the_opt_in(self, opt_in_settings_data):
        assert opt_in_settings_data[ENV_ALLOW_UNVERIFIED_JWT] == "true"

    @pytest.mark.parametrize(
        "allow", ["allowUnverifiedJwt=false", "allowUnverifiedJwt=null"]
    )
    def test_jwt_unverified_without_the_opt_in_fails_the_render(self, allow):
        result = _auth_render_result("principalSource=jwtUnverified", allow)
        assert UNVERIFIED_NEEDS_OPT_IN in _render_error(result)

    def test_jwt_unverified_behind_the_auth_extension_still_needs_the_opt_in(self):
        result = _auth_render_result(
            *DEPLOYMENT_AUTH_VALUES, "principalSource=jwtUnverified"
        )
        assert UNVERIFIED_NEEDS_OPT_IN in _render_error(result)

    @pytest.mark.parametrize(
        "principal_source",
        [(), ("principalSource=jwt",), ("principalSource=opaque",)],
        ids=["openid-selected-jwt", "explicit-jwt", "explicit-opaque"],
    )
    def test_opt_in_without_explicit_jwt_unverified_fails_the_render(
        self, principal_source
    ):
        result = _auth_render_result(
            *DEPLOYMENT_AUTH_VALUES, *principal_source, "allowUnverifiedJwt=true"
        )
        assert OPT_IN_NEEDS_UNVERIFIED in _render_error(result)

    def test_opt_in_with_tokenless_mode_fails_the_render(self):
        result = _auth_render_result(
            "principalSource=disabled", "allowUnverifiedJwt=true"
        )
        assert OPT_IN_NEEDS_UNVERIFIED in _render_error(result)

    def test_explicit_false_outside_jwt_unverified_renders_false(self):
        data = _auth_settings_data(*DEPLOYMENT_AUTH_VALUES, "allowUnverifiedJwt=false")
        assert data[ENV_ALLOW_UNVERIFIED_JWT] == "false"

    @pytest.mark.parametrize("quoted", ["true", "false"])
    def test_quoted_value_fails_the_render(self, quoted):
        result = _auth_render_result(
            "principalSource=jwtUnverified",
            set_string=(f"allowUnverifiedJwt={quoted}",),
        )
        assert (
            "allowUnverifiedJwt must be true, false, or null; got a value of kind "
            "string" in _render_error(result)
        )

    def test_number_fails_the_render(self):
        # Only the prefix: the number's kind depends on how helm parsed it.
        result = _auth_render_result(*DEPLOYMENT_AUTH_VALUES, "allowUnverifiedJwt=0")
        assert "allowUnverifiedJwt must be true, false, or null; got" in (
            _render_error(result)
        )

    def test_quoted_value_is_reported_before_the_selection_fails(self):
        result = _auth_render_result(set_string=("allowUnverifiedJwt=true",))
        assert "allowUnverifiedJwt must be true, false, or null" in (
            _render_error(result)
        )


class TestValidateAuthSettings:
    """Values the library's AuthSettings would refuse fail at render instead."""

    def test_unknown_principal_source_fails_listing_the_allowed_values(self):
        result = _auth_render_result("principalSource=JwtUnverified")
        assert (
            '"JwtUnverified" is not one of: disabled, jwtUnverified, jwt, opaque, '
            "userInfoService" in _render_error(result)
        )

    def test_jwt_without_openid_uri_fails_the_render(self):
        result = _auth_render_result("principalSource=jwt", *OPENID_CONFIGURED[1:])
        assert "principalSource jwt needs openId.openIdConfigurationUri" in (
            _render_error(result)
        )

    def test_jwt_without_an_audience_fails_the_render(self):
        result = _auth_render_result(
            "principalSource=jwt", f"openId.openIdConfigurationUri={OPENID_URI}"
        )
        assert "principalSource jwt needs at least one JWT audience" in (
            _render_error(result)
        )

    def test_opaque_without_openid_uri_fails_the_render(self):
        result = _auth_render_result("principalSource=opaque")
        assert "principalSource opaque needs openId.openIdConfigurationUri" in (
            _render_error(result)
        )

    def test_opaque_with_openid_uri_renders_the_uri(self):
        data = _auth_settings_data(
            "principalSource=opaque", f"openId.openIdConfigurationUri={OPENID_URI}"
        )
        assert data[ENV_OPENID_CONFIGURATION_URI] == OPENID_URI

    def test_user_info_service_without_uri_fails_the_render(self):
        result = _auth_render_result(
            "principalSource=userInfoService", "userInfoService.uri="
        )
        assert "principalSource userInfoService needs userInfoService.uri" in (
            _render_error(result)
        )

    def test_disabled_with_permissions_fails_the_render(self):
        result = _auth_render_result(
            "principalSource=disabled", "permissions.enabled=true"
        )
        assert "permissions.enabled requires an authenticated caller" in (
            _render_error(result)
        )

    def test_disabled_with_permissions_spelled_as_a_string_fails_the_render(self):
        # pydantic reads "yes" as true, so the guard must too.
        result = _auth_render_result(
            "principalSource=disabled", set_string=("permissions.enabled=yes",)
        )
        assert "permissions.enabled requires an authenticated caller" in (
            _render_error(result)
        )

    def test_disabled_with_permissions_quoted_false_renders(self):
        # pydantic reads the rendered "false" as false, so the guard must too;
        # Go template truthiness would read the quoted string as true.
        result = _auth_render_result(
            "principalSource=disabled", set_string=("permissions.enabled=false",)
        )
        data = _parse_settings_data(_stdout(result))
        assert data["OMNI_EVENTS_PERMISSIONS_ENABLED"] == "false"

    def test_permissions_without_endpoint_fails_the_render(self):
        # The service's own Settings validator refuses this at startup.
        result = _auth_render_result(
            *OPENID_CONFIGURED, "permissions.enabled=true", "permissions.endpoint="
        )
        assert "permissions.enabled needs permissions.endpoint" in (
            _render_error(result)
        )

    def test_permissions_spelled_as_a_string_without_endpoint_fails_the_render(
        self,
    ):
        result = _auth_render_result(
            *OPENID_CONFIGURED,
            "permissions.endpoint=",
            set_string=("permissions.enabled=yes",),
        )
        assert "permissions.enabled needs permissions.endpoint" in (
            _render_error(result)
        )

    def test_permissions_quoted_false_without_endpoint_renders(self):
        result = _auth_render_result(
            *OPENID_CONFIGURED,
            "permissions.endpoint=",
            set_string=("permissions.enabled=false",),
        )
        data = _parse_settings_data(_stdout(result))
        assert data["OMNI_EVENTS_PERMISSIONS_ENDPOINT"] == ""


class TestCorsAllowOrigins:
    def test_default_render_lists_no_origins(self):
        assert _settings_data()[ENV_CORS_ALLOW_ORIGINS] == "[]"

    def test_null_renders_no_origins(self):
        data = _settings_data("service.rest.corsAllowOrigins=null")
        assert data[ENV_CORS_ALLOW_ORIGINS] == "[]"

    def test_origins_render_as_a_json_array(self):
        data = _settings_data(
            "service.rest.corsAllowOrigins={https://a.example.com,https://b.example.com}"
        )
        assert json.loads(data[ENV_CORS_ALLOW_ORIGINS]) == [
            "https://a.example.com",
            "https://b.example.com",
        ]

    def test_plain_string_fails_the_render(self):
        result = _render_result("service.rest.corsAllowOrigins=https://a.example.com")
        assert (
            "service.rest.corsAllowOrigins must be a YAML list of origins"
            in _render_error(result)
        )

    def test_non_string_entry_fails_the_render(self):
        result = _render_result("service.rest.corsAllowOrigins={1}")
        assert "corsAllowOrigins entries must be origin strings" in (
            _render_error(result)
        )

    def test_yaml_null_entry_fails_the_render(self):
        # --set reads a bare null as YAML null, not as the "null" origin string.
        result = _render_result("service.rest.corsAllowOrigins={null}")
        assert "service.rest.corsAllowOrigins contains a YAML null" in (
            _render_error(result)
        )

    @pytest.mark.parametrize("origin", ["*", "null", " NULL "])
    def test_refused_origin_fails_the_render(self, origin):
        # --set-string, because --set reads a bare null as YAML null rather
        # than as the "null" origin string.
        result = _auth_render_result(
            "principalSource=disabled",
            set_string=(
                f"service.rest.corsAllowOrigins={{https://a.example.com,{origin}}}",
            ),
        )
        assert "corsAllowOrigins must not contain" in _render_error(result)

    def test_malformed_origin_is_left_to_the_library(self):
        # The chart checks only the list's kind and the "*" / "null" entries.
        # The origin format (scheme://host[:port], no path) is the library's
        # startup check, so a trailing slash renders here as given.
        data = _settings_data("service.rest.corsAllowOrigins={https://a.example.com/}")
        assert json.loads(data[ENV_CORS_ALLOW_ORIGINS]) == ["https://a.example.com/"]


# Value combinations that render, each fed to the library below. Between them
# they cover every principalSource the chart accepts and a CORS origin list.
STARTABLE_VALUES = {
    "deployment-openid-selected-jwt": DEPLOYMENT_AUTH_VALUES,
    "explicit-jwt-additional-audience": (
        "principalSource=jwt",
        f"openId.openIdConfigurationUri={OPENID_URI}",
        "openId.additionalJwtAudience={api://my-resource}",
    ),
    "opaque": ("principalSource=opaque", f"openId.openIdConfigurationUri={OPENID_URI}"),
    "user-info-service": ("principalSource=userInfoService",),
    "jwt-unverified-opt-in-with-permissions": (
        "principalSource=jwtUnverified",
        "allowUnverifiedJwt=true",
        "permissions.enabled=true",
    ),
    "tokenless-with-cors-origins": (
        "principalSource=disabled",
        "service.rest.corsAllowOrigins={https://a.example.com,http://localhost:3000}",
    ),
}


@pytest.fixture
def rendered_environment(monkeypatch, request) -> dict[str, str]:
    """The settings ConfigMap of one STARTABLE_VALUES render, as the environment.

    Every OMNI_EVENTS_* variable the test session set is removed first, so the
    settings read only what the chart rendered.
    """
    data = _auth_settings_data(*STARTABLE_VALUES[request.param])
    for key in [key for key in os.environ if key.startswith("OMNI_EVENTS_")]:
        monkeypatch.delenv(key)
    for key, value in data.items():
        monkeypatch.setenv(key, value)
    return data


@pytest.mark.parametrize("rendered_environment", list(STARTABLE_VALUES), indirect=True)
class TestRenderedSettingsLoad:
    """If the settings ConfigMap renders, the settings the service builds accept it.

    The render guards mirror notification-common's startup validators and the
    service's own permissions-endpoint check; these tests catch the guards and
    the validators drifting apart for the combinations above, by building the
    library's settings and the service's Settings from nothing but the
    rendered environment.
    """

    def test_common_settings_load(self, rendered_environment):
        expected = json.loads(rendered_environment[ENV_CORS_ALLOW_ORIGINS])
        assert CommonSettings().cors_allow_origins == expected

    def test_auth_settings_load(self, rendered_environment, mocker):
        # AuthSettings reads permissions_enabled from the module's
        # common_settings singleton, which the test session built from its
        # own environment, so it is swapped for one built from the render.
        mocker.patch.object(auth_settings_module, "common_settings", CommonSettings())
        expected = rendered_environment[ENV_PRINCIPAL_SOURCE]
        assert AuthSettings().principal_source.value == expected

    def test_service_settings_load(self, rendered_environment):
        # The service's Settings adds its own startup checks, such as
        # permissions.enabled needing permissions.endpoint, to the library's.
        expected = rendered_environment[ENV_PERMISSIONS_ENABLED] == "true"
        assert Settings().permissions_enabled is expected
