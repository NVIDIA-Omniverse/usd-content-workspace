# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Tests for the generate_attributions module."""

import json
import subprocess

import pytest

from notification_common.scripts import generate_attributions
from notification_common.scripts.generate_attributions import (
    format_attribution,
    generate_attributions_content,
    get_dependencies_with_licenses,
    main,
    parse_existing_attributions,
    should_update_file,
)


@pytest.fixture
def mock_dependency():
    """Fixture for a mock dependency."""
    return {
        "Name": "test-package",
        "Version": "1.0.0",
        "License": "MIT",
        "LicenseText": "MIT License\n\nCopyright (c) 2024\n",
    }


@pytest.fixture
def mock_dependencies():
    """Fixture for a list of mock dependencies."""
    return [
        {
            "Name": "package-b",
            "Version": "2.0.0",
            "License": "Apache-2.0",
            "LicenseText": "Apache License 2.0\n",
        },
        {
            "Name": "package-a",
            "Version": "1.5.0",
            "License": "BSD-3-Clause",
            "LicenseText": "BSD 3-Clause License\n",
        },
    ]


@pytest.fixture
def mock_subprocess_run(mocker):
    """Fixture for mocking subprocess.run."""
    return mocker.patch.object(
        subprocess, "run", return_value=mocker.MagicMock(stdout="{}")
    )


class TestGetDependenciesWithLicenses:
    """Tests for the get_dependencies_with_licenses function."""

    def test_returns_dependencies_list(self, mocker):
        """Test that the function returns a list of dependencies."""
        expected_deps = [
            {"Name": "test", "Version": "1.0", "License": "MIT", "LicenseText": "..."}
        ]
        mock_result = mocker.MagicMock()
        mock_result.stdout = json.dumps(expected_deps)
        mocker.patch.object(subprocess, "run", return_value=mock_result)

        result = get_dependencies_with_licenses()

        assert result == expected_deps

    def test_calls_uv_run_pip_licenses(self, mocker):
        """Test that the function calls uv run pip-licenses."""
        mock_result = mocker.MagicMock()
        mock_result.stdout = "[]"
        mock_run = mocker.patch.object(subprocess, "run", return_value=mock_result)

        get_dependencies_with_licenses()

        mock_run.assert_called_once_with(
            [
                "uv",
                "run",
                "pip-licenses",
                "--format",
                "json",
                "--with-license-file",
                "--no-license-path",
            ],
            capture_output=True,
            text=True,
            check=True,
        )

    def test_handles_subprocess_error(self, mocker):
        """Test that the function handles subprocess errors."""
        mocker.patch.object(
            subprocess,
            "run",
            side_effect=subprocess.CalledProcessError(1, "cmd", stderr="error message"),
        )

        with pytest.raises(SystemExit) as exc_info:
            get_dependencies_with_licenses()

        assert exc_info.value.code == 1

    def test_handles_json_decode_error(self, mocker):
        """Test that the function handles JSON decode errors."""
        mock_result = mocker.MagicMock()
        mock_result.stdout = "invalid json"
        mocker.patch.object(subprocess, "run", return_value=mock_result)

        with pytest.raises(SystemExit) as exc_info:
            get_dependencies_with_licenses()

        assert exc_info.value.code == 1


class TestFormatAttribution:
    """Tests for the format_attribution function."""

    def test_formats_dependency_correctly(self, mock_dependency):
        """Test that the function formats a dependency correctly."""
        result = format_attribution(mock_dependency)

        assert "## test-package@1.0.0" in result
        assert "MIT" in result
        assert "MIT License" in result
        assert "Copyright (c) 2024" in result

    def test_handles_missing_name(self):
        """Test that the function handles missing name."""
        dep = {"Version": "1.0.0", "License": "MIT", "LicenseText": "..."}
        result = format_attribution(dep)

        assert "## Unknown@1.0.0" in result

    def test_handles_missing_version(self):
        """Test that the function handles missing version."""
        dep = {"Name": "test", "License": "MIT", "LicenseText": "..."}
        result = format_attribution(dep)

        assert "## test@Unknown" in result

    def test_handles_missing_license(self):
        """Test that the function handles missing license."""
        dep = {"Name": "test", "Version": "1.0.0", "LicenseText": "..."}
        result = format_attribution(dep)

        assert "UNKNOWN" in result

    def test_handles_missing_license_text(self):
        """Test that the function handles missing license text."""
        dep = {"Name": "test", "Version": "1.0.0", "License": "MIT"}
        result = format_attribution(dep)

        assert "No license text available" in result

    def test_strips_trailing_newlines(self):
        """Test that the function strips trailing newlines."""
        dep = {
            "Name": "test",
            "Version": "1.0.0",
            "License": "MIT",
            "LicenseText": "License text\n\n\n",
        }
        result = format_attribution(dep)

        assert result.endswith("License text\n")
        assert not result.endswith("License text\n\n\n")


class TestParseExistingAttributions:
    """Tests for the parse_existing_attributions function."""

    def test_parses_empty_content(self):
        """Test that the function parses empty content."""
        result = parse_existing_attributions("")

        assert result == {}

    def test_parses_single_entry(self):
        """Test that the function parses a single entry."""
        content = "# Attributions\n\n## package@1.0.0\n\nMIT\n\nLicense text"
        result = parse_existing_attributions(content)

        assert "package@1.0.0" in result
        assert "MIT" in result["package@1.0.0"]
        assert "License text" in result["package@1.0.0"]

    def test_parses_multiple_entries(self):
        """Test that the function parses multiple entries."""
        content = (
            "# Attributions\n\n"
            "## package-a@1.0.0\n\nMIT\n\nLicense A\n\n"
            "## package-b@2.0.0\n\nApache\n\nLicense B"
        )
        result = parse_existing_attributions(content)

        assert len(result) == 2
        assert "package-a@1.0.0" in result
        assert "package-b@2.0.0" in result
        assert "License A" in result["package-a@1.0.0"]
        assert "License B" in result["package-b@2.0.0"]

    def test_ignores_lines_without_at_symbol(self):
        """Test that the function ignores ## lines without @."""
        content = "## Not a package\n\n## package@1.0.0\n\nMIT"
        result = parse_existing_attributions(content)

        assert "Not a package" not in result
        assert "package@1.0.0" in result


class TestGenerateAttributionsContent:
    """Tests for the generate_attributions_content function."""

    def test_generates_content_with_header(self, mock_dependencies):
        """Test that the function generates content with a header."""
        result = generate_attributions_content(mock_dependencies)

        assert result.startswith("# Attributions\n")

    def test_sorts_dependencies_alphabetically(self, mock_dependencies):
        """Test that the function sorts dependencies alphabetically."""
        result = generate_attributions_content(mock_dependencies)

        # package-a should come before package-b
        pos_a = result.find("package-a@1.5.0")
        pos_b = result.find("package-b@2.0.0")
        assert pos_a < pos_b

    def test_includes_all_dependencies(self, mock_dependencies):
        """Test that the function includes all dependencies."""
        result = generate_attributions_content(mock_dependencies)

        assert "package-a@1.5.0" in result
        assert "package-b@2.0.0" in result
        assert "BSD-3-Clause" in result
        assert "Apache-2.0" in result

    def test_handles_empty_dependencies_list(self):
        """Test that the function handles an empty dependencies list."""
        result = generate_attributions_content([])

        assert result == "# Attributions\n"


class TestShouldUpdateFile:
    """Tests for the should_update_file function."""

    def test_returns_true_when_file_does_not_exist(self):
        """Test that the function returns True when file doesn't exist."""
        result = should_update_file("new content", None)

        assert result is True

    def test_returns_false_when_content_identical(self):
        """Test that the function returns False when content is identical."""
        content = "# Attributions\n\n## package@1.0.0\n\nMIT\n\nLicense text"
        result = should_update_file(content, content)

        assert result is False

    def test_returns_true_when_packages_differ(self):
        """Test that the function returns True when packages differ."""
        old_content = "# Attributions\n\n## package-a@1.0.0\n\nMIT\n\nLicense"
        new_content = "# Attributions\n\n## package-b@1.0.0\n\nMIT\n\nLicense"
        result = should_update_file(new_content, old_content)

        assert result is True

    def test_returns_true_when_content_differs(self):
        """Test that the function returns True when content differs."""
        old_content = "# Attributions\n\n## package@1.0.0\n\nMIT\n\nOld license"
        new_content = "# Attributions\n\n## package@1.0.0\n\nMIT\n\nNew license"
        result = should_update_file(new_content, old_content)

        assert result is True

    def test_returns_true_when_package_added(self):
        """Test that the function returns True when a package is added."""
        old_content = "# Attributions\n\n## package-a@1.0.0\n\nMIT\n\nLicense"
        new_content = (
            "# Attributions\n\n"
            "## package-a@1.0.0\n\nMIT\n\nLicense\n\n"
            "## package-b@1.0.0\n\nMIT\n\nLicense"
        )
        result = should_update_file(new_content, old_content)

        assert result is True


class TestMain:
    """Tests for the main function."""

    @pytest.fixture
    def mock_main_environment(self, mocker, tmp_path):
        """Fixture to set up common mocking for main() tests."""

        def _setup(dependencies):
            # Mock subprocess.run to return the given dependencies
            mock_result = mocker.MagicMock()
            mock_result.stdout = json.dumps(dependencies)
            mocker.patch.object(subprocess, "run", return_value=mock_result)

            # Patch Path.cwd() to return tmp_path
            mock_path_class = mocker.patch.object(generate_attributions, "Path")
            mock_path_class.cwd.return_value = tmp_path

            return tmp_path / "ATTRIBUTIONS.md"

        return _setup

    def test_creates_attributions_file(self, mock_main_environment):
        """Test that the function creates an ATTRIBUTIONS.md file."""
        mock_deps = [
            {
                "Name": "test",
                "Version": "1.0.0",
                "License": "MIT",
                "LicenseText": "License",
            }
        ]
        attributions_file = mock_main_environment(mock_deps)

        main()

        # Verify file was created
        assert attributions_file.exists()
        content = attributions_file.read_text()
        assert "test@1.0.0" in content
        assert "MIT" in content

    def test_updates_existing_file_when_changed(self, mock_main_environment):
        """Test that the function updates existing file when changed."""
        old_deps = [
            {
                "Name": "old-package",
                "Version": "1.0.0",
                "License": "MIT",
                "LicenseText": "Old",
            }
        ]
        new_deps = [
            {
                "Name": "new-package",
                "Version": "2.0.0",
                "License": "Apache",
                "LicenseText": "New",
            }
        ]

        # Create existing file with old content
        attributions_file = mock_main_environment(new_deps)
        old_content = generate_attributions_content(old_deps)
        attributions_file.write_text(old_content)

        main()

        # Verify file was updated
        content = attributions_file.read_text()
        assert "new-package@2.0.0" in content
        assert "old-package@1.0.0" not in content

    def test_does_not_update_when_no_changes(self, mock_main_environment, capsys):
        """Test that the function doesn't update when no changes."""
        deps = [
            {
                "Name": "test",
                "Version": "1.0.0",
                "License": "MIT",
                "LicenseText": "License",
            }
        ]

        # Create existing file with same content
        attributions_file = mock_main_environment(deps)
        content = generate_attributions_content(deps)
        attributions_file.write_text(content)

        main()

        # Verify file was not modified
        captured = capsys.readouterr()
        assert "up to date" in captured.out
