# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &
# AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Tests for the header_lint module.

Drives the in-repo Apache-2.0 header lint against small ``tmp_path`` fixture
trees (never the live repo tree) so the pass/fail behaviour is demonstrable per
SR-3.3 and the tests stay order-independent under ``--random-order``.
"""

import pytest

from notification_common.scripts import header_lint

# The exact wrapped 3-physical-line Apache-2.0 header standardized by Task 2.
# Byte-identical to what the license swap emits and to the matcher in the lint
# under test; each physical line stays <= 88 chars (a single-line copyright is
# 98 chars and would itself trip flake8 on this module).
HEADER_LINE_1 = "# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION &"
HEADER_LINE_2 = "# AFFILIATES. All rights reserved."
HEADER_LINE_3 = "# SPDX-License-Identifier: Apache-2.0"

COMPLIANT_HEADER = f"{HEADER_LINE_1}\n{HEADER_LINE_2}\n{HEADER_LINE_3}\n"

# The still-proprietary header the license swap replaces: correct first two
# lines but the wrong SPDX identifier, so it must be flagged.
PROPRIETARY_HEADER = (
    f"{HEADER_LINE_1}\n"
    f"{HEADER_LINE_2}\n"
    "# SPDX-License-Identifier: LicenseRef-NvidiaProprietary\n"
)

# Only physical line 1 of the wrapped header is present.
PARTIAL_HEADER = f"{HEADER_LINE_1}\n"

BODY = '\n"""A published module."""\n'

# The PEP 263 encoding line protoc writes on line 1 of every ``*_pb2.py``.
CODING_LINE = "# -*- coding: utf-8 -*-\n"


@pytest.fixture
def write_py(tmp_path):
    """Return a helper that writes a ``.py`` file under ``tmp_path``.

    The helper creates any missing parent directories and returns the written
    path, so tests build a mini source tree without hardcoding line numbers.
    """

    def _write(relpath, content):
        path = tmp_path / relpath
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
        return path

    return _write


@pytest.fixture
def compliant_file(write_py):
    """Write a compliant sample under the ``notification_common`` root."""
    return write_py("notification_common/sample.py", COMPLIANT_HEADER + BODY)


@pytest.fixture
def noncompliant_file(write_py):
    """Write a deliberately non-compliant sample under the ``tests`` root."""
    return write_py("tests/sample.py", PROPRIETARY_HEADER + BODY)


class TestCompliantHeaderFixture:
    """Lock the compliant fixture to Task 2's exact wrapped header form."""

    def test_header_lines_are_the_wrapped_apache_form(self):
        expected = (
            "# SPDX-FileCopyrightText: Copyright (c) 2025 "
            "NVIDIA CORPORATION &\n"
            "# AFFILIATES. All rights reserved.\n"
            "# SPDX-License-Identifier: Apache-2.0\n"
        )
        assert COMPLIANT_HEADER == expected

    def test_every_fixture_line_stays_within_line_limit(self):
        lines = COMPLIANT_HEADER + PROPRIETARY_HEADER + PARTIAL_HEADER
        assert all(len(line) <= 88 for line in lines.splitlines())


class TestHasExpectedHeader:
    """Tests for ``header_lint._has_expected_header``."""

    def test_compliant_header_passes(self, write_py):
        path = write_py("notification_common/a.py", COMPLIANT_HEADER + BODY)
        assert header_lint._has_expected_header(path) is True

    def test_proprietary_header_fails(self, write_py):
        path = write_py("tests/a.py", PROPRIETARY_HEADER + BODY)
        assert header_lint._has_expected_header(path) is False

    def test_partial_header_fails(self, write_py):
        path = write_py("tests/a.py", PARTIAL_HEADER + BODY)
        assert header_lint._has_expected_header(path) is False

    def test_header_not_at_top_fails(self, write_py):
        path = write_py("tests/a.py", '"""Docstring first."""\n' + COMPLIANT_HEADER)
        assert header_lint._has_expected_header(path) is False

    def test_empty_file_fails(self, write_py):
        path = write_py("tests/a.py", "")
        assert header_lint._has_expected_header(path) is False

    def test_optional_shebang_is_tolerated(self, write_py):
        path = write_py(
            "tests/a.py", "#!/usr/bin/env python\n" + COMPLIANT_HEADER + BODY
        )
        assert header_lint._has_expected_header(path) is True

    def test_optional_coding_line_is_tolerated(self, write_py):
        path = write_py("nvidia/a_pb2.py", CODING_LINE + COMPLIANT_HEADER + BODY)
        assert header_lint._has_expected_header(path) is True

    def test_shebang_then_coding_line_is_tolerated(self, write_py):
        content = "#!/usr/bin/env python\n" + CODING_LINE + COMPLIANT_HEADER + BODY
        path = write_py("tests/a.py", content)
        assert header_lint._has_expected_header(path) is True

    def test_header_after_an_ordinary_comment_fails(self, write_py):
        path = write_py("tests/a.py", "# A comment.\n" + COMPLIANT_HEADER + BODY)
        assert header_lint._has_expected_header(path) is False

    def test_crlf_line_endings_are_tolerated(self, write_py):
        content = COMPLIANT_HEADER.replace("\n", "\r\n") + BODY
        path = write_py("tests/a.py", content)
        assert header_lint._has_expected_header(path) is True

    def test_non_2025_copyright_year_passes(self, write_py):
        # The matcher treats the year as a ``\d{4}`` wildcard; every fixture
        # hardcodes 2025, so a different valid year guards against a regression
        # that pins the year in the regex.
        line_1 = "# SPDX-FileCopyrightText: Copyright (c) 2024 NVIDIA CORPORATION &"
        header = f"{line_1}\n{HEADER_LINE_2}\n{HEADER_LINE_3}\n"
        path = write_py("notification_common/a.py", header + BODY)
        assert header_lint._has_expected_header(path) is True

    def test_unreadable_file_fails(self, tmp_path):
        missing = tmp_path / "gone.py"
        assert header_lint._has_expected_header(missing) is False


class TestFindMissingHeaders:
    """Tests for ``header_lint.find_missing_headers``."""

    def test_compliant_tree_reports_no_violations(self, tmp_path, compliant_file):
        assert header_lint.find_missing_headers(tmp_path) == []

    def test_noncompliant_file_is_reported(self, tmp_path, noncompliant_file):
        assert header_lint.find_missing_headers(tmp_path) == [noncompliant_file]

    def test_partial_header_is_reported(self, tmp_path, write_py):
        path = write_py("tests/a.py", PARTIAL_HEADER + BODY)
        assert header_lint.find_missing_headers(tmp_path) == [path]

    def test_header_not_at_top_is_reported(self, tmp_path, write_py):
        path = write_py("tests/a.py", BODY + COMPLIANT_HEADER)
        assert header_lint.find_missing_headers(tmp_path) == [path]

    def test_empty_file_is_reported(self, tmp_path, write_py):
        path = write_py("notification_common/a.py", "")
        assert header_lint.find_missing_headers(tmp_path) == [path]

    def test_in_scope_file_under_notification_common_is_checked(
        self, tmp_path, write_py
    ):
        path = write_py("notification_common/pkg/mod.py", PROPRIETARY_HEADER)
        assert path in header_lint.find_missing_headers(tmp_path)

    def test_in_scope_file_under_tests_is_checked(self, tmp_path, write_py):
        path = write_py("tests/pkg/mod.py", PROPRIETARY_HEADER)
        assert path in header_lint.find_missing_headers(tmp_path)

    def test_generated_nvidia_stub_is_checked(self, tmp_path, write_py):
        path = write_py("nvidia/pkg/v1/thing_pb2.py", CODING_LINE + BODY)
        assert header_lint.find_missing_headers(tmp_path) == [path]

    def test_pyi_stub_is_checked(self, tmp_path, write_py):
        path = write_py("nvidia/pkg/v1/thing_pb2.pyi", BODY)
        assert header_lint.find_missing_headers(tmp_path) == [path]

    def test_stamped_nvidia_stubs_pass(self, tmp_path, write_py):
        write_py("nvidia/pkg/v1/thing_pb2.py", CODING_LINE + COMPLIANT_HEADER + BODY)
        write_py("nvidia/pkg/v1/thing_pb2.pyi", COMPLIANT_HEADER + BODY)
        assert header_lint.find_missing_headers(tmp_path) == []

    def test_pycache_tree_is_skipped(self, tmp_path, write_py):
        write_py("notification_common/__pycache__/mod.py", BODY)
        assert header_lint.find_missing_headers(tmp_path) == []

    @pytest.mark.parametrize(
        "excluded_root",
        ["scripts", "docs", "tools", ".vscode", ".bees", ".idea"],
    )
    def test_out_of_scope_root_is_skipped(self, tmp_path, write_py, excluded_root):
        write_py(f"{excluded_root}/mod.py", BODY)
        assert header_lint.find_missing_headers(tmp_path) == []

    def test_missing_source_root_is_tolerated(self, tmp_path):
        assert header_lint.find_missing_headers(tmp_path) == []

    def test_violations_are_sorted(self, tmp_path, write_py):
        second = write_py("tests/b.py", PROPRIETARY_HEADER)
        first = write_py("notification_common/a.py", PROPRIETARY_HEADER)
        assert header_lint.find_missing_headers(tmp_path) == sorted([second, first])

    def test_default_root_resolves_via_repo_root(self, tmp_path, write_py, mocker):
        offender = write_py("tests/a.py", PROPRIETARY_HEADER)
        mocker.patch.object(header_lint, "repo_root", return_value=tmp_path)
        assert header_lint.find_missing_headers() == [offender]


class TestCheckHeaders:
    """Tests for ``header_lint.check_headers``."""

    def test_clean_tree_returns_without_exit(self, mocker):
        mocker.patch.object(header_lint, "find_missing_headers", return_value=[])
        assert header_lint.check_headers() is None

    def test_violation_raises_systemexit(self, mocker, tmp_path):
        mocker.patch.object(
            header_lint,
            "find_missing_headers",
            return_value=[tmp_path / "tests" / "a.py"],
        )
        with pytest.raises(SystemExit) as excinfo:
            header_lint.check_headers()
        assert excinfo.value.code == 1

    def test_violation_is_printed_to_stderr(self, mocker, tmp_path, capsys):
        offender = tmp_path / "tests" / "a.py"
        mocker.patch.object(
            header_lint, "find_missing_headers", return_value=[offender]
        )
        with pytest.raises(SystemExit):
            header_lint.check_headers()
        assert str(offender) in capsys.readouterr().err


class TestRepoRoot:
    """Tests for ``header_lint.repo_root``."""

    def test_resolves_to_a_directory_holding_the_package(self):
        assert (header_lint.repo_root() / "notification_common").is_dir()


class TestStampHeader:
    """Tests for ``header_lint.stamp_header``."""

    def test_stamps_at_the_top(self, write_py):
        path = write_py("nvidia/a_pb2_grpc.py", BODY)
        header_lint.stamp_header(path, 2025)
        assert path.read_text(encoding="utf-8") == COMPLIANT_HEADER + "\n" + BODY

    def test_stamps_after_the_coding_line(self, write_py):
        path = write_py("nvidia/a_pb2.py", CODING_LINE + BODY)
        header_lint.stamp_header(path, 2025)
        expected = CODING_LINE + COMPLIANT_HEADER + "\n" + BODY
        assert path.read_text(encoding="utf-8") == expected

    def test_stamped_file_passes_the_lint(self, write_py):
        path = write_py("nvidia/a_pb2.py", CODING_LINE + BODY)
        header_lint.stamp_header(path, 2026)
        assert header_lint._has_expected_header(path) is True

    def test_returns_true_when_it_stamps(self, write_py):
        path = write_py("nvidia/a_pb2.pyi", BODY)
        assert header_lint.stamp_header(path, 2025) is True

    def test_is_idempotent(self, write_py):
        path = write_py("nvidia/a_pb2.pyi", BODY)
        header_lint.stamp_header(path, 2025)
        assert header_lint.stamp_header(path, 2025) is False

    def test_leaves_a_file_with_an_spdx_header_unchanged(self, write_py):
        path = write_py("tests/a.py", PROPRIETARY_HEADER + BODY)
        header_lint.stamp_header(path, 2025)
        assert path.read_text(encoding="utf-8") == PROPRIETARY_HEADER + BODY
