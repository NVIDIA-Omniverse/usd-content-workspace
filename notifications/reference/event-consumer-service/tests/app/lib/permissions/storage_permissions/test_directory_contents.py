# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import time

import pytest
from notification_common.errors.errors import ServerError

from app.config import settings as _settings_module
from app.lib.permissions.storage_permissions import directory_contents as dc_module
from app.lib.permissions.storage_permissions import redis_cache
from app.lib.permissions.storage_permissions.directory_contents import (
    DirectoryContents,
    ListingOutcome,
)
from tests.fixtures.cache_scope import CACHE_SCOPE_KEY

#: What addresses a stream's cache: the broker queue name plus the
#: presenting principal's reference. The mutators build Ops from the
#: writer's copy of it and never see the halves separately.
SCOPE_KEY = CACHE_SCOPE_KEY
DIR_PATH = "/some/dir/"


# noinspection PyUnusedLocal
@pytest.fixture
def dir_contents(freezer):
    return DirectoryContents(["a"], ["dir_a"])


@pytest.fixture
def dir_contents_with_path(freezer):
    return DirectoryContents(["a"], ["dir_a"], dir_path=DIR_PATH)


class _FakeWriter:
    """
    Minimal stand-in for CacheWriter exposing only what the mutators
    use: ``scope_key`` and a synchronous ``enqueue``. Records every Op
    pushed for later assertions.
    """

    def __init__(self, scope_key: str = SCOPE_KEY) -> None:
        self.scope_key = scope_key
        self.ops: list[object] = []

    def enqueue(self, op: object) -> None:
        self.ops.append(op)


@pytest.fixture
def writer():
    return _FakeWriter()


@pytest.fixture
def patch_active_writer(mocker, writer):
    """
    Patch ``active_writer`` on the directory_contents module (the
    name-bound import site) to return our fake. Tests that want
    "no active writer" patch it to ``None`` themselves.
    """
    mocker.patch.object(dc_module, "active_writer", return_value=writer)
    return writer


class TestAddFilePath:
    def test_adds_resource_to_the_list(self, dir_contents):
        dir_contents.add_file_path("b")
        assert dir_contents.resource_paths == {"a", "b"}

    def test_raises_error_if_not_authorized(self, dir_contents):
        dir_contents.authorized = False
        with pytest.raises(
            ServerError, match="Attempting to add a file to an unauthorized directory"
        ):
            dir_contents.add_file_path("b")

    def test_enqueues_add_file_op_when_writer_active(
        self, dir_contents_with_path, patch_active_writer
    ):
        dir_contents_with_path.add_file_path("b")

        assert patch_active_writer.ops == [
            redis_cache.add_file(scope_key=SCOPE_KEY, dir_path=DIR_PATH, path="b")
        ]

    def test_no_op_enqueued_when_dir_path_is_none(
        self, dir_contents, patch_active_writer
    ):
        dir_contents.add_file_path("b")

        assert patch_active_writer.ops == []

    def test_no_op_enqueued_when_no_active_writer(self, dir_contents_with_path, mocker):
        mocker.patch.object(dc_module, "active_writer", return_value=None)

        dir_contents_with_path.add_file_path("b")

    def test_no_op_enqueued_when_unauthorized_raises(
        self, dir_contents_with_path, patch_active_writer
    ):
        dir_contents_with_path.authorized = False

        with pytest.raises(ServerError):
            dir_contents_with_path.add_file_path("b")

        assert patch_active_writer.ops == []


class TestAddFolderPath:
    def test_adds_folder_to_the_list(self, dir_contents):
        dir_contents.add_folder_path("dir_b")
        assert dir_contents.folders == {"dir_a", "dir_b"}

    def test_raises_error_if_not_authorized(self, dir_contents):
        dir_contents.authorized = False
        with pytest.raises(
            ServerError, match="Attempting to add a dir to an unauthorized directory"
        ):
            dir_contents.add_folder_path("dir_b")

    def test_enqueues_add_folder_op_when_writer_active(
        self, dir_contents_with_path, patch_active_writer
    ):
        dir_contents_with_path.add_folder_path("dir_b")

        assert patch_active_writer.ops == [
            redis_cache.add_folder(scope_key=SCOPE_KEY, dir_path=DIR_PATH, path="dir_b")
        ]

    def test_no_op_enqueued_when_dir_path_is_none(
        self, dir_contents, patch_active_writer
    ):
        dir_contents.add_folder_path("dir_b")

        assert patch_active_writer.ops == []

    def test_no_op_enqueued_when_unauthorized_raises(
        self, dir_contents_with_path, patch_active_writer
    ):
        dir_contents_with_path.authorized = False

        with pytest.raises(ServerError):
            dir_contents_with_path.add_folder_path("dir_b")

        assert patch_active_writer.ops == []


class TestRemoveFolderPath:
    def test_removes_folder_from_the_list(self, dir_contents):
        dir_contents.remove_folder_path("dir_a")
        assert len(dir_contents.folders) == 0

    def test_raises_error_if_not_authorized(self, dir_contents):
        dir_contents.authorized = False
        with pytest.raises(
            ServerError,
            match="Attempting to remove a folder from an unauthorized directory",
        ):
            dir_contents.remove_folder_path("dir_a")

    def test_enqueues_remove_folder_op_when_writer_active(
        self, dir_contents_with_path, patch_active_writer
    ):
        dir_contents_with_path.remove_folder_path("dir_a")

        assert patch_active_writer.ops == [
            redis_cache.remove_folder(
                scope_key=SCOPE_KEY, dir_path=DIR_PATH, path="dir_a"
            )
        ]

    def test_enqueues_remove_folder_op_even_when_path_was_already_gone(
        self, dir_contents_with_path, patch_active_writer
    ):
        # SREM is idempotent; the convergence Op must still emit so
        # Redis matches in-memory even if we lost a race in another pod.
        dir_contents_with_path.remove_folder_path("not_present")

        assert patch_active_writer.ops == [
            redis_cache.remove_folder(
                scope_key=SCOPE_KEY, dir_path=DIR_PATH, path="not_present"
            )
        ]

    def test_no_op_enqueued_when_dir_path_is_none(
        self, dir_contents, patch_active_writer
    ):
        dir_contents.remove_folder_path("dir_a")

        assert patch_active_writer.ops == []

    def test_no_op_enqueued_when_unauthorized_raises(
        self, dir_contents_with_path, patch_active_writer
    ):
        dir_contents_with_path.authorized = False

        with pytest.raises(ServerError):
            dir_contents_with_path.remove_folder_path("dir_a")

        assert patch_active_writer.ops == []


class TestRemoveFilePath:
    def test_removes_path_from_resource_paths(self, dir_contents_with_path):
        dir_contents_with_path.remove_file_path("a")
        assert "a" not in dir_contents_with_path.resource_paths

    def test_raises_error_if_not_authorized(self, dir_contents):
        dir_contents.authorized = False
        with pytest.raises(
            ServerError,
            match="Attempting to remove a file from an unauthorized directory",
        ):
            dir_contents.remove_file_path("a")

    def test_enqueues_remove_file_op_when_writer_active(
        self, dir_contents_with_path, patch_active_writer
    ):
        dir_contents_with_path.remove_file_path("a")

        assert patch_active_writer.ops == [
            redis_cache.remove_file(scope_key=SCOPE_KEY, dir_path=DIR_PATH, path="a")
        ]

    def test_enqueues_remove_file_op_even_when_path_was_already_gone(
        self, dir_contents_with_path, patch_active_writer
    ):
        dir_contents_with_path.remove_file_path("not_present")

        assert patch_active_writer.ops == [
            redis_cache.remove_file(
                scope_key=SCOPE_KEY, dir_path=DIR_PATH, path="not_present"
            )
        ]

    def test_no_op_enqueued_when_dir_path_is_none(
        self, dir_contents, patch_active_writer
    ):
        dir_contents.remove_file_path("a")

        assert patch_active_writer.ops == []

    def test_no_op_enqueued_when_unauthorized_raises(
        self, dir_contents_with_path, patch_active_writer
    ):
        dir_contents_with_path.authorized = False

        with pytest.raises(ServerError):
            dir_contents_with_path.remove_file_path("a")

        assert patch_active_writer.ops == []


class TestAbsorbDisappearedPaths:
    def test_captures_paths_that_disappeared_from_resource_paths(self, freezer):
        old = DirectoryContents(file_paths=["a", "b", "c"], folder_paths=[])
        new = DirectoryContents(file_paths=["a"], folder_paths=[])

        new.absorb_disappeared_paths(old)

        assert "b" in new.previously_visible_paths
        assert "c" in new.previously_visible_paths
        assert "a" not in new.previously_visible_paths

    def test_sets_ttl_on_newly_disappeared_paths(self, freezer, mocker):
        mocker.patch.object(
            _settings_module.settings,
            "previously_visible_paths_ttl_seconds",
            42,
        )
        old = DirectoryContents(file_paths=["a", "b"], folder_paths=[])
        new = DirectoryContents(file_paths=[], folder_paths=[])

        new.absorb_disappeared_paths(old)

        expected_expiry = time.time() + 42
        assert new.previously_visible_paths["a"] == expected_expiry
        assert new.previously_visible_paths["b"] == expected_expiry

    def test_carries_forward_unexpired_previously_visible_paths(self, freezer):
        old = DirectoryContents(file_paths=["a"], folder_paths=[])
        future_expiry = time.time() + 100
        old.previously_visible_paths = {"x": future_expiry}

        new = DirectoryContents(file_paths=["a"], folder_paths=[])
        new.absorb_disappeared_paths(old)

        assert new.previously_visible_paths["x"] == future_expiry

    def test_drops_expired_previously_visible_paths(self, freezer):
        old = DirectoryContents(file_paths=["a"], folder_paths=[])
        old.previously_visible_paths = {"x": time.time() - 1}

        new = DirectoryContents(file_paths=["a"], folder_paths=[])
        new.absorb_disappeared_paths(old)

        assert "x" not in new.previously_visible_paths

    def test_does_not_carry_forward_paths_now_in_resource_paths(self, freezer):
        old = DirectoryContents(file_paths=[], folder_paths=[])
        old.previously_visible_paths = {"a": time.time() + 100}

        new = DirectoryContents(file_paths=["a"], folder_paths=[])
        new.absorb_disappeared_paths(old)

        assert "a" not in new.previously_visible_paths

    def test_newly_disappeared_overrides_carried_forward_ttl(self, freezer, mocker):
        mocker.patch.object(
            _settings_module.settings,
            "previously_visible_paths_ttl_seconds",
            200,
        )
        old_carried_expiry = time.time() + 50
        old = DirectoryContents(file_paths=["b"], folder_paths=[])
        old.previously_visible_paths = {"b": old_carried_expiry}

        new = DirectoryContents(file_paths=[], folder_paths=[])
        new.absorb_disappeared_paths(old)

        fresh_expiry = time.time() + 200
        assert new.previously_visible_paths["b"] == fresh_expiry

    def test_handles_old_with_no_resource_paths(self, freezer):
        old = DirectoryContents()
        new = DirectoryContents(file_paths=["a"], folder_paths=[])
        new.absorb_disappeared_paths(old)

        assert new.previously_visible_paths == {}

    def test_enqueues_single_set_prev_op_with_union_of_added_entries(
        self, freezer, mocker, patch_active_writer
    ):
        mocker.patch.object(
            _settings_module.settings,
            "previously_visible_paths_ttl_seconds",
            42,
        )
        carried_expiry = time.time() + 100
        old = DirectoryContents(file_paths=["a", "b"], folder_paths=[])
        old.previously_visible_paths = {"x": carried_expiry}

        new = DirectoryContents(file_paths=[], folder_paths=[], dir_path=DIR_PATH)
        new.absorb_disappeared_paths(old)

        fresh_expiry = time.time() + 42
        assert patch_active_writer.ops == [
            redis_cache.set_prev(
                scope_key=SCOPE_KEY,
                dir_path=DIR_PATH,
                entries={
                    "x": carried_expiry,
                    "a": fresh_expiry,
                    "b": fresh_expiry,
                },
            )
        ]

    def test_no_op_enqueued_when_nothing_added(self, freezer, patch_active_writer):
        # Old has resource_paths == {"a"} and new also has {"a"} —
        # nothing disappears and there are no carried-forward prev paths.
        old = DirectoryContents(file_paths=["a"], folder_paths=[])
        new = DirectoryContents(file_paths=["a"], folder_paths=[], dir_path=DIR_PATH)

        new.absorb_disappeared_paths(old)

        assert patch_active_writer.ops == []

    def test_no_op_enqueued_when_dir_path_is_none(self, freezer, patch_active_writer):
        old = DirectoryContents(file_paths=["a", "b"], folder_paths=[])
        new = DirectoryContents(file_paths=[], folder_paths=[])  # no dir_path

        new.absorb_disappeared_paths(old)

        assert patch_active_writer.ops == []


class TestConsumePreviouslyVisiblePath:
    def test_returns_true_and_removes_unexpired_path(self, freezer):
        dc = DirectoryContents(file_paths=[], folder_paths=[])
        dc.previously_visible_paths = {"a": time.time() + 100}

        assert dc.consume_previously_visible_path("a") is True
        assert "a" not in dc.previously_visible_paths

    def test_returns_false_for_expired_path(self, freezer):
        dc = DirectoryContents(file_paths=[], folder_paths=[])
        dc.previously_visible_paths = {"a": time.time() - 1}

        assert dc.consume_previously_visible_path("a") is False
        assert "a" not in dc.previously_visible_paths

    def test_returns_false_for_unknown_path(self, freezer):
        dc = DirectoryContents(file_paths=[], folder_paths=[])

        assert dc.consume_previously_visible_path("nope") is False

    def test_does_not_affect_other_paths(self, freezer):
        dc = DirectoryContents(file_paths=[], folder_paths=[])
        future = time.time() + 100
        dc.previously_visible_paths = {
            "a": future,
            "b": future,
        }

        dc.consume_previously_visible_path("a")

        assert "b" in dc.previously_visible_paths

    def test_enqueues_del_prev_op_on_consumed_path(self, freezer, patch_active_writer):
        dc = DirectoryContents(file_paths=[], folder_paths=[], dir_path=DIR_PATH)
        dc.previously_visible_paths = {"a": time.time() + 100}

        dc.consume_previously_visible_path("a")

        assert patch_active_writer.ops == [
            redis_cache.del_prev(scope_key=SCOPE_KEY, dir_path=DIR_PATH, path="a")
        ]

    def test_enqueues_del_prev_op_on_expired_path(self, freezer, patch_active_writer):
        dc = DirectoryContents(file_paths=[], folder_paths=[], dir_path=DIR_PATH)
        dc.previously_visible_paths = {"a": time.time() - 1}

        dc.consume_previously_visible_path("a")

        assert patch_active_writer.ops == [
            redis_cache.del_prev(scope_key=SCOPE_KEY, dir_path=DIR_PATH, path="a")
        ]

    def test_no_op_enqueued_for_path_not_in_dict(self, freezer, patch_active_writer):
        dc = DirectoryContents(file_paths=[], folder_paths=[], dir_path=DIR_PATH)

        dc.consume_previously_visible_path("nope")

        assert patch_active_writer.ops == []

    def test_no_op_enqueued_when_dir_path_is_none(self, freezer, patch_active_writer):
        dc = DirectoryContents(file_paths=[], folder_paths=[])
        dc.previously_visible_paths = {"a": time.time() + 100}

        dc.consume_previously_visible_path("a")

        assert patch_active_writer.ops == []


class TestExpiresAt:
    def test_authorized_uses_full_permissions_ttl(self, freezer):
        dc = DirectoryContents(file_paths=[], folder_paths=[])
        expected = time.time() + _settings_module.settings.permissions_ttl_seconds
        assert dc.expires_at == expected

    def test_unauthorized_expires_immediately(self, freezer):
        dc = DirectoryContents()
        assert dc.expires_at == 0

    def test_unauthorized_is_expired_at_creation(self, freezer):
        dc = DirectoryContents()
        assert dc.expired() is True


class TestListingOutcomeStorageStatus:
    @pytest.mark.parametrize(
        "outcome, status",
        [
            (ListingOutcome.LISTED, 200),
            (ListingOutcome.EMPTY_LISTING, 404),
            (ListingOutcome.FORBIDDEN, 403),
            (ListingOutcome.UNSUPPORTED_ADDRESS, 400),
            (ListingOutcome.UNRECOGNIZED_NOT_FOUND, 404),
        ],
    )
    def test_is_the_status_the_outcome_was_classified_from(self, outcome, status):
        assert outcome.storage_status == status

    @pytest.mark.parametrize("outcome", list(ListingOutcome))
    def test_every_outcome_has_a_status(self, outcome):
        # The unauthorized-listing log line reads this on the event path, so
        # an outcome added without a status would raise there.
        assert isinstance(outcome.storage_status, int)


class TestExpired:
    def test_returns_true_if_expired(self, dir_contents):
        dir_contents.expires_at = time.time() - 1
        assert dir_contents.expired() is True

    def test_returns_false_if_not_expired(self, dir_contents):
        dir_contents.expires_at = time.time() + 1
        assert dir_contents.expired() is False
