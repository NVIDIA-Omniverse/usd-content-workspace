# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Tests for the app_scripts module."""

import os
import sys
from unittest.mock import MagicMock

import pytest

from notification_common.scripts import app_scripts, header_lint
from notification_common.scripts.app_scripts import (
    generate_openapi_spec_internal,
    generate_openapi_yaml,
)


@pytest.fixture
def mock_openapi_schema():
    """Fixture for a mock OpenAPI schema."""
    return {
        "openapi": "3.1.0",
        "info": {"title": "Test API", "version": "1.0.0"},
        "paths": {"/test": {"get": {}}},
        "components": {"schemas": {"TestSchema": {}}},
    }


@pytest.fixture
def mock_fastapi_app(mocker, mock_openapi_schema):
    """Fixture for a mocked FastAPI app with importlib setup."""
    mock_module = MagicMock()
    mock_app = MagicMock()
    mock_module.rest_server = mock_app
    mock_app.openapi.return_value = mock_openapi_schema

    mocker.patch.object(
        sys.modules["notification_common.scripts.app_scripts"].importlib,
        "import_module",
        return_value=mock_module,
    )

    return mock_app


@pytest.fixture
def mock_generate_internal(mocker):
    """Fixture for mocking generate_openapi_spec_internal."""
    return mocker.patch.object(
        sys.modules["notification_common.scripts.app_scripts"],
        "generate_openapi_spec_internal",
    )


@pytest.fixture
def emitted_header_content(mock_fastapi_app, tmp_path):
    """Generate the OpenAPI file once and return the emitted file's text.

    Shared setup for the header-assertion tests, which each generate the same
    spec and read back the same file; only their assertions differ.
    """
    output_file = tmp_path / "openapi.yaml"
    generate_openapi_spec_internal(
        "app.rest.rest_server", "rest_server", str(output_file), False
    )
    return output_file.read_text()


class TestGenerateOpenapiSpecInternal:
    """Tests for the generate_openapi_spec_internal function."""

    def test_generates_openapi_yaml(self, mock_fastapi_app, tmp_path):
        """Test that the function generates an OpenAPI YAML file."""
        output_file = tmp_path / "openapi.yaml"
        generate_openapi_spec_internal(
            "app.rest.rest_server", "rest_server", str(output_file), False
        )

        assert output_file.exists()
        content = output_file.read_text()
        assert "openapi: 3.1.0" in content
        assert "Test API" in content

    def test_handles_import_error(self, mocker, tmp_path):
        """Test that the function handles import errors."""
        mocker.patch.object(
            sys.modules["notification_common.scripts.app_scripts"].importlib,
            "import_module",
            side_effect=ImportError("Module not found"),
        )

        output_file = tmp_path / "openapi.yaml"

        with pytest.raises(SystemExit) as exc_info:
            generate_openapi_spec_internal(
                "app.rest.rest_server", "rest_server", str(output_file), False
            )

        assert exc_info.value.code == 1

    def test_handles_attribute_error(self, mocker, tmp_path):
        """Test that the function handles missing app attribute."""
        mock_module = MagicMock()
        mock_module.rest_server = None
        mocker.patch.object(
            sys.modules["notification_common.scripts.app_scripts"].importlib,
            "import_module",
            return_value=mock_module,
        )

        mocker.patch.object(
            sys.modules["notification_common.scripts.app_scripts"],
            "getattr",
            side_effect=AttributeError("Attribute not found"),
        )

        output_file = tmp_path / "openapi.yaml"

        with pytest.raises(SystemExit) as exc_info:
            generate_openapi_spec_internal(
                "app.rest.rest_server", "rest_server", str(output_file), False
            )

        assert exc_info.value.code == 1

    def test_creates_output_directory(self, mock_fastapi_app, tmp_path):
        """Test that the function creates the output directory."""
        output_file = tmp_path / "nested" / "dir" / "openapi.yaml"
        generate_openapi_spec_internal(
            "app.rest.rest_server", "rest_server", str(output_file), False
        )

        assert output_file.exists()

    def test_verbose_logging(self, mock_fastapi_app, tmp_path):
        """Test that verbose logging is enabled when requested."""
        output_file = tmp_path / "openapi.yaml"
        generate_openapi_spec_internal(
            "app.rest.rest_server", "rest_server", str(output_file), True
        )

        assert output_file.exists()

    def test_sets_placeholder_environment_vars(
        self, mock_fastapi_app, tmp_path, monkeypatch
    ):
        """Test that placeholder environment variables are set."""
        monkeypatch.delenv("OMNI_EVENTS_OPENID_CONFIGURATION_URI", False)
        monkeypatch.delenv("OMNI_EVENTS_JWT_AUDIENCES", False)

        output_file = tmp_path / "openapi.yaml"
        generate_openapi_spec_internal(
            "app.rest.rest_server",
            "rest_server",
            str(output_file),
            False,
        )

        assert (
            os.environ["OMNI_EVENTS_OPENID_CONFIGURATION_URI"]
            == "https://example.com/.well-known/openid-configuration"
        )
        assert os.environ["OMNI_EVENTS_JWT_AUDIENCES"] == "openapi-generation"

    def test_selects_the_verifying_principal_source_when_unset(
        self, mock_fastapi_app, tmp_path, monkeypatch
    ):
        """Test that an unset principal source becomes the verifying jwt one."""
        monkeypatch.delenv("OMNI_EVENTS_PRINCIPAL_SOURCE", False)

        generate_openapi_spec_internal(
            "app.rest.rest_server",
            "rest_server",
            str(tmp_path / "openapi.yaml"),
            False,
        )

        assert os.environ["OMNI_EVENTS_PRINCIPAL_SOURCE"] == "jwt"

    def test_preserves_existing_environment_vars(
        self, mock_fastapi_app, tmp_path, monkeypatch
    ):
        """Test that existing environment variables are preserved."""
        existing_endpoint = "https://custom.endpoint.com/.well-known"
        existing_audience = "custom-audience"
        monkeypatch.setenv(
            "OMNI_EVENTS_OPENID_CONFIGURATION_URI",
            existing_endpoint,
        )
        monkeypatch.setenv("OMNI_EVENTS_JWT_AUDIENCES", existing_audience)

        output_file = tmp_path / "openapi.yaml"
        generate_openapi_spec_internal(
            "app.rest.rest_server",
            "rest_server",
            str(output_file),
            False,
        )

        assert os.environ["OMNI_EVENTS_OPENID_CONFIGURATION_URI"] == existing_endpoint
        assert os.environ["OMNI_EVENTS_JWT_AUDIENCES"] == existing_audience

    def test_emits_single_line_apache_copyright(self, emitted_header_content):
        """Test that the copyright is one physical single-line Apache line.

        Locks the byte-exact first line so a future edit that re-wraps the
        copyright (splitting ``CORPORATION & AFFILIATES.`` onto a second
        physical line) fails here. event_consumer_service's header-lint
        depends on this exact single-line form (Bee b.7z8).
        """
        # Built from concatenated pieces (the emitted line is 98 chars, which
        # would itself trip flake8 as a single literal) to mirror the source.
        expected_copyright = (
            "# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA "
            "CORPORATION & AFFILIATES. All rights reserved."
        )

        first_line = emitted_header_content.splitlines()[0]
        assert first_line == expected_copyright

    def test_emits_apache_license_identifier(self, emitted_header_content):
        """Test that the second header line is the Apache-2.0 identifier."""
        second_line = emitted_header_content.splitlines()[1]
        assert second_line == "# SPDX-License-Identifier: Apache-2.0"

    def test_header_has_no_proprietary_license_ref(self, emitted_header_content):
        """Test that the emitted header carries no proprietary LicenseRef."""
        assert "LicenseRef-NvidiaProprietary" not in emitted_header_content

    def test_header_has_no_proprietary_boilerplate(self, emitted_header_content):
        """Test that the proprietary boilerplate text is absent."""
        assert "strictly prohibited" not in emitted_header_content


class TestGenerateOpenapiYaml:
    """Tests for the generate_openapi_yaml function."""

    def test_parses_command_line_arguments(self, mocker, mock_generate_internal):
        """Test that the function parses command-line arguments."""
        mocker.patch.object(
            sys,
            "argv",
            [
                "generate_openapi_yaml",
                "-m",
                "custom.module",
                "-a",
                "custom_app",
                "-o",
                "custom.yaml",
                "-v",
            ],
        )

        generate_openapi_yaml()

        mock_generate_internal.assert_called_once_with(
            "custom.module", "custom_app", "custom.yaml", True
        )

    def test_uses_default_arguments(self, mocker, mock_generate_internal):
        """Test that the function uses default arguments."""
        mocker.patch.object(sys, "argv", ["generate_openapi_yaml"])

        generate_openapi_yaml()

        mock_generate_internal.assert_called_once_with(
            "app.rest.rest_server", "rest_server", "openapi.yaml", False
        )

    def test_uses_environment_variables(
        self, mocker, monkeypatch, mock_generate_internal
    ):
        """Test that the function uses environment variables."""
        monkeypatch.setenv("FASTAPI_MODULE", "env.module")
        monkeypatch.setenv("FASTAPI_APP_VAR", "env_app")
        mocker.patch.object(sys, "argv", ["generate_openapi_yaml"])

        generate_openapi_yaml()

        mock_generate_internal.assert_called_once_with(
            "env.module", "env_app", "openapi.yaml", False
        )

    def test_command_line_overrides_environment(
        self, mocker, monkeypatch, mock_generate_internal
    ):
        """Test that command-line args override environment variables."""
        monkeypatch.setenv("FASTAPI_MODULE", "env.module")
        monkeypatch.setenv("FASTAPI_APP_VAR", "env_app")
        mocker.patch.object(sys, "argv", ["generate_openapi_yaml", "-m", "cli.module"])

        generate_openapi_yaml()

        mock_generate_internal.assert_called_once_with(
            "cli.module", "env_app", "openapi.yaml", False
        )


class TestUpdateProto:
    """Tests for ``update_proto``: run ``buf generate``, then stamp the stubs."""

    @pytest.fixture
    def stub_dir(self, mocker, tmp_path):
        """Point the stub directory at ``tmp_path`` and stub out ``buf``."""
        mocker.patch.object(app_scripts, "repo_root", return_value=tmp_path)
        stub_dir = tmp_path / app_scripts.GENERATED_STUBS_DIR
        stub_dir.mkdir(parents=True)

        def _fake_buf_generate(args):
            (stub_dir / "thing_pb2.py").write_text(
                "# -*- coding: utf-8 -*-\n# Generated.\n", encoding="utf-8"
            )
            (stub_dir / "thing_pb2.pyi").write_text("import grpc\n", encoding="utf-8")

        mocker.patch.object(app_scripts, "check_call", side_effect=_fake_buf_generate)
        return stub_dir

    def test_runs_buf_generate(self, stub_dir):
        app_scripts.update_proto()
        app_scripts.check_call.assert_called_once_with(["buf", "generate"])

    @pytest.mark.parametrize("name", ["thing_pb2.py", "thing_pb2.pyi"])
    def test_stamps_each_generated_stub(self, stub_dir, name):
        app_scripts.update_proto()
        assert header_lint._has_expected_header(stub_dir / name)
