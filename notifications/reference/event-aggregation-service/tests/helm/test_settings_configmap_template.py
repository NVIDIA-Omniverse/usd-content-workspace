# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import json

import pytest

from tests.helm.conftest import (
    SETTINGS_CONFIGMAP_TEMPLATE,
    render,
    render_raw,
    requires_helm,
    settings_data,
)

pytestmark = requires_helm


def _configmap_value(output: str, key: str) -> str:
    needle = f"{key}:"
    for line in output.splitlines():
        if needle in line:
            return line.split(":", 1)[1].strip().strip('"')
    raise AssertionError(f"{key} not found in rendered output")


def _principal_id_claim(output: str) -> str:
    return _configmap_value(output, "OMNI_EVENTS_PRINCIPAL_ID_CLAIM")


class TestRestMaxRequestBytes:
    """The REST body cap renders as a plain byte count, not in float notation.

    Helm decodes a number this large as a float64, so the row pipes it through
    `int64`. Without that it renders as 4.194304e+06, which pydantic-settings
    cannot parse as an integer -- a value the chart ships by default would
    crash-loop the pod. Presence of the key itself is covered by
    test_settings_coverage.py; what is pinned here is the rendered form.
    """

    def test_default_render_emits_the_plain_byte_count(self):
        value = _configmap_value(render(), "OMNI_EVENTS_REST_MAX_REQUEST_BYTES")

        assert value == "4194304"

    def test_follows_its_values_key(self):
        # A sentinel distinct from every other number the ConfigMap renders,
        # so a row cross-wired to another values key cannot pass by accident.
        output = render("rest.maxRequestBytes=7340032")

        assert (
            _configmap_value(output, "OMNI_EVENTS_REST_MAX_REQUEST_BYTES") == "7340032"
        )


# Values equivalent to the deployment repository's dev and performance
# overlays (environments/{dev,performance}/openid.yaml): an OpenID provider, the
# "default" client registration, and the HTTPProxy with the Envoy auth
# extension, but no principalSource.
DEPLOYMENT_OPENID = (
    "httpProxy.enabled=true",
    "openId.enabled=true",
    "openId.openIdConfigurationUri=https://idp.example.com/.well-known/openid-configuration",
    "openId.clientRegistrations[0].name=default",
    "openId.clientRegistrations[0].clientId=default-client",
)

# The HTTPProxy with the Envoy auth extension and nothing else. The extension
# verifies only HTTPProxy traffic, so these values alone are no reason to trust
# unverified tokens.
AUTH_EXTENSION_ONLY = ("httpProxy.enabled=true", "openId.enabled=true")

URI_ONLY = ("openId.openIdConfigurationUri=https://idp.example.com/x",)

JWT_UNVERIFIED_OPT_IN = ("principalSource=jwtUnverified", "allowUnverifiedJwt=true")

NO_OPENID_MESSAGE = "principalSource is unset and OpenID is not fully configured"
JWT_UNVERIFIED_MESSAGE = "jwtUnverified trusts any token that reaches the pod"
OPT_IN_PAIRING_MESSAGE = "allowUnverifiedJwt applies only to principalSource"
AUDIENCE_KIND_MESSAGE = "openId.additionalJwtAudience must be a YAML list"


def _auth_data(*extra_set: str) -> dict:
    """The settings ConfigMap data with no auth base, so the chart selects."""
    return settings_data(*extra_set, base=())


def _auth_render_error(*extra_set: str, set_string: tuple[str, ...] = ()) -> str:
    """The stderr of a render with no auth base, asserting that it failed."""
    result = render_raw(*extra_set, base=(), set_string=set_string)
    assert result.returncode != 0, f"render unexpectedly succeeded:\n{result.stdout}"
    return result.stderr


class TestPrincipalIdClaim:
    def test_openid_render_emits_non_empty_claim(self):
        # The principalIdClaim invariant: a render that authenticates (here
        # the chart-selected jwt) must carry a non-empty
        # OMNI_EVENTS_PRINCIPAL_ID_CLAIM. If it were dropped, the code default
        # would extract the wrong principal.
        data = _auth_data(*DEPLOYMENT_OPENID)

        assert data["OMNI_EVENTS_PRINCIPAL_ID_CLAIM"] != ""

    def test_disabled_principal_source_omits_claim(self):
        # principalSource=disabled is the only mode that drops the claim,
        # locking the gating contract both ways.
        assert _principal_id_claim(render("principalSource=disabled")) == ""


class TestPrincipalSource:
    def test_chart_defaults_fail_render(self):
        assert NO_OPENID_MESSAGE in _auth_render_error()

    def test_auth_extension_alone_fails_render(self):
        # The auth extension covers only HTTPProxy traffic, so it does not
        # make the chart select jwtUnverified.
        assert NO_OPENID_MESSAGE in _auth_render_error(*AUTH_EXTENSION_ONLY)

    def test_deployment_openid_values_select_jwt(self):
        data = _auth_data(*DEPLOYMENT_OPENID)

        assert data["OMNI_EVENTS_PRINCIPAL_SOURCE"] == "jwt"

    def test_additional_audience_alone_selects_jwt(self):
        data = _auth_data(*URI_ONLY, "openId.additionalJwtAudience[0]=extra")

        assert data["OMNI_EVENTS_PRINCIPAL_SOURCE"] == "jwt"

    def test_uri_without_audience_fails_render(self):
        assert NO_OPENID_MESSAGE in _auth_render_error(*URI_ONLY)

    def test_failure_names_the_missing_audience(self):
        error = _auth_render_error(*URI_ONLY)

        assert "openIdConfigurationUri: set; JWT audience: missing" in error

    def test_blank_client_id_is_not_an_audience(self):
        error = _auth_render_error(
            *URI_ONLY,
            "openId.clientRegistrations[0].name=default",
            set_string=("openId.clientRegistrations[0].clientId= ",),
        )

        assert NO_OPENID_MESSAGE in error

    def test_non_default_client_is_not_an_audience(self):
        error = _auth_render_error(
            *URI_ONLY,
            "openId.clientRegistrations[0].name=other",
            "openId.clientRegistrations[0].clientId=other-client",
        )

        assert NO_OPENID_MESSAGE in error

    def test_audience_without_uri_fails_render(self):
        error = _auth_render_error(
            "openId.clientRegistrations[0].name=default",
            "openId.clientRegistrations[0].clientId=default-client",
        )

        assert NO_OPENID_MESSAGE in error

    def test_explicit_value_wins_over_openid(self):
        data = _auth_data(*DEPLOYMENT_OPENID, "principalSource=userInfoService")

        assert data["OMNI_EVENTS_PRINCIPAL_SOURCE"] == "userInfoService"

    @pytest.mark.parametrize("value", ["bogus", "JWT", "true"])
    def test_unknown_value_fails_render(self, value):
        error = _auth_render_error(f"principalSource={value}")

        assert f'principalSource "{value}" is not one of' in error


class TestJwtAudiences:
    def test_renders_the_default_client_id(self):
        data = _auth_data(*DEPLOYMENT_OPENID)

        assert data["OMNI_EVENTS_JWT_AUDIENCES"] == "default-client"

    def test_appends_the_default_client_id_to_additional_audiences(self):
        data = _auth_data(*DEPLOYMENT_OPENID, "openId.additionalJwtAudience[0]=extra")

        assert data["OMNI_EVENTS_JWT_AUDIENCES"] == "extra,default-client"

    def test_trims_each_audience(self):
        data = _auth_data(
            *URI_ONLY,
            "openId.clientRegistrations[0].name=default",
            "openId.clientRegistrations[0].clientId= default-client ",
        )

        assert data["OMNI_EVENTS_JWT_AUDIENCES"] == "default-client"

    def test_second_default_client_fails_render(self):
        error = _auth_render_error(
            *DEPLOYMENT_OPENID,
            "openId.clientRegistrations[1].name=default",
            "openId.clientRegistrations[1].clientId=other-client",
        )

        assert 'more than one entry named "default"' in error

    def test_is_empty_outside_jwt_and_opaque(self):
        data = _auth_data(*DEPLOYMENT_OPENID, "principalSource=userInfoService")

        assert data["OMNI_EVENTS_JWT_AUDIENCES"] == ""

    def test_absent_additional_audience_adds_nothing(self):
        # --set key=null removes the key, so this covers an absent value.
        data = _auth_data(*DEPLOYMENT_OPENID, "openId.additionalJwtAudience=null")

        assert data["OMNI_EVENTS_JWT_AUDIENCES"] == "default-client"

    def test_empty_string_additional_audience_adds_nothing(self):
        data = _auth_data(*DEPLOYMENT_OPENID, "openId.additionalJwtAudience=")

        assert data["OMNI_EVENTS_JWT_AUDIENCES"] == "default-client"

    def test_plain_string_additional_audience_fails_render(self):
        error = _auth_render_error(
            *DEPLOYMENT_OPENID, "openId.additionalJwtAudience=extra"
        )

        assert AUDIENCE_KIND_MESSAGE in error

    def test_plain_string_additional_audience_fails_render_when_disabled(self):
        # The audiences are not rendered for disabled, but the mistyped value
        # still fails rather than lying in wait for a later mode change.
        error = _auth_render_error(
            "principalSource=disabled", "openId.additionalJwtAudience=extra"
        )

        assert AUDIENCE_KIND_MESSAGE in error


class TestOpenIdConfigurationUri:
    def test_deployment_openid_values_render_the_uri(self):
        data = _auth_data(*DEPLOYMENT_OPENID)

        assert data["OMNI_EVENTS_OPENID_CONFIGURATION_URI"] == (
            "https://idp.example.com/.well-known/openid-configuration"
        )


class TestAllowUnverifiedJwt:
    def test_deployment_openid_values_do_not_allow_unverified_jwt(self):
        data = _auth_data(*DEPLOYMENT_OPENID)

        assert data["OMNI_EVENTS_ALLOW_UNVERIFIED_JWT"] == "false"

    def test_explicit_jwt_unverified_with_opt_in_renders_true(self):
        data = _auth_data(*JWT_UNVERIFIED_OPT_IN)

        assert data["OMNI_EVENTS_ALLOW_UNVERIFIED_JWT"] == "true"

    def test_explicit_jwt_unverified_with_opt_in_selects_it(self):
        data = _auth_data(*DEPLOYMENT_OPENID, *JWT_UNVERIFIED_OPT_IN)

        assert data["OMNI_EVENTS_PRINCIPAL_SOURCE"] == "jwtUnverified"

    @pytest.mark.parametrize(
        "opt_in", [(), ("allowUnverifiedJwt=false",)], ids=["unset", "false"]
    )
    def test_explicit_jwt_unverified_without_opt_in_fails_render(self, opt_in):
        error = _auth_render_error("principalSource=jwtUnverified", *opt_in)

        assert JWT_UNVERIFIED_MESSAGE in error

    def test_auth_extension_does_not_stand_in_for_the_opt_in(self):
        error = _auth_render_error(*DEPLOYMENT_OPENID, "principalSource=jwtUnverified")

        assert JWT_UNVERIFIED_MESSAGE in error

    @pytest.mark.parametrize("value", ["true", "false"])
    def test_quoted_value_fails_render(self, value):
        error = _auth_render_error(
            "principalSource=jwtUnverified",
            set_string=(f"allowUnverifiedJwt={value}",),
        )

        assert f"got a value of kind string ({value})" in error

    def test_number_fails_render(self):
        error = _auth_render_error("principalSource=disabled", "allowUnverifiedJwt=0")

        assert "got a value of kind int64 (0)" in error

    def test_quoted_value_is_reported_before_the_selection_fails(self):
        # The chart defaults would also fail the selection (no OpenID), but
        # the quoted opt-in is the first thing the render reports.
        error = _auth_render_error(set_string=("allowUnverifiedJwt=true",))

        assert "allowUnverifiedJwt must be true, false, or null" in error

    def test_opt_in_without_jwt_unverified_fails_render(self):
        error = _auth_render_error(*DEPLOYMENT_OPENID, "allowUnverifiedJwt=true")

        assert OPT_IN_PAIRING_MESSAGE in error

    def test_opt_in_with_disabled_fails_render(self):
        error = _auth_render_error(
            "principalSource=disabled", "allowUnverifiedJwt=true"
        )

        assert OPT_IN_PAIRING_MESSAGE in error


class TestValidateAuthSettings:
    """Each mode's requirements fail the render, as the service would at startup."""

    @pytest.mark.parametrize(
        ("values", "message"),
        [
            (
                ("principalSource=jwt",),
                "principalSource jwt needs openId.openIdConfigurationUri",
            ),
            (
                ("principalSource=jwt", *URI_ONLY),
                "principalSource jwt needs at least one JWT audience",
            ),
            (
                ("principalSource=opaque",),
                "principalSource opaque needs openId.openIdConfigurationUri",
            ),
            (
                ("principalSource=userInfoService", "userInfoService.uri="),
                "principalSource userInfoService needs userInfoService.uri",
            ),
            (
                ("principalSource=disabled", "permissions.enabled=true"),
                "permissions.enabled requires an authenticated caller",
            ),
            (
                (
                    *JWT_UNVERIFIED_OPT_IN,
                    "permissions.enabled=true",
                    "permissions.endpoint=",
                ),
                "permissions.enabled needs permissions.endpoint",
            ),
        ],
        ids=[
            "jwt-without-uri",
            "jwt-without-audience",
            "opaque-without-uri",
            "user-info-service-without-uri",
            "disabled-with-permissions",
            "permissions-without-endpoint",
        ],
    )
    def test_unmet_requirement_fails_render(self, values, message):
        assert message in _auth_render_error(*values)

    @pytest.mark.parametrize(
        "values",
        [
            ("principalSource=disabled",),
            (*JWT_UNVERIFIED_OPT_IN, "permissions.endpoint="),
        ],
        ids=["disabled", "without-endpoint"],
    )
    def test_quoted_false_permissions_pass_the_permissions_checks(self, values):
        # pydantic reads the rendered "false" as false, so the guards must
        # too; Go template truthiness would read the quoted string as true.
        output = render(
            *values,
            base=(),
            show_only=SETTINGS_CONFIGMAP_TEMPLATE,
            set_string=("permissions.enabled=false",),
        )

        assert _configmap_value(output, "OMNI_EVENTS_PERMISSIONS_ENABLED") == "false"

    @pytest.mark.parametrize(
        ("values", "message"),
        [
            (
                ("principalSource=disabled",),
                "permissions.enabled requires an authenticated caller",
            ),
            (
                (*JWT_UNVERIFIED_OPT_IN, "permissions.endpoint="),
                "permissions.enabled needs permissions.endpoint",
            ),
        ],
        ids=["disabled", "without-endpoint"],
    )
    def test_permissions_spelled_as_a_string_fail_render(self, values, message):
        # pydantic reads "yes" as true, so the guards must too.
        error = _auth_render_error(*values, set_string=("permissions.enabled=yes",))

        assert message in error


class TestCorsAllowOrigins:
    def test_default_render_lists_no_origins(self):
        output = render()
        assert _configmap_value(output, "OMNI_EVENTS_CORS_ALLOW_ORIGINS") == "[]"

    def test_null_renders_no_origins(self):
        data = settings_data("rest.corsAllowOrigins=null")

        assert data["OMNI_EVENTS_CORS_ALLOW_ORIGINS"] == "[]"

    def test_origins_render_as_a_json_array(self):
        data = settings_data(
            "rest.corsAllowOrigins={https://a.example.com,https://b.example.com}"
        )

        assert json.loads(data["OMNI_EVENTS_CORS_ALLOW_ORIGINS"]) == [
            "https://a.example.com",
            "https://b.example.com",
        ]

    def test_plain_string_fails_render(self):
        result = render_raw("rest.corsAllowOrigins=https://a.example.com")

        assert "rest.corsAllowOrigins must be a YAML list" in result.stderr

    @pytest.mark.parametrize("origin", ["*", "null", " NULL "])
    def test_refused_origin_fails_render(self, origin):
        result = render_raw(set_string=(f"rest.corsAllowOrigins[0]={origin}",))

        assert f'must not contain "{origin}"' in result.stderr

    def test_refused_origin_among_valid_ones_fails_render(self):
        result = render_raw("rest.corsAllowOrigins={https://a.example.com,*}")

        assert 'must not contain "*"' in result.stderr

    def test_yaml_null_entry_fails_render(self):
        result = render_raw("rest.corsAllowOrigins={null}")

        assert "rest.corsAllowOrigins contains a YAML null" in result.stderr
