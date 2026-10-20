# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

from pathlib import Path

import pytest

from notification_common.config import metadata

# noinspection PyProtectedMember
from notification_common.config.metadata import _pyproject_path, _read_pyproject_field


class TestVersionNumber:
    def test_returns_version_num_from_yml_file(self, mocker):
        spy = mocker.patch.object(
            metadata, "_read_pyproject_field", return_value="v-num"
        )
        assert metadata.version_number() == "v-num"
        spy.assert_called_once_with("version")


class TestProjectName:
    def test_returns_project_name_from_yml_file(self, mocker):
        spy = mocker.patch.object(
            metadata, "_read_pyproject_field", return_value="p-name"
        )
        assert metadata.project_name() == "p-name"
        spy.assert_called_once_with("name")


class TestReadPyProjectField:
    def test_returns_name_field_value(self):
        assert _read_pyproject_field("name") == "notification-common"


class TestProjectPath:
    @pytest.fixture
    def path_to_proj_file(self):
        return Path(__file__).parent.parent.parent

    class TestWhenProjectFileInCurrentDirectory:
        def test_returns_project_path(self, path_to_proj_file, mocker):
            mocker.patch.object(Path, "cwd", return_value=path_to_proj_file)
            assert _pyproject_path() == str(path_to_proj_file / "pyproject.toml")

    def test_returns_project_path(self, path_to_proj_file):
        assert _pyproject_path() == str(path_to_proj_file / "pyproject.toml")
