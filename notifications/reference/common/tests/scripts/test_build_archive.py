# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Tests for the build_archive module."""

import shutil
import subprocess
import zipfile

import pytest

from notification_common.scripts import build_archive
from notification_common.scripts.build_archive import (
    build_docs,
    create_archive,
    get_docs_project_and_version,
    get_project_info_from_pyproject,
    main,
)


def _create_mock_project_structure(tmp_path, service_name, proto_subdir):
    """Helper to create a mock project structure for testing.

    Args:
        tmp_path: Temporary directory path
        service_name: Name of the service (e.g., "event-consumer-service")
        proto_subdir: Proto subdirectory name (e.g., "consumer" or "publisher")

    Returns:
        The temporary path with the mock structure created
    """
    # Create pyproject.toml
    pyproject = tmp_path / "pyproject.toml"
    pyproject.write_text(f'[project]\nname = "{service_name}"\nversion = "1.2.3"\n')

    # Create notification_protos structure
    protos_dir = (
        tmp_path
        / "notification_protos"
        / "protos"
        / "nvidia"
        / "omniverse"
        / "notifications"
        / proto_subdir
    )
    protos_dir.mkdir(parents=True)
    (protos_dir / "test.proto").write_text("syntax = 'proto3';")

    # Create repo.toml (drives the docs output path) and the docs build
    # output at the derived <project>/<version> location.
    docs_dir = tmp_path / "notification_protos" / "docs"
    docs_dir.mkdir(parents=True, exist_ok=True)
    (docs_dir / "repo.toml").write_text(
        '[repo_docs]\nproject = "notification-api"\nversion = "1.0.0-beta.2"\n'
    )
    docs_build = docs_dir / "_build" / "docs" / "notification-api" / "1.0.0-beta.2"
    docs_build.mkdir(parents=True)
    (docs_build / "index.html").write_text("<html>Docs</html>")

    # Create openapi.yaml
    openapi = tmp_path / "openapi.yaml"
    openapi.write_text("openapi: 3.0.0\n")

    # Create oss directory
    oss_dir = tmp_path / "oss"
    oss_dir.mkdir()
    (oss_dir / "OSS_LICENSE.txt").write_text("OSS License")

    # Create LICENSE.txt
    (tmp_path / "LICENSE.txt").write_text("License")

    # Create CHANGELOG.md
    (tmp_path / "CHANGELOG.md").write_text("# Changelog\n")

    return tmp_path


@pytest.fixture
def mock_project_structure(tmp_path):
    """Create a mock project structure for testing (consumer service)."""
    return _create_mock_project_structure(
        tmp_path, "event-consumer-service", "consumer"
    )


@pytest.fixture
def mock_aggregation_project_structure(tmp_path):
    """Create a mock project structure for testing (aggregation service)."""
    return _create_mock_project_structure(
        tmp_path, "event-aggregation-service", "publisher"
    )


@pytest.fixture
def mock_cwd(tmp_path, monkeypatch):
    """Mock Path.cwd() to return tmp_path."""
    # Actually change the current working directory
    monkeypatch.chdir(tmp_path)
    return tmp_path


class TestGetProjectInfoFromPyproject:
    """Tests for the get_project_info_from_pyproject function."""

    def test_extracts_consumer_info_correctly(self, mock_cwd, mock_project_structure):
        """Test that the function extracts consumer project info correctly."""
        project_name, version, proto_subdir = get_project_info_from_pyproject()

        assert project_name == "event-consumer-service"
        assert version == "1.2.3"
        assert proto_subdir == "consumer"

    def test_extracts_aggregation_info_correctly(
        self, mock_cwd, mock_aggregation_project_structure
    ):
        """Test that the function extracts aggregation project info correctly."""
        project_name, version, proto_subdir = get_project_info_from_pyproject()

        assert project_name == "event-aggregation-service"
        assert version == "1.2.3"
        assert proto_subdir == "publisher"

    def test_handles_missing_file(self, mock_cwd, tmp_path):
        """Test that the function handles missing pyproject.toml."""
        with pytest.raises(FileNotFoundError):
            get_project_info_from_pyproject()

    def test_handles_malformed_toml(self, mock_cwd, tmp_path):
        """Test that the function handles malformed TOML."""
        pyproject = tmp_path / "pyproject.toml"
        pyproject.write_text("invalid toml {{{")

        with pytest.raises(Exception):
            get_project_info_from_pyproject()

    def test_handles_unknown_project_type(self, mock_cwd, tmp_path):
        """Test that the function raises error for unknown project types."""
        pyproject = tmp_path / "pyproject.toml"
        pyproject.write_text('[project]\nname = "unknown-service"\nversion = "1.0.0"\n')

        with pytest.raises(ValueError, match="Unknown project type"):
            get_project_info_from_pyproject()


class TestGetDocsProjectAndVersion:
    """Tests for the get_docs_project_and_version function."""

    def test_reads_project_and_version_from_repo_toml(self, mock_cwd, tmp_path):
        """Test that project and version are read from repo.toml [repo_docs]."""
        docs_dir = tmp_path / "notification_protos" / "docs"
        docs_dir.mkdir(parents=True)
        (docs_dir / "repo.toml").write_text(
            '[repo_docs]\nproject = "notification-api"\nversion = "9.9.9-beta.1"\n'
        )

        project, version = get_docs_project_and_version()

        assert project == "notification-api"
        assert version == "9.9.9-beta.1"

    def test_raises_when_repo_toml_missing(self, mock_cwd, tmp_path):
        """Test that a missing repo.toml raises FileNotFoundError."""
        with pytest.raises(FileNotFoundError):
            get_docs_project_and_version()


class TestBuildDocs:
    """Tests for the build_docs function."""

    def test_calls_repo_script(self, mocker, mock_cwd, mock_project_structure):
        """Test that the function calls the repo.sh script."""
        repo_script = (
            mock_project_structure / "notification_protos" / "docs" / "repo.sh"
        )
        repo_script.write_text("#!/bin/bash\necho 'Building docs'\n")
        repo_script.chmod(0o755)

        mock_run = mocker.patch.object(
            subprocess,
            "run",
            return_value=mocker.MagicMock(stdout="Built successfully"),
        )

        build_docs()

        mock_run.assert_called_once()
        call_args = mock_run.call_args
        assert str(repo_script) in str(call_args[0][0][0])
        assert "docs" in call_args[0][0]
        assert call_args[1]["cwd"] == mock_project_structure
        assert call_args[1]["check"] is True

    def test_handles_subprocess_error(self, mocker, mock_cwd, mock_project_structure):
        """Test that the function handles subprocess errors."""
        repo_script = (
            mock_project_structure / "notification_protos" / "docs" / "repo.sh"
        )
        repo_script.write_text("#!/bin/bash\nexit 1\n")

        error = subprocess.CalledProcessError(1, "cmd")
        error.stdout = "out"
        error.stderr = "err"
        mocker.patch.object(subprocess, "run", side_effect=error)

        with pytest.raises(subprocess.CalledProcessError):
            build_docs()


class TestCreateArchive:
    """Tests for the create_archive function."""

    def test_creates_consumer_archive_with_correct_name(
        self, mock_cwd, mock_project_structure
    ):
        """Test that the function creates consumer archive with correct name."""
        archive_name = create_archive()

        assert archive_name == "notifications-consumer-api-1.2.3.zip"
        assert (mock_project_structure / archive_name).exists()

    def test_creates_aggregation_archive_with_correct_name(
        self, mock_cwd, mock_aggregation_project_structure
    ):
        """Test that the function creates aggregation archive with the correct name."""
        archive_name = create_archive()

        assert archive_name == "notifications-aggregation-api-1.2.3.zip"
        assert (mock_aggregation_project_structure / archive_name).exists()

    def test_includes_consumer_proto_files(self, mock_cwd, mock_project_structure):
        """Test that the archive includes consumer proto files."""
        archive_name = create_archive()

        with zipfile.ZipFile(mock_project_structure / archive_name, "r") as zipf:
            assert any(
                "protos/nvidia/omniverse/notifications/consumer/test.proto" in name
                for name in zipf.namelist()
            )

    def test_includes_publisher_proto_files(
        self, mock_cwd, mock_aggregation_project_structure
    ):
        """Test that the archive includes publisher proto files."""
        archive_name = create_archive()

        with zipfile.ZipFile(
            mock_aggregation_project_structure / archive_name, "r"
        ) as zipf:
            assert any(
                "protos/nvidia/omniverse/notifications/publisher/test.proto" in name
                for name in zipf.namelist()
            )

    def test_includes_docs(self, mock_cwd, mock_project_structure):
        """Test that the archive includes documentation."""
        archive_name = create_archive()

        with zipfile.ZipFile(mock_project_structure / archive_name, "r") as zipf:
            assert any("docs/index.html" in name for name in zipf.namelist())

    def test_includes_openapi(self, mock_cwd, mock_project_structure):
        """Test that the archive includes OpenAPI spec."""
        archive_name = create_archive()

        with zipfile.ZipFile(mock_project_structure / archive_name, "r") as zipf:
            assert "openapi/v1beta/openapi.yaml" in zipf.namelist()

    def test_includes_oss_files(self, mock_cwd, mock_project_structure):
        """Test that the archive includes OSS files."""
        archive_name = create_archive()

        with zipfile.ZipFile(mock_project_structure / archive_name, "r") as zipf:
            assert "OSS_LICENSE.txt" in zipf.namelist()

    def test_includes_license(self, mock_cwd, mock_project_structure):
        """Test that the archive includes LICENSE.txt."""
        archive_name = create_archive()

        with zipfile.ZipFile(mock_project_structure / archive_name, "r") as zipf:
            assert "LICENSE.txt" in zipf.namelist()

    def test_does_not_include_attributions(self, mock_cwd, mock_project_structure):
        """Test that the archive does not include ATTRIBUTIONS.md."""
        archive_name = create_archive()

        with zipfile.ZipFile(mock_project_structure / archive_name, "r") as zipf:
            assert "ATTRIBUTIONS.md" not in zipf.namelist()

    def test_includes_changelog(self, mock_cwd, mock_project_structure):
        """Test that the archive includes CHANGELOG.md."""
        archive_name = create_archive()

        with zipfile.ZipFile(mock_project_structure / archive_name, "r") as zipf:
            assert "CHANGELOG.md" in zipf.namelist()

    def test_raises_when_docs_build_missing(self, mock_cwd, mock_project_structure):
        """Test that a missing docs build is a hard error, not a warning."""
        # Keep repo.toml but remove the built docs output.
        docs_build_root = (
            mock_project_structure / "notification_protos" / "docs" / "_build"
        )
        shutil.rmtree(docs_build_root)

        with pytest.raises(RuntimeError, match="No documentation found"):
            create_archive()

    def test_handles_missing_optional_files(self, mock_cwd, tmp_path, capsys):
        """Test that the function handles missing optional files.

        Docs are mandatory (repo.toml + the built docs output must be
        present), but openapi/oss/license/changelog are optional and only
        produce warnings when absent.
        """
        # pyproject.toml plus the mandatory repo.toml + docs build.
        pyproject = tmp_path / "pyproject.toml"
        pyproject.write_text(
            '[project]\nname = "event-consumer-service"\nversion = "1.0.0"\n'
        )
        docs_dir = tmp_path / "notification_protos" / "docs"
        docs_dir.mkdir(parents=True)
        (docs_dir / "repo.toml").write_text(
            '[repo_docs]\nproject = "notification-api"\nversion = "1.0.0-beta.2"\n'
        )
        docs_build = docs_dir / "_build" / "docs" / "notification-api" / "1.0.0-beta.2"
        docs_build.mkdir(parents=True)
        (docs_build / "index.html").write_text("<html>Docs</html>")

        archive_name = create_archive()

        # Should still create archive
        assert archive_name == "notifications-consumer-api-1.0.0.zip"
        assert (tmp_path / archive_name).exists()

        # Check for warning messages (missing openapi/oss/license/changelog)
        captured = capsys.readouterr()
        assert "Warning" in captured.out


class TestMain:
    """Tests for the main function."""

    @pytest.fixture
    def mock_build_docs(self, mocker):
        """Fixture to mock build_docs."""
        return mocker.patch.object(build_archive, "build_docs")

    @pytest.fixture
    def mock_create_archive(self, mocker):
        """Fixture to mock create_archive."""
        return mocker.patch.object(
            build_archive, "create_archive", return_value="test-archive.zip"
        )

    def test_calls_build_docs_and_create_archive(
        self, mock_build_docs, mock_create_archive
    ):
        """Test that main calls both build_docs and create_archive."""
        result = main()

        mock_build_docs.assert_called_once()
        mock_create_archive.assert_called_once()
        assert result == 0

    def test_returns_zero_on_success(self, mock_build_docs, mock_create_archive):
        """Test that main returns 0 on success."""
        result = main()

        assert result == 0

    def test_returns_one_on_error(self, mock_build_docs, mock_create_archive):
        """Test that main returns 1 on error."""
        mock_build_docs.side_effect = Exception("Test error")

        result = main()

        assert result == 1

    def test_handles_build_docs_error(self, mock_build_docs, mock_create_archive):
        """Test that main handles build_docs errors."""
        mock_build_docs.side_effect = subprocess.CalledProcessError(1, "cmd")

        result = main()

        assert result == 1
        mock_create_archive.assert_not_called()

    def test_handles_create_archive_error(self, mock_build_docs, mock_create_archive):
        """Test that main handles create_archive errors."""
        mock_create_archive.side_effect = Exception("Archive error")

        result = main()

        assert result == 1
