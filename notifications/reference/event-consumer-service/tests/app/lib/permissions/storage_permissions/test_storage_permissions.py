# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import logging
import random
import time
import urllib.parse
from unittest.mock import AsyncMock

import pytest
import pytest_asyncio
from notification_common.config.common_settings import common_settings
from notification_common.errors.errors import ClientFacingServerError
from notification_common.http_client.http_client import http_client

from app.config import settings as _settings_module
from app.lib.permissions.storage_permissions import directory_contents as dc_module
from app.lib.permissions.storage_permissions import redis_cache
from app.lib.permissions.storage_permissions import storage_permissions
from app.lib.permissions.storage_permissions import storage_permissions_cache
from app.lib.permissions.storage_permissions.directory_contents import (
    DirectoryContents,
    ListingOutcome,
)

# noinspection PyProtectedMember
from app.lib.permissions.storage_permissions.storage_permissions import (
    can_access_deleted_event,
    can_access_created_event,
    can_access_dir_created_event,
    can_access_dir_deleted_event,
    initialize,
    _can_access_dir,
    _dir_was_not_deleted,
    _ensure_trailing_slash,
    _resource_path,
)
from tests.fixtures.cache_scope import CACHE_SCOPE, CACHE_SCOPE_KEY
from tests.fixtures.mock_filter_group import filter_group
from tests.fixtures.mock_http_response import MockHttpResponse
from tests.fixtures.storage_listing_bodies import AZURE_EMPTY_LISTING_BODY


@pytest.fixture(autouse=True)
def enable_storage_permissions(mocker):
    mocker.patch.object(
        storage_permissions.settings, "storage_permissions_enabled", True
    )


@pytest.fixture
def rabbitmq_message() -> dict:
    return {
        "message": {"resource_address": "test-address"},
    }


@pytest.fixture
def authorized_dir_contents() -> DirectoryContents:
    return DirectoryContents(file_paths=[], folder_paths=[])


@pytest.fixture
def unauthorized_dir_contents() -> DirectoryContents:
    return DirectoryContents()


@pytest.fixture
def mock_authorized(mocker, authorized_dir_contents):
    mocker.patch.object(
        storage_permissions.cache,
        "get_parent_resource",
        return_value=authorized_dir_contents,
    )


@pytest.fixture
def mock_unauthorized(mocker, unauthorized_dir_contents):
    mocker.patch.object(
        storage_permissions.cache,
        "get_parent_resource",
        return_value=unauthorized_dir_contents,
    )


@pytest.fixture
def mock_can_access_dir(mocker):
    return mocker.patch.object(storage_permissions, "_can_access_dir")


#: The outcomes that let the principal list the folder.
AUTHORIZED_OUTCOMES = [ListingOutcome.LISTED, ListingOutcome.EMPTY_LISTING]

#: The outcomes that do not.
UNAUTHORIZED_OUTCOMES = [
    ListingOutcome.FORBIDDEN,
    ListingOutcome.UNSUPPORTED_ADDRESS,
    ListingOutcome.UNRECOGNIZED_NOT_FOUND,
]


def listing(outcome: ListingOutcome) -> DirectoryContents:
    """
    The ``DirectoryContents`` ``fetch_from_storage_api`` returns for an
    ``outcome``: empty sets for an authorized listing, ``None`` otherwise,
    and the outcome itself, which ``_dir_was_not_deleted`` keys on.
    """
    if outcome in AUTHORIZED_OUTCOMES:
        return DirectoryContents(file_paths=[], folder_paths=[], outcome=outcome)
    return DirectoryContents(outcome=outcome)


@pytest.fixture
def mock_direct_fetch(mocker) -> AsyncMock:
    """The uncached listing ``_can_access_dir`` and ``_dir_was_not_deleted`` make."""
    return mocker.patch.object(storage_permissions, "fetch_from_storage_api")


@pytest.fixture
def spy_dir_was_not_deleted(mocker) -> AsyncMock:
    return mocker.patch.object(storage_permissions, "_dir_was_not_deleted")


@pytest.fixture
def redis_hydrated_unauthorized_dir_contents() -> DirectoryContents:
    """
    An unauthorized entry as Redis hydration builds it. ``from_redis`` skips
    ``__init__``, so the entry has ``outcome`` and ``dropped_events`` only
    because ``from_redis`` sets them itself.
    """
    return DirectoryContents.from_redis(
        authorized=False,
        files=None,
        folders=None,
        expires_at=time.time() + 30,
        prev={},
        dir_path="/parent/dir/",
    )


@pytest.mark.asyncio
class TestInitialize:
    async def test_calls_init_cache_with_all_storage_resource_filters(self, mocker):
        storage_type_groups = [
            filter_group(filter_count=2),
            filter_group(filter_count=2),
        ]

        non_storage_groups = [
            filter_group(event_type="et"),
            filter_group(event_type="et"),
        ]

        mock_init_cache = mocker.patch.object(
            storage_permissions.cache, "init_cache_for_user"
        )

        filter_groups = storage_type_groups + non_storage_groups
        random.shuffle(filter_groups)

        await storage_permissions.initialize(CACHE_SCOPE, filter_groups)

        mock_init_cache.assert_called_once()
        mock_init_cache_args = mock_init_cache.call_args[0][1]

        assert len(mock_init_cache_args) == 4
        assert storage_type_groups[0].filters[0] in mock_init_cache_args
        assert storage_type_groups[0].filters[1] in mock_init_cache_args
        assert storage_type_groups[1].filters[0] in mock_init_cache_args
        assert storage_type_groups[1].filters[1] in mock_init_cache_args

    async def test_forwards_the_scope_unchanged(self, mocker):
        # The scope is what namespaces every key the cache writes, so it
        # has to reach the cache layer as the caller's own object -- not a
        # queue name re-derived here.
        mock_init_cache = mocker.patch.object(
            storage_permissions.cache, "init_cache_for_user"
        )

        await storage_permissions.initialize(CACHE_SCOPE, [filter_group()])

        assert mock_init_cache.call_args[0][0] is CACHE_SCOPE


@pytest.mark.asyncio
class TestCanAccessCreatedEvent:
    async def test_gets_parent_resource_for_event_resource(
        self, rabbitmq_message, mocker
    ):
        spy = mocker.patch.object(
            storage_permissions.cache,
            "get_parent_resource",
            return_value=DirectoryContents(),
        )
        await can_access_created_event(rabbitmq_message)
        spy.assert_called_once_with("test-address")

    class TestWhenAuthorizedForParentResource:
        @pytest.fixture(autouse=True)
        def mock_authorization(self, mock_authorized):
            pass

        async def test_returns_true(self, rabbitmq_message):
            assert await can_access_created_event(rabbitmq_message) is True

        async def test_adds_file_path_to_parent_resource(
            self, rabbitmq_message, authorized_dir_contents, mocker
        ):
            spy = mocker.patch.object(authorized_dir_contents, "add_file_path")
            await can_access_created_event(rabbitmq_message)
            spy.assert_called_once_with("test-address")

    class TestWhenNotAuthorizedForParentResource:
        @pytest.fixture(autouse=True)
        def setup_unauthorized(self, mock_unauthorized):
            pass

        async def test_returns_false(self, rabbitmq_message):
            assert await can_access_created_event(rabbitmq_message) is False

        async def test_doesnt_add_file_path_to_parent_resource(
            self, rabbitmq_message, unauthorized_dir_contents, mocker
        ):
            spy = mocker.patch.object(unauthorized_dir_contents, "add_file_path")
            await can_access_created_event(rabbitmq_message)
            spy.assert_not_called()

        async def test_counts_the_drop_against_the_parent_entry(
            self, rabbitmq_message, unauthorized_dir_contents
        ):
            await can_access_created_event(rabbitmq_message)
            assert unauthorized_dir_contents.dropped_events == 1

    async def test_does_not_count_a_delivered_event_as_a_drop(
        self, rabbitmq_message, authorized_dir_contents, mock_authorized
    ):
        await can_access_created_event(rabbitmq_message)
        assert authorized_dir_contents.dropped_events == 0


@pytest.mark.asyncio
class TestCanAccessDeletedEvent:
    async def test_gets_parent_resource_for_event_resource(
        self, rabbitmq_message, mocker
    ):
        spy = mocker.patch.object(
            storage_permissions.cache,
            "get_parent_resource",
            return_value=DirectoryContents(),
        )
        await can_access_deleted_event(rabbitmq_message)
        spy.assert_called_once_with("test-address")

    async def test_returns_true_when_path_in_resource_paths(
        self, rabbitmq_message, authorized_dir_contents, mocker
    ):
        authorized_dir_contents.resource_paths = {"test-address"}

        mocker.patch.object(
            storage_permissions.cache,
            "get_parent_resource",
            return_value=authorized_dir_contents,
        )

        assert await can_access_deleted_event(rabbitmq_message) is True

    async def test_removes_path_from_resource_paths_on_delete(
        self, rabbitmq_message, authorized_dir_contents, mocker
    ):
        authorized_dir_contents.resource_paths = {
            "test-address",
            "other-file",
        }

        mocker.patch.object(
            storage_permissions.cache,
            "get_parent_resource",
            return_value=authorized_dir_contents,
        )

        await can_access_deleted_event(rabbitmq_message)

        assert "test-address" not in authorized_dir_contents.resource_paths
        assert "other-file" in authorized_dir_contents.resource_paths

    async def test_returns_false_when_not_authorized_for_parent_resource(
        self, rabbitmq_message, unauthorized_dir_contents, mocker
    ):
        mocker.patch.object(
            storage_permissions.cache,
            "get_parent_resource",
            return_value=unauthorized_dir_contents,
        )
        assert await can_access_deleted_event(rabbitmq_message) is False

    async def test_counts_the_drop_when_not_authorized_for_parent_resource(
        self, rabbitmq_message, unauthorized_dir_contents, mock_unauthorized
    ):
        await can_access_deleted_event(rabbitmq_message)
        assert unauthorized_dir_contents.dropped_events == 1

    async def test_returns_true_when_path_in_previously_visible_paths(
        self, rabbitmq_message, authorized_dir_contents, mocker
    ):
        mocker.patch.object(
            storage_permissions.cache,
            "get_parent_resource",
            return_value=authorized_dir_contents,
        )
        mocker.patch.object(
            authorized_dir_contents,
            "consume_previously_visible_path",
            return_value=True,
        )

        assert await can_access_deleted_event(rabbitmq_message) is True

    async def test_consumes_previously_visible_path(
        self, rabbitmq_message, authorized_dir_contents, mocker
    ):
        mocker.patch.object(
            storage_permissions.cache,
            "get_parent_resource",
            return_value=authorized_dir_contents,
        )
        spy = mocker.patch.object(
            authorized_dir_contents,
            "consume_previously_visible_path",
            return_value=True,
        )

        await can_access_deleted_event(rabbitmq_message)
        spy.assert_called_once_with("test-address")

    async def test_returns_false_when_path_not_in_any_set(
        self, rabbitmq_message, authorized_dir_contents, mocker
    ):
        mocker.patch.object(
            storage_permissions.cache,
            "get_parent_resource",
            return_value=authorized_dir_contents,
        )

        assert await can_access_deleted_event(rabbitmq_message) is False


@pytest.mark.asyncio
class TestCanAccessDirCreatedEvent:
    async def test_gets_parent_resource_for_event_resource(
        self, rabbitmq_message, mock_can_access_dir, mocker
    ):
        mock_can_access_dir.return_value = False
        spy = mocker.patch.object(storage_permissions.cache, "get_parent_resource")
        await can_access_dir_created_event(rabbitmq_message)
        spy.assert_called_once_with("test-address")

    class TestWhenNotAuthorizedForParentResource:
        @pytest.fixture(autouse=True)
        def setup_unauthorized(self, mock_unauthorized):
            pass

        async def test_returns_false(self, rabbitmq_message):
            assert await can_access_dir_created_event(rabbitmq_message) is False

        async def test_doesnt_check_with_api_for_access(
            self, rabbitmq_message, unauthorized_dir_contents, mock_can_access_dir
        ):
            await can_access_dir_created_event(rabbitmq_message)
            mock_can_access_dir.assert_not_called()

        async def test_doesnt_add_folder_path_to_parent_resource(
            self, rabbitmq_message, unauthorized_dir_contents, mocker
        ):
            spy = mocker.patch.object(unauthorized_dir_contents, "add_folder_path")
            await can_access_dir_created_event(rabbitmq_message)
            spy.assert_not_called()

        async def test_counts_the_drop_against_the_parent_entry(
            self, rabbitmq_message, unauthorized_dir_contents
        ):
            await can_access_dir_created_event(rabbitmq_message)
            assert unauthorized_dir_contents.dropped_events == 1

    class TestWhenTheNewFolderListsAs:
        @pytest.fixture(autouse=True)
        def mock_authorization(self, mock_authorized):
            pass

        @pytest.mark.parametrize("outcome", AUTHORIZED_OUTCOMES)
        async def test_an_authorized_listing_delivers_the_event(
            self, outcome, rabbitmq_message, mock_direct_fetch
        ):
            # An empty-listing 404 for the new folder is the storage API's
            # answer to a principal that may list it.
            mock_direct_fetch.return_value = listing(outcome)
            assert await can_access_dir_created_event(rabbitmq_message) is True

        @pytest.mark.parametrize("outcome", UNAUTHORIZED_OUTCOMES)
        async def test_an_unauthorized_listing_drops_the_event(
            self, outcome, rabbitmq_message, mock_direct_fetch
        ):
            mock_direct_fetch.return_value = listing(outcome)
            assert await can_access_dir_created_event(rabbitmq_message) is False

    class TestWhenAuthorizedForParentResource:
        @pytest.fixture(autouse=True)
        def mock_authorization(self, mock_authorized):
            pass

        async def test_returns_false_when_the_dir_is_already_cached(
            self, authorized_dir_contents, rabbitmq_message, mock_can_access_dir
        ):
            authorized_dir_contents.add_folder_path("test-address")
            assert await can_access_dir_created_event(rabbitmq_message) is False
            mock_can_access_dir.assert_not_called()

        class TestWhenAuthorizedForTheDir:
            @pytest.fixture(autouse=True)
            def mock_cached_dir_results(self, mock_can_access_dir):
                mock_can_access_dir.return_value = True

            async def test_returns_true(self, rabbitmq_message):
                assert await can_access_dir_created_event(rabbitmq_message) is True

            async def test_adds_folder_path_to_parent_resource(
                self, rabbitmq_message, authorized_dir_contents, mocker
            ):
                spy = mocker.patch.object(authorized_dir_contents, "add_folder_path")
                await can_access_dir_created_event(rabbitmq_message)
                spy.assert_called_once_with("test-address")

        class TestWhenNotAuthorizedForTheDir:
            @pytest.fixture(autouse=True)
            def mock_cached_dir_results(self, mock_can_access_dir):
                mock_can_access_dir.return_value = False

            async def test_returns_false(self, rabbitmq_message):
                assert await can_access_dir_created_event(rabbitmq_message) is False

            async def test_doesnt_add_folder_path_to_parent_resource(
                self, rabbitmq_message, authorized_dir_contents, mocker
            ):
                spy = mocker.patch.object(authorized_dir_contents, "add_folder_path")
                await can_access_dir_created_event(rabbitmq_message)
                spy.assert_not_called()


@pytest.mark.asyncio
class TestCanAccessDirDeletedEvent:
    async def test_gets_parent_resource_for_event_resource(
        self, rabbitmq_message, mocker
    ):
        spy = mocker.patch.object(storage_permissions.cache, "get_parent_resource")
        await can_access_dir_deleted_event(rabbitmq_message)
        spy.assert_called_once_with("test-address")

    class TestWhenNotAuthorizedForParentResource:
        @pytest.fixture(autouse=True)
        def setup_unauthorized(self, mock_unauthorized):
            pass

        async def test_returns_false(self, rabbitmq_message):
            assert await can_access_dir_deleted_event(rabbitmq_message) is False

        async def test_doesnt_check_with_api_to_see_if_the_folder_was_really_deleted(
            self, rabbitmq_message, spy_dir_was_not_deleted
        ):
            await can_access_dir_deleted_event(rabbitmq_message)
            spy_dir_was_not_deleted.assert_not_called()

        async def test_counts_the_drop_against_the_parent_entry(
            self, rabbitmq_message, unauthorized_dir_contents
        ):
            await can_access_dir_deleted_event(rabbitmq_message)
            assert unauthorized_dir_contents.dropped_events == 1

    class TestWhenTheKnownFolderListsAs:
        """
        The deleted folder's own listing decides whether the delete is real.
        Only a 200 listing means it still holds blobs; an empty-listing 404 is
        authorized but means the folder is gone, so the delete is delivered.
        """

        @pytest.fixture(autouse=True)
        def known_folder(self, mock_authorized, authorized_dir_contents):
            authorized_dir_contents.folders.add("test-address")

        async def test_a_listed_folder_drops_the_synthetic_delete(
            self, rabbitmq_message, mock_direct_fetch
        ):
            mock_direct_fetch.return_value = listing(ListingOutcome.LISTED)
            assert await can_access_dir_deleted_event(rabbitmq_message) is False

        @pytest.mark.parametrize(
            "outcome", [ListingOutcome.EMPTY_LISTING, *UNAUTHORIZED_OUTCOMES]
        )
        async def test_any_other_listing_delivers_the_delete(
            self, outcome, rabbitmq_message, mock_direct_fetch
        ):
            mock_direct_fetch.return_value = listing(outcome)
            assert await can_access_dir_deleted_event(rabbitmq_message) is True

    class TestWhenAuthorizedForParentResource:
        @pytest.fixture(autouse=True)
        def mock_authorization(self, mock_authorized):
            pass

        class TestButNotAuthorizedForTheDir:
            async def test_returns_false(self, rabbitmq_message):
                assert await can_access_dir_deleted_event(rabbitmq_message) is False

            async def test_no_check_with_api_to_see_if_the_folder_was_really_deleted(
                self, rabbitmq_message, spy_dir_was_not_deleted
            ):
                await can_access_dir_deleted_event(rabbitmq_message)
                spy_dir_was_not_deleted.assert_not_called()

            async def test_doesnt_try_to_remove_folder_path_from_parent_resource_cache(
                self, rabbitmq_message, authorized_dir_contents, mocker
            ):
                spy = mocker.patch.object(authorized_dir_contents, "remove_folder_path")
                await can_access_dir_deleted_event(rabbitmq_message)
                spy.assert_not_called()

        class TestAndIsAuthorizedForDir:
            @pytest.fixture(autouse=True)
            def mock_cached_dir_results(self, authorized_dir_contents):
                authorized_dir_contents.folders.add("test-address")

            class TestWhenTheFolderWasActuallyDeleted:
                @pytest.fixture(autouse=True)
                def mock_dir_was_not_deleted(self, mocker):
                    mocker.patch.object(
                        storage_permissions, "_dir_was_not_deleted", return_value=False
                    )

                async def test_returns_true(self, rabbitmq_message):
                    assert await can_access_dir_deleted_event(rabbitmq_message) is True

                async def test_removes_the_folder_path_from_parent_resource_cache(
                    self, rabbitmq_message, authorized_dir_contents, mocker
                ):
                    spy = mocker.patch.object(
                        authorized_dir_contents, "remove_folder_path"
                    )
                    await can_access_dir_deleted_event(rabbitmq_message)
                    spy.assert_called_once_with("test-address")

            class TestWhenTheFolderWasNotActuallyDeleted:
                async def test_returns_false(self, rabbitmq_message, mocker):
                    mocker.patch.object(
                        storage_permissions, "_dir_was_not_deleted", return_value=True
                    )
                    assert await can_access_dir_deleted_event(rabbitmq_message) is False


#: Every reader that checks the event's parent directory first.
PARENT_DIR_READERS = [
    pytest.param(can_access_created_event, id="created"),
    pytest.param(can_access_deleted_event, id="deleted"),
    pytest.param(can_access_dir_created_event, id="dir-created"),
    pytest.param(can_access_dir_deleted_event, id="dir-deleted"),
]


@pytest.mark.asyncio
class TestReadersOnARedisHydratedUnauthorizedEntry:
    @pytest.fixture(autouse=True)
    def hydrated_parent(self, mocker, redis_hydrated_unauthorized_dir_contents):
        mocker.patch.object(
            storage_permissions.cache,
            "get_parent_resource",
            return_value=redis_hydrated_unauthorized_dir_contents,
        )

    @pytest.mark.parametrize("reader", PARENT_DIR_READERS)
    async def test_drops_the_event(self, reader, rabbitmq_message):
        assert await reader(rabbitmq_message) is False

    @pytest.mark.parametrize("reader", PARENT_DIR_READERS)
    async def test_counts_the_drop(
        self, reader, rabbitmq_message, redis_hydrated_unauthorized_dir_contents
    ):
        await reader(rabbitmq_message)
        assert redis_hydrated_unauthorized_dir_contents.dropped_events == 1


@pytest.mark.asyncio
class TestCanAccessDir:
    @pytest.fixture
    def mock_fetch(self, mocker) -> AsyncMock:
        return mocker.patch.object(
            storage_permissions,
            "fetch_from_storage_api",
            return_value=DirectoryContents(),
        )

    @pytest.mark.parametrize("address", ["address", "address/"])
    async def test_adds_trailing_slash_to_dir_path_if_missing(
        self, mock_fetch, address
    ):
        await _can_access_dir(address)
        mock_fetch.assert_called_once_with("address/", limit_to_one_result=True)

    @pytest.mark.parametrize("outcome", AUTHORIZED_OUTCOMES)
    async def test_true_for_a_listing_that_authorizes(self, mock_fetch, outcome):
        mock_fetch.return_value = listing(outcome)
        assert await _can_access_dir("") is True

    @pytest.mark.parametrize("outcome", UNAUTHORIZED_OUTCOMES)
    async def test_false_for_a_listing_that_does_not_authorize(
        self, mock_fetch, outcome
    ):
        mock_fetch.return_value = listing(outcome)
        assert await _can_access_dir("") is False


@pytest.mark.asyncio
class TestDirWasNotDeleted:
    @pytest.mark.parametrize("address", ["address", "address/"])
    async def test_lists_the_folder_with_a_trailing_slash_and_one_result(
        self, mock_direct_fetch, address
    ):
        mock_direct_fetch.return_value = listing(ListingOutcome.LISTED)
        await _dir_was_not_deleted(address)
        mock_direct_fetch.assert_called_once_with("address/", limit_to_one_result=True)

    async def test_true_only_for_a_200_listing(self, mock_direct_fetch):
        mock_direct_fetch.return_value = listing(ListingOutcome.LISTED)
        assert await _dir_was_not_deleted("") is True

    @pytest.mark.parametrize(
        "outcome", [ListingOutcome.EMPTY_LISTING, *UNAUTHORIZED_OUTCOMES]
    )
    async def test_false_for_any_other_listing(self, mock_direct_fetch, outcome):
        # An empty-listing 404 authorizes the folder (see _can_access_dir) but
        # means it holds no blobs, i.e. it really was deleted.
        mock_direct_fetch.return_value = listing(outcome)
        assert await _dir_was_not_deleted("") is False


class TestEnsureTrailingSlash:
    @pytest.mark.parametrize("original_value", ["value", "value/"])
    def test_adds_trailing_slash_if_missing(self, original_value):
        assert _ensure_trailing_slash(original_value) == "value/"


class TestResourcePath:
    def test_returns_resource_address_from_message(self, rabbitmq_message):
        result = _resource_path(rabbitmq_message)
        assert result == "test-address"

    def test_raises_error_when_resource_address_is_missing(self):
        message_without_resource_address = {"message": {}}
        with pytest.raises(
            ClientFacingServerError,
            match="Cannot process a storage event message that "
            "doesn't include a resource address",
        ):
            _resource_path(message_without_resource_address)


# --------------------------------------------------------------------------
# Storage-gate short-circuit (Epic .zi two-gate split)
# --------------------------------------------------------------------------


@pytest.mark.asyncio
class TestStoragePermissionsDisabled:
    """
    With the consumer-local ``storage_permissions_enabled`` gate OFF, every
    public reader short-circuits before touching the cache / storage API:
    the ``can_access_*`` readers return ``True`` and ``initialize`` returns
    early. The gate is keyed on the storage gate alone — independent of the
    permission-service gate ``common_settings.permissions_enabled``.
    """

    @pytest.fixture(autouse=True)
    def disable_storage_permissions(self, mocker):
        mocker.patch.object(
            storage_permissions.settings, "storage_permissions_enabled", False
        )

    @pytest.fixture
    def get_parent_resource_spy(self, mocker):
        return mocker.patch.object(storage_permissions.cache, "get_parent_resource")

    @pytest.fixture
    def fetch_spy(self, mocker):
        return mocker.patch.object(storage_permissions, "fetch_from_storage_api")

    @pytest.fixture
    def init_cache_spy(self, mocker):
        return mocker.patch.object(storage_permissions.cache, "init_cache_for_user")

    async def test_can_access_created_event_returns_true(self, rabbitmq_message):
        assert await can_access_created_event(rabbitmq_message) is True

    async def test_can_access_created_event_skips_cache(
        self, rabbitmq_message, get_parent_resource_spy
    ):
        await can_access_created_event(rabbitmq_message)
        get_parent_resource_spy.assert_not_called()

    async def test_can_access_deleted_event_returns_true(self, rabbitmq_message):
        assert await can_access_deleted_event(rabbitmq_message) is True

    async def test_can_access_deleted_event_skips_cache(
        self, rabbitmq_message, get_parent_resource_spy
    ):
        await can_access_deleted_event(rabbitmq_message)
        get_parent_resource_spy.assert_not_called()

    async def test_can_access_dir_created_event_returns_true(self, rabbitmq_message):
        assert await can_access_dir_created_event(rabbitmq_message) is True

    async def test_can_access_dir_created_event_skips_cache(
        self, rabbitmq_message, get_parent_resource_spy
    ):
        await can_access_dir_created_event(rabbitmq_message)
        get_parent_resource_spy.assert_not_called()

    async def test_can_access_dir_created_event_skips_storage_api(
        self, rabbitmq_message, fetch_spy
    ):
        await can_access_dir_created_event(rabbitmq_message)
        fetch_spy.assert_not_called()

    async def test_can_access_dir_deleted_event_returns_true(self, rabbitmq_message):
        assert await can_access_dir_deleted_event(rabbitmq_message) is True

    async def test_can_access_dir_deleted_event_skips_cache(
        self, rabbitmq_message, get_parent_resource_spy
    ):
        await can_access_dir_deleted_event(rabbitmq_message)
        get_parent_resource_spy.assert_not_called()

    async def test_can_access_dir_deleted_event_skips_storage_api(
        self, rabbitmq_message, fetch_spy
    ):
        await can_access_dir_deleted_event(rabbitmq_message)
        fetch_spy.assert_not_called()

    async def test_initialize_returns_early(self, init_cache_spy):
        await initialize(CACHE_SCOPE, [filter_group(filter_count=2)])
        init_cache_spy.assert_not_called()


@pytest.mark.asyncio
class TestStorageGateIndependentOfPermissionServiceGate:
    """
    The storage readers key on ``storage_permissions_enabled`` ONLY. Turning
    the permission-service gate (``common_settings.permissions_enabled``) ON
    must NOT re-enable the storage readers while the storage gate is OFF — this
    pins the by-code-path seam from the storage side. Gating these readers on
    ``permissions_enabled`` instead would make these tests fail.
    """

    @pytest.fixture(autouse=True)
    def storage_off_permission_service_on(self, mocker):
        mocker.patch.object(
            storage_permissions.settings, "storage_permissions_enabled", False
        )
        mocker.patch.object(common_settings, "permissions_enabled", True)

    @pytest.fixture
    def get_parent_resource_spy(self, mocker):
        return mocker.patch.object(storage_permissions.cache, "get_parent_resource")

    @pytest.fixture
    def init_cache_spy(self, mocker):
        return mocker.patch.object(storage_permissions.cache, "init_cache_for_user")

    async def test_can_access_created_event_still_short_circuits(
        self, rabbitmq_message, get_parent_resource_spy
    ):
        assert await can_access_created_event(rabbitmq_message) is True
        get_parent_resource_spy.assert_not_called()

    async def test_can_access_deleted_event_still_short_circuits(
        self, rabbitmq_message, get_parent_resource_spy
    ):
        assert await can_access_deleted_event(rabbitmq_message) is True
        get_parent_resource_spy.assert_not_called()

    async def test_can_access_dir_created_event_still_short_circuits(
        self, rabbitmq_message, get_parent_resource_spy
    ):
        assert await can_access_dir_created_event(rabbitmq_message) is True
        get_parent_resource_spy.assert_not_called()

    async def test_can_access_dir_deleted_event_still_short_circuits(
        self, rabbitmq_message, get_parent_resource_spy
    ):
        assert await can_access_dir_deleted_event(rabbitmq_message) is True
        get_parent_resource_spy.assert_not_called()

    async def test_initialize_still_returns_early(self, init_cache_spy):
        await initialize(CACHE_SCOPE, [filter_group(filter_count=2)])
        init_cache_spy.assert_not_called()


# --------------------------------------------------------------------------
# Per-mutation write-through Op enqueue (Bee oo / Epic .nd)
# --------------------------------------------------------------------------


#: What the writer hands the Op builders: the cache-scope key.
SCOPE_KEY = CACHE_SCOPE_KEY
DIR_PATH = "/parent/dir/"


class _FakeWriter:
    """
    Minimal stand-in matching the CacheWriter shape used by
    DirectoryContents mutators: ``scope_key`` + sync ``enqueue``.
    Records every Op for later assertions.
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
    mocker.patch.object(dc_module, "active_writer", return_value=writer)
    return writer


@pytest.fixture
def dir_contents_with_path():
    """
    DirectoryContents with a real ``dir_path`` so mutators emit Ops.
    Authorized + populated to mirror a freshly populated cache entry.
    """
    return DirectoryContents(
        file_paths=["test-address", "other-file"],
        folder_paths=["test-folder"],
        dir_path=DIR_PATH,
    )


@pytest.mark.asyncio
class TestCanAccessDeletedEventOpEnqueue:
    async def test_enqueues_one_remove_file_op_when_path_in_resource_paths(
        self, rabbitmq_message, dir_contents_with_path, patch_active_writer, mocker
    ):
        mocker.patch.object(
            storage_permissions.cache,
            "get_parent_resource",
            return_value=dir_contents_with_path,
        )

        await can_access_deleted_event(rabbitmq_message)

        assert patch_active_writer.ops == [
            redis_cache.remove_file(
                scope_key=SCOPE_KEY, dir_path=DIR_PATH, path="test-address"
            )
        ]

    async def test_no_op_when_unauthorized_for_parent(
        self,
        rabbitmq_message,
        unauthorized_dir_contents,
        patch_active_writer,
        mocker,
    ):
        mocker.patch.object(
            storage_permissions.cache,
            "get_parent_resource",
            return_value=unauthorized_dir_contents,
        )

        await can_access_deleted_event(rabbitmq_message)

        assert patch_active_writer.ops == []

    async def test_no_remove_file_op_on_consume_previously_visible_path_hit(
        self, rabbitmq_message, dir_contents_with_path, patch_active_writer, mocker
    ):
        # The path is NOT in resource_paths but IS in
        # previously_visible_paths — the consume() path runs and emits a
        # del_prev Op (engineer's `.uu` contract), not a remove_file.
        dir_contents_with_path.resource_paths = set()
        dir_contents_with_path.previously_visible_paths = {
            "test-address": float("inf"),
        }
        mocker.patch.object(
            storage_permissions.cache,
            "get_parent_resource",
            return_value=dir_contents_with_path,
        )

        await can_access_deleted_event(rabbitmq_message)

        assert all(
            not isinstance(op, redis_cache.remove_file)
            for op in patch_active_writer.ops
        )
        # Sanity: the consume branch's del_prev Op did emit.
        assert any(
            isinstance(op, redis_cache.del_prev) for op in patch_active_writer.ops
        )

    async def test_no_op_when_path_not_found_in_any_set(
        self, rabbitmq_message, authorized_dir_contents, patch_active_writer, mocker
    ):
        # Authorized parent, empty resource_paths, no previously-visible
        # path matching test-address.
        mocker.patch.object(
            storage_permissions.cache,
            "get_parent_resource",
            return_value=authorized_dir_contents,
        )

        await can_access_deleted_event(rabbitmq_message)

        assert patch_active_writer.ops == []


@pytest.mark.asyncio
class TestHotPathNoRedisAwait:
    """
    The auth-check hot path must never await a Redis call directly. The
    write-through pipeline runs in a separate background drainer task;
    enqueue is sync. We assert no awaits land on redis_client during
    can_access_*_event execution.
    """

    @pytest.fixture
    def redis_spy(self, mocker):
        from notification_common.redis import redis_client

        spy_client = mocker.AsyncMock()
        mocker.patch.object(redis_client, "redis_client", spy_client)
        return spy_client

    async def test_can_access_deleted_event_path_in_resource_paths(
        self,
        rabbitmq_message,
        dir_contents_with_path,
        patch_active_writer,
        redis_spy,
        mocker,
    ):
        mocker.patch.object(
            storage_permissions.cache,
            "get_parent_resource",
            return_value=dir_contents_with_path,
        )

        await can_access_deleted_event(rabbitmq_message)

        assert redis_spy.await_count == 0
        # writer.enqueue was hit (sync, no await).
        assert len(patch_active_writer.ops) == 1

    async def test_can_access_deleted_event_unauthorized(
        self,
        rabbitmq_message,
        unauthorized_dir_contents,
        patch_active_writer,
        redis_spy,
        mocker,
    ):
        mocker.patch.object(
            storage_permissions.cache,
            "get_parent_resource",
            return_value=unauthorized_dir_contents,
        )

        await can_access_deleted_event(rabbitmq_message)

        assert redis_spy.await_count == 0

    async def test_can_access_created_event(
        self,
        rabbitmq_message,
        dir_contents_with_path,
        patch_active_writer,
        redis_spy,
        mocker,
    ):
        mocker.patch.object(
            storage_permissions.cache,
            "get_parent_resource",
            return_value=dir_contents_with_path,
        )

        await can_access_created_event(rabbitmq_message)

        assert redis_spy.await_count == 0

    async def test_can_access_dir_deleted_event(
        self,
        rabbitmq_message,
        dir_contents_with_path,
        patch_active_writer,
        redis_spy,
        mocker,
    ):
        # Place the deleted path in folders so the function reaches the
        # remove_folder_path call (its in-memory mutator path).
        dir_contents_with_path.folders.add("test-address")
        mocker.patch.object(
            storage_permissions.cache,
            "get_parent_resource",
            return_value=dir_contents_with_path,
        )
        mocker.patch.object(
            storage_permissions, "_dir_was_not_deleted", return_value=False
        )

        await can_access_dir_deleted_event(rabbitmq_message)

        assert redis_spy.await_count == 0


# --------------------------------------------------------------------------
# The storage API's empty-listing 404, end to end (b.54z)
# --------------------------------------------------------------------------


#: A folder a client starts watching before any file in it lists.
NEW_DIR = "/tenant-x/project/new-folder/"
NEW_FILE = f"{NEW_DIR}file.usd"
SUB_DIR = f"{NEW_DIR}sub"

#: The message of the line that records a listing which left a directory
#: unauthorized.
UNAUTHORIZED_LISTING_MESSAGE = (
    "storage listing did not authorize a directory; "
    "its storage events are dropped until the recheck"
)

_LIST_ENDPOINT = "/v1beta/filefolder/list/"


def _event(resource_address: str) -> dict:
    return {"message": {"resource_address": resource_address}}


class _FakeStorageApi:
    """
    Stands in for the HTTP session's ``get`` against the folder-listing
    endpoint. Each folder answers with its queued responses in order, and
    the last one repeats; ``listed`` records every folder listed.
    """

    def __init__(self) -> None:
        self.answers: dict[str, list[MockHttpResponse]] = {}
        self.listed: list[str] = []

    def answer(self, folder, status, body="test-msg", files=(), folders=()):
        listing_json = {
            "sub_resource_addresses": list(files),
            "subfolder_addresses": list(folders),
        }
        response = MockHttpResponse(
            json_response=listing_json, status=status, text=body
        )
        self.answers.setdefault(folder, []).append(response)

    async def get(self, url, headers):
        encoded = url.split(_LIST_ENDPOINT, 1)[1].split("?", 1)[0]
        folder = urllib.parse.unquote(encoded)
        self.listed.append(folder)
        queued = self.answers[folder]
        return queued.pop(0) if len(queued) > 1 else queued[0]


@pytest_asyncio.fixture
async def storage_api(mocker, cache_instance):
    mocker.patch.object(
        _settings_module, "storage_endpoint", return_value="storage-endpoint/"
    )
    await http_client.open()
    fake = _FakeStorageApi()
    mocker.patch.object(http_client.session, "get", side_effect=fake.get)

    yield fake

    await http_client.close()


async def _watch_new_dir():
    """Open the stream's storage cache on ``NEW_DIR``, as a connect does."""
    await initialize(CACHE_SCOPE, [filter_group(resource_id=NEW_DIR)])


@pytest.mark.asyncio
class TestEmptyListingEndToEnd:
    """
    The storage API answers a folder that holds no blobs yet, or whose
    listing lags a write, with a 404 whose detail reads "No elements found".
    It sends that only to a principal that passed the folder's List check,
    so the folder's storage events reach the stream.
    """

    async def test_created_event_in_a_folder_that_listed_empty_is_delivered(
        self, storage_api
    ):
        storage_api.answer(NEW_DIR, 404, AZURE_EMPTY_LISTING_BODY)
        await _watch_new_dir()

        assert await can_access_created_event(_event(NEW_FILE)) is True

    async def test_created_event_in_a_folder_that_listed_empty_lists_it_once(
        self, storage_api
    ):
        storage_api.answer(NEW_DIR, 404, AZURE_EMPTY_LISTING_BODY)
        await _watch_new_dir()

        await can_access_created_event(_event(NEW_FILE))

        assert storage_api.listed == [NEW_DIR]

    async def test_delete_of_the_last_file_is_delivered_after_the_entry_expires(
        self, storage_api, freezer
    ):
        storage_api.answer(NEW_DIR, 200, files=[NEW_FILE])
        storage_api.answer(NEW_DIR, 404, AZURE_EMPTY_LISTING_BODY)
        await _watch_new_dir()
        freezer.tick(_settings_module.settings.permissions_ttl_seconds + 1)

        assert await can_access_deleted_event(_event(NEW_FILE)) is True

    async def test_dir_created_event_for_a_folder_that_lists_empty_is_delivered(
        self, storage_api
    ):
        storage_api.answer(NEW_DIR, 200)
        storage_api.answer(f"{SUB_DIR}/", 404, AZURE_EMPTY_LISTING_BODY)
        await _watch_new_dir()

        assert await can_access_dir_created_event(_event(SUB_DIR)) is True

    async def test_dir_deleted_event_for_a_folder_that_lists_empty_is_delivered(
        self, storage_api
    ):
        storage_api.answer(NEW_DIR, 200, folders=[SUB_DIR])
        storage_api.answer(f"{SUB_DIR}/", 404, AZURE_EMPTY_LISTING_BODY)
        await _watch_new_dir()

        assert await can_access_dir_deleted_event(_event(SUB_DIR)) is True

    async def test_the_recheck_line_counts_the_events_dropped_in_the_window(
        self, storage_api, freezer, caplog
    ):
        # The first event rechecks the 403 initialize() left and opens the
        # window; it and the next two are dropped against that entry. The
        # recheck after the window reports all three.
        storage_api.answer(NEW_DIR, 403)
        await _watch_new_dir()
        await can_access_created_event(_event(NEW_FILE))
        await can_access_created_event(_event(NEW_FILE))
        await can_access_created_event(_event(NEW_FILE))
        freezer.tick(_settings_module.settings.unauthorized_recheck_seconds + 1)
        caplog.clear()

        with caplog.at_level(
            logging.INFO, logger=storage_permissions_cache.logger.name
        ):
            await can_access_created_event(_event(NEW_FILE))

        [record] = [
            r for r in caplog.records if r.getMessage() == UNAUTHORIZED_LISTING_MESSAGE
        ]
        assert record.dropped_events == 3
