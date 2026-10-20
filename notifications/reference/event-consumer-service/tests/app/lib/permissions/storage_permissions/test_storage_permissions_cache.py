# SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import asyncio
import json
import logging
import time
from unittest.mock import call, AsyncMock

import pytest
import pytest_asyncio
from notification_common.errors.errors import ServerError, AuthenticationError
from notification_common.http_client.http_client import http_client

from app.config import settings as _settings_module
from app.lib.models.filter_type import FilterType
from app.lib.models.resource_filter import ResourceFilter
from app.lib.permissions.storage_permissions import redis_cache
from app.lib.permissions.storage_permissions import storage_permissions_cache
from app.lib.permissions.storage_permissions.directory_contents import (
    DirectoryContents,
    ListingOutcome,
)
from app.lib.permissions.storage_permissions.storage_cache import StorageCache
from app.lib.queue_names import redact_reconnect_token

# noinspection PyProtectedMember
from app.lib.permissions.storage_permissions.storage_permissions_cache import (
    init_cache_for_user,
    get_parent_resource,
    _parent_folder_path,
    _cache_key,
    _fetch_dir_contents,
    fetch_from_storage_api,
    _build_storage_api_url,
)
from tests.fixtures.cache_scope import CACHE_SCOPE, CACHE_SCOPE_KEY
from tests.fixtures.dropped_storage_events import (
    DROP_REPORT_MESSAGE,
    DROPPED,
    unauthorized_entry_with_drops,
)
from tests.fixtures.mock_http_response import MockHttpResponse
from tests.fixtures.storage_listing_bodies import (
    AZURE_EMPTY_LISTING_BODY,
    EMPTY_LISTING_BODIES,
    EMPTY_LISTING_IDS,
    UNRECOGNIZED_NOT_FOUND_BODIES,
    UNRECOGNIZED_NOT_FOUND_IDS,
    UNSUPPORTED_ADDRESS_BODY,
)

# The queues that hydrate storage permissions are non-durable ones, and a
# non-durable broker name embeds the stream's reconnect token -- the queue's
# identifier. The Redis keys keep the exact name (inside the scope key); the
# log lines must not.
from tests.fixtures.reconnect_token import RECONNECT_TOKEN


def filters(second_filter_resource_id="b/") -> list[ResourceFilter]:
    return [
        ResourceFilter(filter_type=FilterType.STARTS_WITH_LAZY, resource_id="a/"),
        ResourceFilter(
            filter_type=FilterType.STARTS_WITH_LAZY,
            resource_id=second_filter_resource_id,
        ),
    ]


@pytest.fixture
def mock_fetch(mocker) -> AsyncMock:
    return mocker.patch.object(
        storage_permissions_cache, "_fetch_dir_contents", return_value="fetch-response"
    )


@pytest.fixture
def mock_api_fetch(mocker) -> AsyncMock:
    return mocker.patch.object(
        storage_permissions_cache,
        "fetch_from_storage_api",
        return_value=DirectoryContents(),
    )


@pytest.fixture(autouse=True)
def setup_mocks(mock_fetch, mock_api_fetch, cache_instance):
    yield


@pytest.mark.asyncio
class TestInitCacheForUser:
    async def test_fetches_dir_contents_for_each_filter(self, mock_fetch):
        await init_cache_for_user(CACHE_SCOPE, filters())

        assert call("a/") in mock_fetch.call_args_list
        assert call("b/") in mock_fetch.call_args_list

    async def test_ignores_duplicate_filters(self, mock_fetch):
        await init_cache_for_user(CACHE_SCOPE, filters(second_filter_resource_id="a/"))
        mock_fetch.assert_called_once_with("a/")

    async def tests_clears_out_filters_no_longer_in_use(self, cache_instance):
        still_in_use = DirectoryContents(file_paths=[], folder_paths=[])
        cache_instance.update(
            {
                "storage-permissions:a/": still_in_use,
                "storage-permissions:not-still-in-use": DirectoryContents(),
            }
        )

        await init_cache_for_user(CACHE_SCOPE, filters())

        assert cache_instance == {"storage-permissions:a/": still_in_use}


class TestIsUnsupportedAddressResponse:
    @pytest.mark.parametrize(
        "body",
        [
            '{"type":"about:blank","status":400,"title":"Bad Request",'
            '"detail":"resource address is not supported"}',
            '{"detail": "Resource Address Is Not Supported"}',
            '{"detail": "  resource address is not supported  "}',
            "resource address is not supported",
        ],
    )
    def test_true_for_unsupported_address(self, body):
        assert storage_permissions_cache._is_unsupported_address_response(body) is True

    @pytest.mark.parametrize(
        "body",
        [
            '{"detail": "malformed request"}',
            '{"status": 400}',
            "some other error",
            "",
        ],
    )
    def test_false_for_anything_else(self, body):
        assert storage_permissions_cache._is_unsupported_address_response(body) is False


#: The body of a storage API 401, which must reach the log but not the client.
_UPSTREAM_401_BODY = "internal-detail-from-storage-api"


@pytest.mark.asyncio
class TestFetchFromStorageApi:
    @pytest_asyncio.fixture(autouse=True)
    async def manage_http_session(self, mock_api_fetch, mocker):
        mocker.patch.object(
            _settings_module,
            "storage_endpoint",
            return_value="storage-endpoint/",
        )
        await http_client.open()

        yield

        await http_client.close()

    @pytest.fixture
    def http_response(self):
        return MockHttpResponse()

    @pytest.fixture(autouse=True)
    def mock_http_request(self, http_response, mocker):
        future = asyncio.Future()
        future.set_result(http_response)
        return mocker.patch.object(http_client.session, "get", return_value=future)

    @pytest.fixture
    def url(self):
        return "storage-endpoint/v1beta/filefolder/list/test%2Fad"

    async def test_makes_http_call_to_api(self, url, mock_http_request):
        await fetch_from_storage_api("test/ad")

        mock_http_request.assert_called_once_with(
            url,
            headers={"Authorization": "Bearer test-token"},
        )

    async def test_can_limit_to_returning_a_single_result(self, mock_http_request, url):
        await fetch_from_storage_api("test/ad", limit_to_one_result=True)

        mock_http_request.assert_called_once_with(
            f"{url}?max_page_size=1",
            headers={"Authorization": "Bearer test-token"},
        )

    async def test_returns_dir_contents_object_with_authorized_resources(self):
        dir_contents = await fetch_from_storage_api("test-address")
        assert dir_contents.authorized is True
        assert dir_contents.resource_paths == {"a", "b"}
        assert dir_contents.folders == {"dir_a", "dir_b"}

    async def test_raises_authentication_error_on_401(self, http_response):
        http_response.status = 401
        with pytest.raises(AuthenticationError, match="Authentication failed"):
            await fetch_from_storage_api("test-address")

    @pytest.fixture
    def rejected_with_a_body(self, http_response):
        """The AuthenticationError a 401 carrying ``_UPSTREAM_401_BODY`` raises."""

        async def _run() -> AuthenticationError:
            http_response.status = 401
            http_response.text_response = _UPSTREAM_401_BODY
            with pytest.raises(AuthenticationError) as exc_info:
                await fetch_from_storage_api("test-address")
            return exc_info.value

        return _run

    async def test_401_keeps_the_upstream_body_out_of_the_client_message(
        self, rejected_with_a_body
    ):
        error = await rejected_with_a_body()

        assert _UPSTREAM_401_BODY not in error.user_message

    async def test_401_keeps_the_upstream_body_for_the_log(self, rejected_with_a_body):
        # The body is diagnostic: it moves to the internal message, which the
        # exception interceptor and the REST error middleware log in full.
        error = await rejected_with_a_body()

        assert _UPSTREAM_401_BODY in str(error)

    async def test_classifies_200_as_listed(self):
        dir_contents = await fetch_from_storage_api("test-address")
        assert dir_contents.outcome is ListingOutcome.LISTED

    async def test_returns_unauthorized_dir_contents_on_403(self, http_response):
        http_response.status = 403
        dir_contents = await fetch_from_storage_api("test-address")
        assert dir_contents.authorized is False

    async def test_classifies_403_as_forbidden(self, http_response):
        http_response.status = 403
        dir_contents = await fetch_from_storage_api("test-address")
        assert dir_contents.outcome is ListingOutcome.FORBIDDEN

    @pytest.fixture
    def not_found_with_body(self, http_response):
        """Answer the listing with a 404 carrying the given body."""

        def _set(body):
            http_response.status = 404
            http_response.text_response = body

        return _set

    @pytest.mark.parametrize("body", EMPTY_LISTING_BODIES, ids=EMPTY_LISTING_IDS)
    async def test_empty_listing_404_is_authorized(self, body, not_found_with_body):
        # The storage service runs its List check before it reads any blobs,
        # so this 404 is only ever sent to a principal that may list the
        # folder.
        not_found_with_body(body)
        dir_contents = await fetch_from_storage_api("test-address")
        assert dir_contents.authorized is True

    @pytest.mark.parametrize("body", EMPTY_LISTING_BODIES, ids=EMPTY_LISTING_IDS)
    async def test_empty_listing_404_has_no_files(self, body, not_found_with_body):
        not_found_with_body(body)
        dir_contents = await fetch_from_storage_api("test-address")
        assert dir_contents.resource_paths == set()

    @pytest.mark.parametrize("body", EMPTY_LISTING_BODIES, ids=EMPTY_LISTING_IDS)
    async def test_empty_listing_404_has_no_folders(self, body, not_found_with_body):
        not_found_with_body(body)
        dir_contents = await fetch_from_storage_api("test-address")
        assert dir_contents.folders == set()

    @pytest.mark.parametrize("body", EMPTY_LISTING_BODIES, ids=EMPTY_LISTING_IDS)
    async def test_classifies_empty_listing_404_as_empty_listing(
        self, body, not_found_with_body
    ):
        not_found_with_body(body)
        dir_contents = await fetch_from_storage_api("test-address")
        assert dir_contents.outcome is ListingOutcome.EMPTY_LISTING

    @pytest.mark.parametrize(
        "body", UNRECOGNIZED_NOT_FOUND_BODIES, ids=UNRECOGNIZED_NOT_FOUND_IDS
    )
    async def test_other_404_is_unauthorized(self, body, not_found_with_body):
        not_found_with_body(body)
        dir_contents = await fetch_from_storage_api("test-address")
        assert dir_contents.authorized is False

    @pytest.mark.parametrize(
        "body", UNRECOGNIZED_NOT_FOUND_BODIES, ids=UNRECOGNIZED_NOT_FOUND_IDS
    )
    async def test_classifies_other_404_as_unrecognized_not_found(
        self, body, not_found_with_body
    ):
        not_found_with_body(body)
        dir_contents = await fetch_from_storage_api("test-address")
        assert dir_contents.outcome is ListingOutcome.UNRECOGNIZED_NOT_FOUND

    @pytest.fixture
    def unreadable_404(self, http_response, mocker):
        """A 404 whose body read fails, as a dropped connection would."""
        # Retries would sleep for real if the 404 branch raised.
        mocker.patch.object(asyncio, "sleep")
        http_response.status = 404
        mocker.patch.object(
            http_response, "text", side_effect=ConnectionResetError("body lost")
        )

    async def test_404_with_unreadable_body_does_not_raise_and_is_unauthorized(
        self, unreadable_404
    ):
        # Raising here would fail the event's authorization check, which
        # nacks the message and tears the stream down.
        dir_contents = await fetch_from_storage_api("test-address")
        assert dir_contents.authorized is False

    async def test_classifies_404_with_unreadable_body_as_unrecognized_not_found(
        self, unreadable_404
    ):
        dir_contents = await fetch_from_storage_api("test-address")
        assert dir_contents.outcome is ListingOutcome.UNRECOGNIZED_NOT_FOUND

    async def test_404_with_unreadable_body_is_not_retried(
        self, unreadable_404, mock_http_request
    ):
        await fetch_from_storage_api("test-address")
        mock_http_request.assert_called_once()

    async def test_returns_unauthorized_dir_contents_on_unsupported_address_400(
        self, http_response
    ):
        # storage-service returns this when the address routes to no configured
        # backend; we treat it as an unauthorized directory so the stream still
        # opens.
        http_response.status = 400
        http_response.text_response = UNSUPPORTED_ADDRESS_BODY
        dir_contents = await fetch_from_storage_api("https://test-resource/x/")
        assert dir_contents.authorized is False

    async def test_classifies_unsupported_address_400_as_unsupported_address(
        self, http_response
    ):
        http_response.status = 400
        http_response.text_response = UNSUPPORTED_ADDRESS_BODY
        dir_contents = await fetch_from_storage_api("https://test-resource/x/")
        assert dir_contents.outcome is ListingOutcome.UNSUPPORTED_ADDRESS

    async def test_other_400_still_raises(self, http_response, mocker):
        mocker.patch.object(asyncio, "sleep")
        http_response.status = 400
        http_response.text_response = json.dumps(
            {"status": 400, "title": "Bad Request", "detail": "malformed request"}
        )
        with pytest.raises(
            ServerError, match="Failed to get data from storage API: 400"
        ):
            await fetch_from_storage_api("test-address")

    class TestWhenErrorsReturnedFromApiCall:
        @pytest.fixture(autouse=True)
        def mock_retry_sleep(self, mocker):
            """
            Doing this for a shorter time between retries during the test suite.
            """
            mocker.patch.object(asyncio, "sleep")

        @pytest.fixture
        def mock_status(self, mocker):
            return mocker.patch.object(storage_permissions_cache, "_status_code")

        async def test_it_retries_on_error(self, http_response, mock_status, mocker):
            mock_status.side_effect = [500, 500, 500, 200]
            spy = mocker.spy(http_response, "text")
            await fetch_from_storage_api("test-address")
            assert spy.call_count == 3

        async def test_it_raises_after_four_tries(self, mock_status):
            mock_status.side_effect = [500, 500, 500, 500]
            with pytest.raises(
                ServerError, match="Failed to get data from storage API: 500"
            ):
                await fetch_from_storage_api("test-address")

    class TestPagination:
        @pytest.fixture
        def paginated_responses(self):
            page1 = MockHttpResponse(
                json_response={
                    "sub_resource_addresses": ["file1", "file2"],
                    "subfolder_addresses": ["folder1"],
                    "next_continuation_handle": "page2-handle",
                }
            )
            page2 = MockHttpResponse(
                json_response={
                    "sub_resource_addresses": ["file3"],
                    "subfolder_addresses": ["folder2", "folder3"],
                    "next_continuation_handle": "page3-handle",
                }
            )
            page3 = MockHttpResponse(
                json_response={
                    "sub_resource_addresses": ["file4"],
                    "subfolder_addresses": [],
                }
            )
            return [page1, page2, page3]

        @pytest.fixture
        def mock_paginated_request(self, paginated_responses, mocker):
            futures = []
            for response in paginated_responses:
                future = asyncio.Future()
                future.set_result(response)
                futures.append(future)
            return mocker.patch.object(http_client.session, "get", side_effect=futures)

        async def test_paginates_until_no_continuation_handle(
            self, mock_paginated_request
        ):
            await fetch_from_storage_api("test/path")

            assert mock_paginated_request.call_count == 3

        async def test_collects_all_files_from_all_pages(self, mock_paginated_request):
            dir_contents = await fetch_from_storage_api("test/path")

            assert dir_contents.resource_paths == {"file1", "file2", "file3", "file4"}

        async def test_collects_all_folders_from_all_pages(
            self, mock_paginated_request
        ):
            dir_contents = await fetch_from_storage_api("test/path")

            assert dir_contents.folders == {"folder1", "folder2", "folder3"}

        async def test_passes_continuation_handle_in_subsequent_requests(
            self, mock_paginated_request
        ):
            await fetch_from_storage_api("test/path")

            calls = mock_paginated_request.call_args_list
            # First call should not have continuation_handle
            assert "continuation_handle" not in calls[0][0][0]
            # The Second call should have the handle from the first response
            assert "continuation_handle=page2-handle" in calls[1][0][0]
            # Third call should have the handle from second response
            assert "continuation_handle=page3-handle" in calls[2][0][0]

        async def test_does_not_paginate_when_limit_to_one_result_is_true(
            self, mock_paginated_request
        ):
            await fetch_from_storage_api("test/path", limit_to_one_result=True)

            assert mock_paginated_request.call_count == 1

        async def test_stops_when_continuation_handle_is_null(self, mocker):
            page1 = MockHttpResponse(
                json_response={
                    "sub_resource_addresses": ["file1"],
                    "subfolder_addresses": [],
                    "next_continuation_handle": None,
                }
            )
            future = asyncio.Future()
            future.set_result(page1)
            mock_request = mocker.patch.object(
                http_client.session, "get", return_value=future
            )

            dir_contents = await fetch_from_storage_api("test/path")

            assert mock_request.call_count == 1
            assert dir_contents.resource_paths == {"file1"}


class TestBuildStorageApiUrl:
    def test_returns_base_url_when_no_params(self):
        url = _build_storage_api_url("https://api.example.com/path", False, None)
        assert url == "https://api.example.com/path"

    def test_adds_max_page_size_when_limit_to_one_result(self):
        url = _build_storage_api_url("https://api.example.com/path", True, None)
        assert url == "https://api.example.com/path?max_page_size=1"

    def test_adds_continuation_handle_when_provided(self):
        url = _build_storage_api_url("https://api.example.com/path", False, "handle123")
        assert url == "https://api.example.com/path?continuation_handle=handle123"

    def test_adds_both_params_when_both_provided(self):
        url = _build_storage_api_url("https://api.example.com/path", True, "handle123")
        expected = (
            "https://api.example.com/path?max_page_size=1&continuation_handle=handle123"
        )
        assert url == expected

    def test_url_encodes_continuation_handle(self):
        url = _build_storage_api_url(
            "https://api.example.com/path", False, "handle/with+special=chars"
        )
        assert "continuation_handle=handle%2Fwith%2Bspecial%3Dchars" in url


@pytest.mark.asyncio
class TestFetchParentResource:
    async def test_gets_the_parent_folder_path(self, mocker):
        spy = mocker.patch.object(storage_permissions_cache, "_parent_folder_path")
        await get_parent_resource("test-address")
        spy.assert_called_once_with("test-address")

    async def test_fetches_the_dir_contents_object_from_cache(self, mock_fetch, mocker):
        mocker.patch.object(
            storage_permissions_cache, "_parent_folder_path", return_value="path"
        )
        await get_parent_resource("")
        mock_fetch.assert_called_once_with("path")

    async def test_returns_fetch_results(self):
        assert await get_parent_resource("") == "fetch-response"


class TestParentFolderPath:
    @pytest.mark.parametrize("path", ["a/b/c", "a/b/c/", "a/b/file.usd"])
    def test_always_returns_parent_folder_path(self, path):
        assert _parent_folder_path(path) == "a/b/"


class TestCacheKey:
    def test_combines_cache_string_with_the_path(self):
        assert _cache_key("test-path") == "storage-permissions:test-path"


@pytest.mark.asyncio
class TestFetchDirContents:
    @pytest.fixture
    def cache_key(self):
        return "test-key"

    @pytest.fixture(autouse=True)
    def cache_key_mock(self, cache_key, mocker):
        return mocker.patch.object(
            storage_permissions_cache, "_cache_key", return_value=cache_key
        )

    async def test_gets_cache_key(self, cache_key_mock):
        await _fetch_dir_contents("test-address")
        cache_key_mock.assert_called_once_with("test-address")

    class TestWithCacheHit:
        @pytest.fixture
        def dir_contents(self):
            return DirectoryContents(file_paths=[], folder_paths=[])

        @pytest_asyncio.fixture(autouse=True)
        async def setup_teardown_cache(self, cache_key, dir_contents, cache_instance):
            cache_instance[cache_key] = dir_contents

            yield

            del cache_instance[cache_key]

        async def test_returns_dir_contents(self, dir_contents):
            assert await _fetch_dir_contents("") == dir_contents

        async def test_doesnt_fetch_from_api(self, mock_api_fetch):
            await _fetch_dir_contents("")
            mock_api_fetch.assert_not_called()

        async def test_does_fetch_from_api_if_dir_contents_expired(
            self, mock_api_fetch, dir_contents, mocker
        ):
            mocker.patch.object(dir_contents, "expired", return_value=True)
            await _fetch_dir_contents("")
            assert mock_api_fetch.call_count == 1

        async def test_absorbs_disappeared_paths_on_refresh(
            self, cache_key, mock_api_fetch, dir_contents, mocker, cache_instance
        ):
            dir_contents.authorized = True
            dir_contents.resource_paths = {"a", "b"}
            mocker.patch.object(dir_contents, "expired", return_value=True)

            new_contents = DirectoryContents(file_paths=["a"], folder_paths=[])
            mock_api_fetch.return_value = new_contents

            result = await _fetch_dir_contents("")

            assert result is new_contents
            assert "b" in result.previously_visible_paths

        async def test_does_not_absorb_when_old_entry_was_unauthorized(
            self, cache_key, mock_api_fetch, dir_contents, mocker, cache_instance
        ):
            dir_contents.authorized = False
            mocker.patch.object(dir_contents, "expired", return_value=True)

            new_contents = DirectoryContents(file_paths=["a"], folder_paths=[])
            mock_api_fetch.return_value = new_contents

            result = await _fetch_dir_contents("")

            assert result.previously_visible_paths == {}

        async def test_does_not_absorb_when_new_entry_is_unauthorized(
            self, cache_key, mock_api_fetch, dir_contents, mocker, cache_instance
        ):
            dir_contents.authorized = True
            dir_contents.resource_paths = {"a"}
            mocker.patch.object(dir_contents, "expired", return_value=True)

            new_contents = DirectoryContents()
            mock_api_fetch.return_value = new_contents

            result = await _fetch_dir_contents("")

            assert result.previously_visible_paths == {}

    class TestWithCacheMiss:
        @pytest_asyncio.fixture(autouse=True)
        async def teardown_cache(self, cache_key, cache_instance):
            try:
                del cache_instance[cache_key]
            except KeyError:
                pass

            yield

            try:
                del cache_instance[cache_key]
            except KeyError:
                pass

        async def test_fetches_from_storage_api(self, mock_api_fetch):
            await _fetch_dir_contents("test-address")
            mock_api_fetch.assert_called_once_with("test-address")

        async def test_sets_result_of_fetch_in_dict_with_cache_key(
            self, cache_key, mock_api_fetch, cache_instance
        ):
            await _fetch_dir_contents("test-address")
            assert cache_instance[cache_key] == mock_api_fetch.return_value

        async def test_returns_results_of_fetch(self, mock_api_fetch):
            expected = mock_api_fetch.return_value
            assert await _fetch_dir_contents("test-address") == expected

        async def test_does_not_debounce_initial_unauthorized_result(
            self, mock_api_fetch, freezer
        ):
            mock_api_fetch.return_value = DirectoryContents()
            result = await _fetch_dir_contents("test-address")
            assert result.expires_at == 0

    class TestUnauthorizedRecheck:
        @pytest_asyncio.fixture(autouse=True)
        async def setup_expired_entry(self, cache_key, cache_instance):
            cache_instance[cache_key] = DirectoryContents()
            yield
            try:
                del cache_instance[cache_key]
            except KeyError:
                pass

        async def test_applies_debounce_ttl(self, mock_api_fetch, freezer):
            mock_api_fetch.return_value = DirectoryContents()
            result = await _fetch_dir_contents("test-address")
            expected = (
                time.time() + _settings_module.settings.unauthorized_recheck_seconds
            )
            assert result.expires_at == expected

        async def test_fetches_from_api(self, mock_api_fetch):
            mock_api_fetch.return_value = DirectoryContents()
            await _fetch_dir_contents("test-address")
            mock_api_fetch.assert_called_once_with("test-address")

        async def test_authorized_recheck_uses_full_ttl(self, mock_api_fetch, freezer):
            mock_api_fetch.return_value = DirectoryContents(
                file_paths=["a"], folder_paths=[]
            )
            result = await _fetch_dir_contents("test-address")
            expected = time.time() + _settings_module.settings.permissions_ttl_seconds
            assert result.expires_at == expected


# --------------------------------------------------------------------------
# _fetch_dir_contents against the storage API's real answers (b.54z)
# --------------------------------------------------------------------------


#: A client folder path distinctive enough that finding it anywhere in a log
#: record means the record carries the path.
CLIENT_DIR = "/tenant-x/private-project/new-folder/"

#: The message of the line that records a listing which left a directory
#: unauthorized.
UNAUTHORIZED_LISTING_MESSAGE = (
    "storage listing did not authorize a directory; "
    "its storage events are dropped until the recheck"
)


@pytest_asyncio.fixture
async def real_storage_fetch(mock_api_fetch, mocker):
    """
    Route ``_fetch_dir_contents`` through the real ``fetch_from_storage_api``
    over a mocked HTTP session, so the cache policy is pinned against the
    status and body the storage API sends rather than against a hand-built
    ``DirectoryContents``.
    """
    mocker.patch.object(
        _settings_module, "storage_endpoint", return_value="storage-endpoint/"
    )
    mock_api_fetch.side_effect = fetch_from_storage_api
    await http_client.open()

    yield

    await http_client.close()


@pytest.fixture
def storage_answers(real_storage_fetch, mocker):
    """
    Make every directory listing answer with ``status`` and ``body``.

    Returns the patched ``session.get``, so a test can count listings.
    """

    def _set(status, body="test-msg"):
        future = asyncio.Future()
        future.set_result(MockHttpResponse(status=status, text=body))
        return mocker.patch.object(http_client.session, "get", return_value=future)

    return _set


@pytest.fixture
def unauthorized_entry(cache_instance):
    """
    Seed the expired unauthorized entry a 403 during initialize() leaves, so
    the next fetch is a recheck.
    """
    entry = DirectoryContents(dir_path=CLIENT_DIR, outcome=ListingOutcome.FORBIDDEN)
    cache_instance[_cache_key(CLIENT_DIR)] = entry
    return entry


@pytest.mark.asyncio
class TestFetchDirContentsByListingOutcome:
    @pytest.fixture
    def expired_entry_holding_a_file(self, cache_instance, freezer):
        """Seed an authorized entry that listed one file and has expired."""
        entry = DirectoryContents(
            file_paths=[f"{CLIENT_DIR}file.usd"],
            folder_paths=[],
            dir_path=CLIENT_DIR,
            outcome=ListingOutcome.LISTED,
        )
        entry.expires_at = time.time() - 1
        cache_instance[_cache_key(CLIENT_DIR)] = entry
        return entry

    async def test_empty_listing_on_the_initial_fetch_gets_the_full_ttl(
        self, storage_answers, freezer
    ):
        storage_answers(404, AZURE_EMPTY_LISTING_BODY)

        result = await _fetch_dir_contents(CLIENT_DIR)

        expected = time.time() + _settings_module.settings.permissions_ttl_seconds
        assert result.expires_at == expected

    async def test_empty_listing_on_a_recheck_gets_the_full_ttl(
        self, storage_answers, unauthorized_entry, freezer
    ):
        storage_answers(404, AZURE_EMPTY_LISTING_BODY)

        result = await _fetch_dir_contents(CLIENT_DIR)

        expected = time.time() + _settings_module.settings.permissions_ttl_seconds
        assert result.expires_at == expected

    async def test_empty_listing_refresh_keeps_vanished_files_deletable(
        self, storage_answers, expired_entry_holding_a_file
    ):
        storage_answers(404, AZURE_EMPTY_LISTING_BODY)

        result = await _fetch_dir_contents(CLIENT_DIR)

        assert f"{CLIENT_DIR}file.usd" in result.previously_visible_paths

    async def test_forbidden_recheck_does_not_list_again_inside_the_window(
        self, storage_answers, unauthorized_entry, freezer
    ):
        listing = storage_answers(403)
        await _fetch_dir_contents(CLIENT_DIR)
        freezer.tick(_settings_module.settings.unauthorized_recheck_seconds - 1)

        await _fetch_dir_contents(CLIENT_DIR)

        assert listing.call_count == 1

    async def test_empty_listing_is_mirrored_to_redis_as_authorized_with_the_full_ttl(
        self, storage_answers, patch_active_writer, freezer
    ):
        storage_answers(404, AZURE_EMPTY_LISTING_BODY)

        await _fetch_dir_contents(CLIENT_DIR)

        assert patch_active_writer.ops == [
            redis_cache.replace_dir(
                scope_key=CACHE_SCOPE_KEY,
                dir_path=CLIENT_DIR,
                files=frozenset(),
                folders=frozenset(),
                authorized=True,
                expires_at=time.time()
                + _settings_module.settings.permissions_ttl_seconds,
                prev={},
            )
        ]


#: The listings that leave a directory unauthorized, each with the level its
#: line is logged at and the outcome it is classified as. A 403 is an
#: ordinary denial; the other two point at a deployment problem.
UNAUTHORIZED_ANSWERS = [
    pytest.param(403, "test-msg", logging.INFO, "forbidden", id="403"),
    pytest.param(
        400,
        UNSUPPORTED_ADDRESS_BODY,
        logging.WARNING,
        "unsupported_address",
        id="unsupported-400",
    ),
    pytest.param(404, "", logging.WARNING, "unrecognized_not_found", id="other-404"),
]


@pytest.mark.asyncio
class TestFetchDirContentsUnauthorizedListingLog:
    """
    A listing that leaves a directory unauthorized is recorded at INFO or
    WARNING, so dropped storage events are visible without DEBUG logging.

    The line is exported with its extras when telemetry is on, so it names
    the stream by redacted reference and leaves the client's folder path out
    (the per-event DEBUG lines carry it).
    """

    @pytest.fixture
    def listing_log_records(self, caplog, storage_answers):
        """Fetch ``CLIENT_DIR`` once and return the unauthorized-listing lines."""

        async def _run(status, body):
            storage_answers(status, body)
            with caplog.at_level(
                logging.DEBUG, logger=storage_permissions_cache.logger.name
            ):
                await _fetch_dir_contents(CLIENT_DIR)
            return [
                r
                for r in caplog.records
                if r.getMessage() == UNAUTHORIZED_LISTING_MESSAGE
            ]

        return _run

    @pytest.mark.parametrize("status, body, level, outcome", UNAUTHORIZED_ANSWERS)
    async def test_is_emitted_once_per_unauthorized_fetch(
        self, status, body, level, outcome, listing_log_records
    ):
        assert len(await listing_log_records(status, body)) == 1

    @pytest.mark.parametrize("status, body, level, outcome", UNAUTHORIZED_ANSWERS)
    async def test_level_follows_the_outcome(
        self, status, body, level, outcome, listing_log_records
    ):
        [record] = await listing_log_records(status, body)

        assert record.levelno == level

    @pytest.mark.parametrize("status, body, level, outcome", UNAUTHORIZED_ANSWERS)
    async def test_carries_the_storage_status(
        self, status, body, level, outcome, listing_log_records
    ):
        [record] = await listing_log_records(status, body)

        assert record.storage_status == status

    @pytest.mark.parametrize("status, body, level, outcome", UNAUTHORIZED_ANSWERS)
    async def test_carries_the_listing_outcome(
        self, status, body, level, outcome, listing_log_records
    ):
        [record] = await listing_log_records(status, body)

        assert record.listing_outcome == outcome

    @pytest.mark.parametrize(
        "status, body",
        [
            pytest.param(200, "test-msg", id="listed"),
            pytest.param(404, AZURE_EMPTY_LISTING_BODY, id="empty-listing"),
        ],
    )
    async def test_is_not_emitted_for_an_authorized_listing(
        self, status, body, listing_log_records
    ):
        assert await listing_log_records(status, body) == []

    async def test_carries_the_redacted_queue_reference(self, listing_log_records):
        [record] = await listing_log_records(403, "test-msg")

        assert record.queue_ref == redact_reconnect_token(RECONNECT_TOKEN)

    async def test_does_not_carry_the_token(self, listing_log_records):
        [record] = await listing_log_records(403, "test-msg")

        assert RECONNECT_TOKEN not in str(record.__dict__)

    async def test_does_not_name_the_directory(self, listing_log_records):
        [record] = await listing_log_records(403, "test-msg")

        assert CLIENT_DIR not in str(record.__dict__)

    async def test_reports_no_drops_without_a_replaced_entry(self, listing_log_records):
        [record] = await listing_log_records(403, "test-msg")

        assert record.dropped_events == 0

    async def test_reports_the_drops_against_the_replaced_entry(
        self, unauthorized_entry, listing_log_records
    ):
        unauthorized_entry.dropped_events = 4

        [record] = await listing_log_records(403, "test-msg")

        assert record.dropped_events == 4

    @staticmethod
    def _redis_built_unauthorized_entry(expires_at):
        """An unauthorized entry built the way Redis hydration builds it."""
        return DirectoryContents.from_redis(
            authorized=False,
            files=None,
            folders=None,
            expires_at=expires_at,
            prev={},
            dir_path=CLIENT_DIR,
        )

    async def test_reports_the_drops_against_a_replaced_entry_built_from_redis(
        self, cache_instance, listing_log_records
    ):
        # from_redis skips __init__, so the count exists only if from_redis
        # sets it. Incremented as the readers do, so a missing count fails
        # here rather than being created by the test.
        entry = self._redis_built_unauthorized_entry(expires_at=time.time() - 1)
        entry.dropped_events += 2
        cache_instance[_cache_key(CLIENT_DIR)] = entry

        [record] = await listing_log_records(403, "test-msg")

        assert record.dropped_events == 2

    async def test_is_not_emitted_when_redis_hydrates_an_unauthorized_entry(
        self, mock_hydrate, listing_log_records
    ):
        mock_hydrate.return_value = self._redis_built_unauthorized_entry(
            expires_at=time.time() + 30
        )

        assert await listing_log_records(403, "test-msg") == []


# --------------------------------------------------------------------------
# Per-mutation write-through replace_dir Op (Bee oo / Epic .nd)
# --------------------------------------------------------------------------


#: What the writer hands the Op builders: the cache-scope key, which is
#: the broker queue name plus the presenting principal's reference.
SCOPE_KEY = CACHE_SCOPE_KEY


class _FakeWriter:
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
    Patch ``active_writer`` on the storage_permissions_cache module
    (the import-bound name used by `_fetch_dir_contents`).
    """
    mocker.patch.object(storage_permissions_cache, "active_writer", return_value=writer)
    return writer


@pytest.mark.asyncio
class TestFetchDirContentsReplaceDirOp:
    @pytest.fixture
    def cache_key(self):
        return "test-key"

    @pytest.fixture(autouse=True)
    def cache_key_mock(self, cache_key, mocker):
        return mocker.patch.object(
            storage_permissions_cache, "_cache_key", return_value=cache_key
        )

    @pytest_asyncio.fixture
    async def empty_cache(self, cache_key, cache_instance):
        try:
            del cache_instance[cache_key]
        except KeyError:
            pass
        yield cache_instance
        try:
            del cache_instance[cache_key]
        except KeyError:
            pass

    async def test_enqueues_one_replace_dir_op_on_cache_miss(
        self, empty_cache, mock_api_fetch, patch_active_writer, freezer
    ):
        new_contents = DirectoryContents(
            file_paths=["a", "b"],
            folder_paths=["d1"],
            dir_path="some/dir/",
        )
        mock_api_fetch.return_value = new_contents

        await _fetch_dir_contents("some/dir/")

        assert patch_active_writer.ops == [
            redis_cache.replace_dir(
                scope_key=SCOPE_KEY,
                dir_path="some/dir/",
                files=frozenset({"a", "b"}),
                folders=frozenset({"d1"}),
                authorized=True,
                expires_at=new_contents.expires_at,
                prev={},
            )
        ]

    async def test_enqueues_one_replace_dir_op_on_expired_refresh(
        self, cache_key, cache_instance, mock_api_fetch, patch_active_writer, freezer
    ):
        # Seed cache with an expired entry. The expired-refresh code path
        # is the same as cache miss (both fall through to the API fetch +
        # enqueue).
        old = DirectoryContents(file_paths=["x"], folder_paths=[])
        old.expires_at = time.time() - 1
        cache_instance[cache_key] = old

        new_contents = DirectoryContents(
            file_paths=["x"], folder_paths=[], dir_path="some/dir/"
        )
        mock_api_fetch.return_value = new_contents

        await _fetch_dir_contents("some/dir/")

        # Exactly one Op (replace_dir), and it is for this dir.
        assert len(patch_active_writer.ops) == 1
        assert isinstance(patch_active_writer.ops[0], redis_cache.replace_dir)
        assert patch_active_writer.ops[0].dir_path == "some/dir/"

        # Cleanup.
        try:
            del cache_instance[cache_key]
        except KeyError:
            pass

    async def test_no_op_on_cache_hit_not_expired(
        self,
        cache_key,
        cache_instance,
        mock_api_fetch,
        patch_active_writer,
        freezer,
    ):
        # Seed cache with a fresh, non-expired entry.
        live = DirectoryContents(file_paths=["a"], folder_paths=[])
        cache_instance[cache_key] = live

        await _fetch_dir_contents("some/dir/")

        assert patch_active_writer.ops == []
        mock_api_fetch.assert_not_called()

        try:
            del cache_instance[cache_key]
        except KeyError:
            pass

    async def test_replace_dir_payload_reflects_post_absorb_prev_state(
        self,
        cache_key,
        cache_instance,
        mock_api_fetch,
        patch_active_writer,
        mocker,
        freezer,
    ):
        # Seed cache with an authorized old entry whose resource_paths
        # contains a path that will disappear in the new fetch. The new
        # entry must absorb that disappeared path into its
        # ``previously_visible_paths`` BEFORE the replace_dir Op is
        # enqueued, so the Op carries the right `prev=` mapping.
        mocker.patch.object(
            _settings_module.settings, "previously_visible_paths_ttl_seconds", 100
        )

        old = DirectoryContents(file_paths=["a", "disappears"], folder_paths=[])
        old.expires_at = time.time() - 1
        cache_instance[cache_key] = old

        new_contents = DirectoryContents(
            file_paths=["a"], folder_paths=[], dir_path="some/dir/"
        )
        mock_api_fetch.return_value = new_contents

        await _fetch_dir_contents("some/dir/")

        assert len(patch_active_writer.ops) == 1
        op = patch_active_writer.ops[0]
        assert isinstance(op, redis_cache.replace_dir)
        assert "disappears" in op.prev
        assert op.prev["disappears"] == time.time() + 100

        try:
            del cache_instance[cache_key]
        except KeyError:
            pass


@pytest.mark.asyncio
class TestInitCacheCleanupBranchNoOp:
    """
    The cleanup branch of init_cache_for_user (the loop that removes the
    keys no filter uses any more) must not enqueue any Redis op — the
    engineer's `.nd` source has a contractual comment to that effect;
    entries TTL out instead.
    """

    async def test_cleanup_branch_does_not_enqueue(
        self, mock_fetch, patch_active_writer, cache_instance
    ):
        cache_instance.update(
            {
                "storage-permissions:a/": DirectoryContents(),
                "storage-permissions:not-still-in-use": DirectoryContents(),
            }
        )

        # mock_fetch replaces _fetch_dir_contents so it does not enqueue
        # anything itself; this test isolates the cleanup branch.
        await init_cache_for_user(CACHE_SCOPE, filters())

        assert patch_active_writer.ops == []


@pytest.mark.asyncio
class TestDirContentsConstructionPassesDirPath:
    """
    All `DirectoryContents` construction sites in storage_permissions_cache
    must pass `dir_path` so future mutations on those instances enqueue
    Ops. Verified by reading `dir_path` off the returned instance.
    """

    @pytest_asyncio.fixture(autouse=True)
    async def manage_http_session(self, mocker):
        mocker.patch.object(
            _settings_module,
            "storage_endpoint",
            return_value="storage-endpoint/",
        )
        await http_client.open()
        yield
        await http_client.close()

    @pytest.fixture
    def http_response_403(self):
        from tests.fixtures.mock_http_response import MockHttpResponse

        return MockHttpResponse(status=403)

    @pytest.fixture
    def http_response_200(self):
        from tests.fixtures.mock_http_response import MockHttpResponse

        return MockHttpResponse()

    async def test_authorized_response_carries_dir_path(
        self, mocker, http_response_200
    ):
        future = asyncio.Future()
        future.set_result(http_response_200)
        mocker.patch.object(http_client.session, "get", return_value=future)

        result = await fetch_from_storage_api("some/dir/")

        assert result.dir_path == "some/dir/"

    async def test_unauthorized_response_carries_dir_path(
        self, mocker, http_response_403
    ):
        future = asyncio.Future()
        future.set_result(http_response_403)
        mocker.patch.object(http_client.session, "get", return_value=future)

        result = await fetch_from_storage_api("some/dir/")

        assert result.dir_path == "some/dir/"
        assert result.authorized is False

    async def test_empty_listing_response_carries_dir_path(self, mocker):
        # The created events an empty listing admits mutate this entry, and
        # those mutations reach Redis only when it knows its dir_path.
        future = asyncio.Future()
        future.set_result(MockHttpResponse(status=404, text=AZURE_EMPTY_LISTING_BODY))
        mocker.patch.object(http_client.session, "get", return_value=future)

        result = await fetch_from_storage_api("some/dir/")

        assert result.dir_path == "some/dir/"


# --------------------------------------------------------------------------
# Redis-first hydration in _fetch_dir_contents (Bee ox / Epic .hn)
# --------------------------------------------------------------------------


@pytest.fixture
def mock_hydrate(mocker) -> AsyncMock:
    """
    Default the hydration helper to a Redis miss so existing test classes
    fall through to the storage-API path unchanged. Individual tests can
    override the return value.
    """
    return mocker.patch.object(
        redis_cache, "hydrate_dir_from_redis", AsyncMock(return_value=None)
    )


@pytest.fixture
def mock_record_hydrate(mocker):
    return mocker.patch.object(
        storage_permissions_cache.cache_hydration_metrics, "record_hydrate"
    )


@pytest.mark.asyncio
class TestRedisHydration:
    @pytest.fixture
    def cache_key(self):
        return "test-key"

    @pytest.fixture(autouse=True)
    def cache_key_mock(self, cache_key, mocker):
        return mocker.patch.object(
            storage_permissions_cache, "_cache_key", return_value=cache_key
        )

    @pytest_asyncio.fixture
    async def empty_cache(self, cache_key, cache_instance):
        try:
            del cache_instance[cache_key]
        except KeyError:
            pass
        yield cache_instance
        try:
            del cache_instance[cache_key]
        except KeyError:
            pass

    @pytest.fixture
    def hydrated_contents(self):
        contents = DirectoryContents.from_redis(
            authorized=True,
            files={"a.txt"},
            folders={"sub"},
            expires_at=time.time() + 100,
            prev={},
            dir_path="test-address",
        )
        return contents

    async def test_does_not_attempt_redis_hydration_when_in_memory_entry_is_fresh(
        self,
        mock_hydrate,
        mock_record_hydrate,
        mock_api_fetch,
        cache_instance,
        cache_key,
    ):
        # Seed cache with fresh entry.
        cache_instance[cache_key] = DirectoryContents(file_paths=["x"], folder_paths=[])

        await _fetch_dir_contents("test-address")

        mock_hydrate.assert_not_called()
        mock_record_hydrate.assert_not_called()
        mock_api_fetch.assert_not_called()

        del cache_instance[cache_key]

    async def test_returns_redis_hydrated_dir_contents_on_cache_miss(
        self, empty_cache, mock_hydrate, mock_api_fetch, hydrated_contents
    ):
        mock_hydrate.return_value = hydrated_contents

        result = await _fetch_dir_contents("test-address")

        assert result is hydrated_contents
        mock_api_fetch.assert_not_called()

    async def test_populates_in_memory_cache_from_redis_on_hydration_hit(
        self, empty_cache, mock_hydrate, hydrated_contents, cache_key
    ):
        mock_hydrate.return_value = hydrated_contents

        await _fetch_dir_contents("test-address")

        assert empty_cache[cache_key] is hydrated_contents

    async def test_emits_redis_source_metric_on_redis_hit(
        self, empty_cache, mock_hydrate, mock_record_hydrate, hydrated_contents
    ):
        mock_hydrate.return_value = hydrated_contents

        await _fetch_dir_contents("test-address")

        mock_record_hydrate.assert_called_once()
        args, _ = mock_record_hydrate.call_args
        assert args[0] == "redis"
        assert isinstance(args[1], float)

    async def test_falls_back_to_storage_api_on_redis_miss(
        self, empty_cache, mock_hydrate, mock_api_fetch, mock_record_hydrate
    ):
        mock_hydrate.return_value = None

        await _fetch_dir_contents("test-address")

        mock_api_fetch.assert_called_once_with("test-address")
        mock_record_hydrate.assert_called_once()
        args, _ = mock_record_hydrate.call_args
        assert args[0] == "storage_api"
        assert isinstance(args[1], float)

    async def test_replace_dir_enqueue_still_fires_on_storage_api_fallback(
        self, empty_cache, mock_hydrate, mock_api_fetch, patch_active_writer
    ):
        mock_hydrate.return_value = None
        new_contents = DirectoryContents(
            file_paths=["a"], folder_paths=["sub"], dir_path="test-address"
        )
        mock_api_fetch.return_value = new_contents

        await _fetch_dir_contents("test-address")

        assert len(patch_active_writer.ops) == 1
        assert isinstance(patch_active_writer.ops[0], redis_cache.replace_dir)

    async def test_replace_dir_enqueue_does_not_fire_on_redis_hit(
        self, empty_cache, mock_hydrate, hydrated_contents, patch_active_writer
    ):
        mock_hydrate.return_value = hydrated_contents

        await _fetch_dir_contents("test-address")

        assert patch_active_writer.ops == []

    async def test_falls_back_to_storage_api_on_redis_connection_error(
        self, empty_cache, mock_hydrate, mock_api_fetch, mock_record_hydrate
    ):
        # The helper's contract: it absorbs ConnectionError internally and
        # returns None. So at this layer we just simulate the helper's
        # observable behavior: returns None on error.
        mock_hydrate.return_value = None

        # Sanity: even if a connection error were to leak through, we want
        # _fetch_dir_contents to keep working.
        result = await _fetch_dir_contents("test-address")

        assert result is mock_api_fetch.return_value
        mock_api_fetch.assert_called_once_with("test-address")
        args, _ = mock_record_hydrate.call_args
        assert args[0] == "storage_api"

    async def test_emits_storage_api_source_metric_when_redis_disabled(
        self,
        empty_cache,
        mock_hydrate,
        mock_api_fetch,
        mock_record_hydrate,
        mocker,
    ):
        # Helper short-circuits to None when redis_enabled is False; the
        # wired call site sees None and falls through, emitting source=storage_api.
        mocker.patch.object(_settings_module.settings, "redis_enabled", False)
        mock_hydrate.return_value = None

        await _fetch_dir_contents("test-address")

        mock_api_fetch.assert_called_once()
        args, _ = mock_record_hydrate.call_args
        assert args[0] == "storage_api"


@pytest.mark.asyncio
class TestRedisHydrationOnExpiredEntry:
    @pytest.fixture
    def cache_key(self):
        return "test-key"

    @pytest.fixture(autouse=True)
    def cache_key_mock(self, cache_key, mocker):
        return mocker.patch.object(
            storage_permissions_cache, "_cache_key", return_value=cache_key
        )

    @pytest_asyncio.fixture
    async def expired_entry(self, cache_key, cache_instance):
        old = DirectoryContents(file_paths=["a", "disappears"], folder_paths=[])
        old.expires_at = time.time() - 1
        cache_instance[cache_key] = old
        yield old
        try:
            del cache_instance[cache_key]
        except KeyError:
            pass

    async def test_uses_redis_hydration_when_in_memory_entry_expired_and_redis_hits(
        self,
        expired_entry,
        mock_hydrate,
        mock_api_fetch,
        mock_record_hydrate,
    ):
        hydrated = DirectoryContents.from_redis(
            authorized=True,
            files={"a"},
            folders=set(),
            expires_at=time.time() + 100,
            prev={},
            dir_path="test-address",
        )
        mock_hydrate.return_value = hydrated

        result = await _fetch_dir_contents("test-address")

        assert result is hydrated
        mock_api_fetch.assert_not_called()
        args, _ = mock_record_hydrate.call_args
        assert args[0] == "redis"

    async def test_absorb_disappeared_paths_parity_preserved_on_redis_hit(
        self, expired_entry, mock_hydrate, mock_api_fetch, mocker
    ):
        mocker.patch.object(
            _settings_module.settings, "previously_visible_paths_ttl_seconds", 100
        )
        hydrated = DirectoryContents.from_redis(
            authorized=True,
            files={"a"},
            folders=set(),
            expires_at=time.time() + 100,
            prev={},
            dir_path="test-address",
        )
        mock_hydrate.return_value = hydrated

        result = await _fetch_dir_contents("test-address")

        # The absorbed disappeared path "disappears" from the old entry
        # must survive into the hydrated DirectoryContents.
        assert "disappears" in result.previously_visible_paths

    async def test_falls_back_to_storage_api_on_expired_entry_redis_miss(
        self, expired_entry, mock_hydrate, mock_api_fetch, mock_record_hydrate
    ):
        mock_hydrate.return_value = None

        await _fetch_dir_contents("test-address")

        mock_api_fetch.assert_called_once_with("test-address")
        args, _ = mock_record_hydrate.call_args
        assert args[0] == "storage_api"


# --------------------------------------------------------------------------
# Reconnect bulk-hydration in init_cache_for_user (Bee toc / Epic .o7)
# --------------------------------------------------------------------------


@pytest.fixture
def mock_hydrate_all(mocker) -> AsyncMock:
    """
    Patch the bulk-hydration helper on the consuming module's bound
    reference. ``init_cache_for_user`` reads it via ``redis_cache.``
    inside `storage_permissions_cache`, so patching that name is what
    the call site actually sees.
    """
    return mocker.patch.object(
        storage_permissions_cache.redis_cache,
        "hydrate_all_dirs_from_redis",
        AsyncMock(return_value=0),
    )


@pytest.mark.asyncio
class TestInitCacheForUserReconnectHydration:
    """
    Reconnect path: when filters_used_in_request is empty, the
    consumer is reconnecting to a previously-used queue and must
    bulk-hydrate every dir Pod A had touched from Redis.
    """

    async def test_calls_hydrate_all_when_filters_empty(
        self, mock_hydrate_all, mock_fetch, cache_instance
    ):
        await init_cache_for_user(CACHE_SCOPE, [])

        mock_hydrate_all.assert_awaited_once_with(CACHE_SCOPE, cache_instance)

    async def test_does_not_fetch_per_filter_on_reconnect(
        self, mock_hydrate_all, mock_fetch
    ):
        await init_cache_for_user(CACHE_SCOPE, [])

        mock_fetch.assert_not_called()

    async def test_does_not_call_hydrate_all_when_filters_present(
        self, mock_hydrate_all, mock_fetch
    ):
        await init_cache_for_user(CACHE_SCOPE, filters())

        mock_hydrate_all.assert_not_called()

    async def test_does_not_call_hydrate_all_for_each_filter(
        self, mock_hydrate_all, mock_fetch
    ):
        # Sanity: the per-filter loop still drives _fetch_dir_contents,
        # not hydrate_all. Both ResourceFilters in `filters()` map to
        # one _fetch_dir_contents call each.
        await init_cache_for_user(CACHE_SCOPE, filters())

        assert mock_fetch.call_count == 2
        mock_hydrate_all.assert_not_called()

    async def test_cache_dict_unchanged_when_helper_returns_zero(
        self, mock_hydrate_all, mock_fetch, cache_instance
    ):
        # Pre-seed cache so we can verify nothing is added or removed
        # by the reconnect path when the helper finds nothing in Redis.
        cache_instance["storage-permissions:/preexisting/"] = DirectoryContents(
            file_paths=["a"], folder_paths=[], dir_path="/preexisting/"
        )
        snapshot = dict(cache_instance)
        mock_hydrate_all.return_value = 0

        await init_cache_for_user(CACHE_SCOPE, [])

        assert dict(cache_instance) == snapshot

    async def test_helper_mutates_cache_dict_returns_count(
        self, mock_hydrate_all, mock_fetch, cache_instance
    ):
        # The helper writes hydrated entries into cache_instance under
        # the same _cache_key shape that _fetch_dir_contents uses
        # ("storage-permissions:<dir_path>") — verified here by having
        # the mock perform that mutation and asserting it survives.
        hydrated = DirectoryContents.from_redis(
            authorized=True,
            files={"a"},
            folders=set(),
            expires_at=time.time() + 100,
            prev={},
            dir_path="/foo/",
        )

        async def fake_hydrate(scope, cd):
            cd["storage-permissions:/foo/"] = hydrated
            return 1

        mock_hydrate_all.side_effect = fake_hydrate

        await init_cache_for_user(CACHE_SCOPE, [])

        assert cache_instance["storage-permissions:/foo/"] is hydrated

    async def test_redis_disabled_branch_is_helpers_responsibility(
        self, mock_hydrate_all, mock_fetch, cache_instance, mocker
    ):
        # init_cache_for_user does not gate on settings.redis_enabled
        # at this layer — the helper short-circuits internally. Patch
        # via the module-import path so when this test grows in the
        # future to verify behavior under disabled Redis, the patch
        # lands on the live settings singleton (b.fwi).
        mocker.patch.object(
            storage_permissions_cache.settings.settings, "redis_enabled", False
        )
        mock_hydrate_all.return_value = 0

        await init_cache_for_user(CACHE_SCOPE, [])

        # Wired contract: helper is still called; it short-circuits
        # internally and returns 0. cache_dict stays unchanged.
        mock_hydrate_all.assert_awaited_once_with(CACHE_SCOPE, cache_instance)
        mock_fetch.assert_not_called()

    async def test_reconnect_logs_hydrated_count(
        self, mock_hydrate_all, mock_fetch, caplog
    ):
        # The Epic AC requires an INFO log on reconnect with the
        # hydrated-dir count, so operators can see whether reconnect
        # carried over state. Helper returns 7; assert log carries 7.
        #
        # Only the count. How that line NAMES the queue is a separate
        # contract -- it reports the redacted reference, never the broker
        # name, because the reconnect branch always holds one carrying the
        # client's token -- and is pinned in
        # TestInitCacheForUserReconnectLogHygiene below.
        import logging as _logging

        mock_hydrate_all.return_value = 7

        with caplog.at_level(
            _logging.INFO, logger=storage_permissions_cache.logger.name
        ):
            await init_cache_for_user(CACHE_SCOPE, [])

        info_records = [r for r in caplog.records if r.levelno == _logging.INFO]
        assert any(
            "7 directories" in record.getMessage() for record in info_records
        ), f"expected INFO log with count=7; saw {info_records}"


@pytest.mark.asyncio
class TestInitCacheForUserReconnectLogHygiene:
    """
    The reconnect summary line names the queue by redacted reference.

    That branch is reached only when a client presents a reconnect token,
    so the scope's queue name ALWAYS embeds one -- this is the log line
    most certain to carry a live token, and it runs once per reconnect at
    INFO, where the aggregator keeps it. The scope KEY carries the name
    plus the presenting principal's reference and is more sensitive still;
    only ``scope.queue_ref`` may be written down.
    """

    @pytest.fixture
    def reconnect_record(self, mock_hydrate_all, mock_fetch, cache_instance, caplog):
        """The INFO record the reconnect branch emits."""

        async def _run():
            with caplog.at_level(
                logging.INFO, logger=storage_permissions_cache.logger.name
            ):
                await init_cache_for_user(CACHE_SCOPE, [])

            return next(
                r for r in caplog.records if "reconnect hydrated" in r.getMessage()
            )

        return _run

    async def test_the_line_carries_the_redacted_reference(self, reconnect_record):
        record = await reconnect_record()

        assert redact_reconnect_token(RECONNECT_TOKEN) in record.getMessage()

    async def test_the_line_does_not_carry_the_token(self, reconnect_record):
        record = await reconnect_record()

        assert RECONNECT_TOKEN not in str(record.__dict__)

    async def test_the_redis_lookup_still_uses_the_exact_scope(
        self, reconnect_record, mock_hydrate_all, cache_instance
    ):
        # Redacting the log line must not redact the key: hydrating from a
        # reference would read a queue that has never existed.
        await reconnect_record()

        mock_hydrate_all.assert_awaited_once_with(CACHE_SCOPE, cache_instance)

    async def test_the_line_does_not_carry_the_principal_reference(
        self, reconnect_record
    ):
        # The scope key reaches Redis and the registry and nothing else;
        # an exported reference stays queue-only so logs, spans, and
        # metrics keep joining by exact match.
        record = await reconnect_record()

        assert CACHE_SCOPE.principal_ref not in str(record.__dict__)


@pytest.mark.asyncio
class TestFetchDirContentsLogHygiene:
    """
    Both hydration DEBUG lines name the queue by redacted reference.

    They fire on every directory a storage-filtered stream touches, which
    makes them the highest-volume place a reconnect token could have ended
    up in the log aggregator.
    """

    @pytest.fixture
    def non_durable_cache(self, mocker):
        """Bind the cache to a non-durable queue's broker name."""
        instance = StorageCache(CACHE_SCOPE)
        mocker.patch.object(StorageCache, "get_instance", return_value=instance)
        return instance

    @pytest.fixture
    def hydration_debug_records(self, caplog, non_durable_cache, mock_hydrate):
        """
        Drive one directory fetch and hand back the DEBUG lines it emitted.

        ``source`` selects which of the two branches runs: the Redis hit or
        the storage-API fallback. Both are exercised, because each writes
        its own line with its own ``extra``.
        """

        async def _run(source):
            if source == "redis":
                mock_hydrate.return_value = DirectoryContents.from_redis(
                    authorized=True,
                    files={"a.txt"},
                    folders=set(),
                    expires_at=time.time() + 100,
                    prev={},
                    dir_path="test-address",
                )

            with caplog.at_level(
                logging.DEBUG, logger=storage_permissions_cache.logger.name
            ):
                await _fetch_dir_contents("test-address")

            return [r for r in caplog.records if r.levelno == logging.DEBUG]

        return _run

    @pytest.mark.parametrize("source", ["redis", "storage_api"])
    async def test_a_hydration_line_is_emitted(self, source, hydration_debug_records):
        # Guards the guards below: an empty capture would make them vacuous.
        assert await hydration_debug_records(source) != []

    @pytest.mark.parametrize("source", ["redis", "storage_api"])
    async def test_the_hydration_line_carries_the_redacted_reference(
        self, source, hydration_debug_records
    ):
        records = await hydration_debug_records(source)

        assert all(
            record.queue_ref == redact_reconnect_token(RECONNECT_TOKEN)
            for record in records
        )

    @pytest.mark.parametrize("source", ["redis", "storage_api"])
    async def test_no_hydration_line_carries_the_token(
        self, source, hydration_debug_records
    ):
        records = await hydration_debug_records(source)

        assert [r for r in records if RECONNECT_TOKEN in str(r.__dict__)] == []


# --------------------------------------------------------------------------
# Exactly-once report of the events dropped against an unauthorized entry
# (b.54z). Each entry's count is reported once, when it leaves the stream's
# cache: by the fetch-time line when an unauthorized storage-API refetch
# replaces it, and by the drop-report line on every other way out.
# --------------------------------------------------------------------------


#: The listings that authorize a directory: a 200 and the storage API's
#: empty-listing 404.
AUTHORIZED_ANSWERS = [
    pytest.param(200, "test-msg", id="listed"),
    pytest.param(404, AZURE_EMPTY_LISTING_BODY, id="empty-listing"),
]


@pytest.fixture
def reports_of_the_drops(caplog):
    """
    Every captured record, of either message, that reports the ``DROPPED``
    events. Exactly-once means this holds one record.
    """
    caplog.set_level(logging.DEBUG, logger=storage_permissions_cache.logger.name)
    return lambda: [
        r for r in caplog.records if getattr(r, "dropped_events", None) == DROPPED
    ]


#: The outcomes an unauthorized entry can carry, each with the level its drop
#: report is logged at and the status and outcome extras it carries. ``None``
#: is an entry hydrated from Redis, whose outcome was not persisted.
UNAUTHORIZED_OUTCOMES = [
    pytest.param(ListingOutcome.FORBIDDEN, logging.INFO, 403, "forbidden", id="403"),
    pytest.param(
        ListingOutcome.UNSUPPORTED_ADDRESS,
        logging.WARNING,
        400,
        "unsupported_address",
        id="unsupported-400",
    ),
    pytest.param(
        ListingOutcome.UNRECOGNIZED_NOT_FOUND,
        logging.WARNING,
        404,
        "unrecognized_not_found",
        id="other-404",
    ),
    pytest.param(None, logging.WARNING, None, None, id="hydrated-from-redis"),
]


class TestReportDroppedEvents:
    """
    The drop-report line. Its extras are exported as log attributes when
    telemetry is on, so it names the stream by redacted reference and leaves
    the folder path out, as the fetch-time line does.
    """

    @pytest.fixture
    def report(self, drop_reports):
        """Report ``entry`` once and return the drop-report lines."""

        def _run(entry):
            storage_permissions_cache.report_dropped_events(CACHE_SCOPE, entry)
            return drop_reports()

        return _run

    @pytest.mark.parametrize("outcome, level, status, value", UNAUTHORIZED_OUTCOMES)
    def test_is_emitted_once(self, outcome, level, status, value, report):
        assert len(report(unauthorized_entry_with_drops(CLIENT_DIR, outcome))) == 1

    @pytest.mark.parametrize("outcome, level, status, value", UNAUTHORIZED_OUTCOMES)
    def test_level_follows_the_outcome(self, outcome, level, status, value, report):
        [record] = report(unauthorized_entry_with_drops(CLIENT_DIR, outcome))

        assert record.levelno == level

    @pytest.mark.parametrize("outcome, level, status, value", UNAUTHORIZED_OUTCOMES)
    def test_carries_the_storage_status(self, outcome, level, status, value, report):
        [record] = report(unauthorized_entry_with_drops(CLIENT_DIR, outcome))

        assert record.storage_status == status

    @pytest.mark.parametrize("outcome, level, status, value", UNAUTHORIZED_OUTCOMES)
    def test_carries_the_listing_outcome(self, outcome, level, status, value, report):
        [record] = report(unauthorized_entry_with_drops(CLIENT_DIR, outcome))

        assert record.listing_outcome == value

    def test_carries_the_drop_count(self, report):
        [record] = report(unauthorized_entry_with_drops(CLIENT_DIR))

        assert record.dropped_events == DROPPED

    def test_carries_the_redacted_queue_reference(self, report):
        [record] = report(unauthorized_entry_with_drops(CLIENT_DIR))

        assert record.queue_ref == redact_reconnect_token(RECONNECT_TOKEN)

    def test_does_not_carry_the_token(self, report):
        [record] = report(unauthorized_entry_with_drops(CLIENT_DIR))

        assert RECONNECT_TOKEN not in str(record.__dict__)

    def test_does_not_carry_the_principal_reference(self, report):
        [record] = report(unauthorized_entry_with_drops(CLIENT_DIR))

        assert CACHE_SCOPE.principal_ref not in str(record.__dict__)

    def test_does_not_name_the_directory(self, report):
        [record] = report(unauthorized_entry_with_drops(CLIENT_DIR))

        assert CLIENT_DIR not in str(record.__dict__)

    def test_is_not_emitted_when_no_events_were_dropped(self, report):
        entry = unauthorized_entry_with_drops(CLIENT_DIR, dropped=0)

        assert report(entry) == []

    def test_is_not_emitted_for_an_authorized_entry(self, report):
        # The readers never count a drop against an authorized entry; a
        # count found on one is not a report of dropped events.
        entry = DirectoryContents(file_paths=[], folder_paths=[], dir_path=CLIENT_DIR)
        entry.dropped_events = DROPPED

        assert report(entry) == []

    def test_a_second_report_for_the_same_entry_emits_nothing(self, report):
        entry = unauthorized_entry_with_drops(CLIENT_DIR)
        report(entry)

        assert len(report(entry)) == 1


class TestReportDroppedEventsAtStreamEnd:
    @pytest.fixture
    def ended_stream_cache(self, cache_instance):
        """
        A stream's cache at stream end: two unauthorized entries with drops
        (two and five) and one authorized entry.
        """
        cache_instance.update(
            {
                _cache_key("/denied/"): unauthorized_entry_with_drops(
                    "/denied/", dropped=2
                ),
                _cache_key("/misrouted/"): unauthorized_entry_with_drops(
                    "/misrouted/", ListingOutcome.UNRECOGNIZED_NOT_FOUND, dropped=5
                ),
                _cache_key("/listed/"): DirectoryContents(
                    file_paths=[], folder_paths=[], dir_path="/listed/"
                ),
            }
        )
        return cache_instance

    def test_reports_each_unauthorized_entry_with_drops(
        self, ended_stream_cache, drop_reports
    ):
        storage_permissions_cache.report_dropped_events_at_stream_end(
            ended_stream_cache
        )

        assert sorted(r.dropped_events for r in drop_reports()) == [2, 5]

    def test_leaves_the_entries_in_the_cache(self, ended_stream_cache):
        keys_before = set(ended_stream_cache)

        storage_permissions_cache.report_dropped_events_at_stream_end(
            ended_stream_cache
        )

        assert set(ended_stream_cache) == keys_before

    def test_a_second_report_emits_nothing(self, ended_stream_cache, drop_reports):
        storage_permissions_cache.report_dropped_events_at_stream_end(
            ended_stream_cache
        )
        storage_permissions_cache.report_dropped_events_at_stream_end(
            ended_stream_cache
        )

        assert len(drop_reports()) == 2

    def test_emits_nothing_for_an_empty_cache(self, cache_instance, drop_reports):
        storage_permissions_cache.report_dropped_events_at_stream_end(cache_instance)

        assert drop_reports() == []


@pytest.mark.asyncio
class TestFetchDirContentsUnauthorizedRefetchDropReport:
    """
    Exit 1: an unauthorized storage-API refetch replaces the entry. The
    fetch-time line carries the replaced entry's drops, and is their only
    report.
    """

    @pytest.fixture(autouse=True)
    def replaced_entry_has_drops(self, unauthorized_entry):
        unauthorized_entry.dropped_events = DROPPED
        return unauthorized_entry

    @pytest.mark.parametrize("status, body, level, outcome", UNAUTHORIZED_ANSWERS)
    async def test_reports_the_drops_exactly_once(
        self, status, body, level, outcome, storage_answers, reports_of_the_drops
    ):
        storage_answers(status, body)

        await _fetch_dir_contents(CLIENT_DIR)

        assert len(reports_of_the_drops()) == 1

    @pytest.mark.parametrize("status, body, level, outcome", UNAUTHORIZED_ANSWERS)
    async def test_does_not_also_emit_the_drop_report(
        self, status, body, level, outcome, storage_answers, drop_reports
    ):
        storage_answers(status, body)

        await _fetch_dir_contents(CLIENT_DIR)

        assert drop_reports() == []

    async def test_a_later_report_for_the_replaced_entry_emits_nothing(
        self, storage_answers, replaced_entry_has_drops, drop_reports
    ):
        storage_answers(403)
        await _fetch_dir_contents(CLIENT_DIR)

        storage_permissions_cache.report_dropped_events(
            CACHE_SCOPE, replaced_entry_has_drops
        )

        assert drop_reports() == []


@pytest.mark.asyncio
class TestFetchDirContentsAuthorizedRefetchDropReport:
    """
    Exit 2: an authorized storage-API refetch replaces an unauthorized entry.
    No fetch-time line is written for an authorized listing, so the
    drop-report line reports the replaced entry's drops.
    """

    @pytest.mark.parametrize("status, body", AUTHORIZED_ANSWERS)
    async def test_reports_the_replaced_entrys_drops(
        self, status, body, storage_answers, unauthorized_entry, drop_reports
    ):
        unauthorized_entry.dropped_events = DROPPED
        storage_answers(status, body)

        await _fetch_dir_contents(CLIENT_DIR)

        [record] = drop_reports()
        assert record.dropped_events == DROPPED

    @pytest.mark.parametrize("status, body", AUTHORIZED_ANSWERS)
    async def test_reports_the_drops_exactly_once(
        self, status, body, storage_answers, unauthorized_entry, reports_of_the_drops
    ):
        unauthorized_entry.dropped_events = DROPPED
        storage_answers(status, body)

        await _fetch_dir_contents(CLIENT_DIR)

        assert len(reports_of_the_drops()) == 1

    async def test_describes_the_replaced_entry_not_the_new_listing(
        self, storage_answers, unauthorized_entry, drop_reports
    ):
        # The drops were against the 403 entry, so the line carries its
        # outcome and level, not the listing that replaced it.
        unauthorized_entry.dropped_events = DROPPED
        storage_answers(200)

        await _fetch_dir_contents(CLIENT_DIR)

        [record] = drop_reports()
        assert (record.listing_outcome, record.levelno) == ("forbidden", logging.INFO)

    @pytest.mark.parametrize("status, body", AUTHORIZED_ANSWERS)
    async def test_reports_nothing_when_no_events_were_dropped(
        self, status, body, storage_answers, unauthorized_entry, drop_reports
    ):
        storage_answers(status, body)

        await _fetch_dir_contents(CLIENT_DIR)

        assert drop_reports() == []


@pytest.mark.asyncio
class TestFetchDirContentsRedisHydrationDropReport:
    """
    Exit 3: Redis hydration in _fetch_dir_contents replaces an unauthorized
    entry. Hydration writes no fetch-time line, whatever the hydrated entry
    says, so the drop-report line reports the replaced entry's drops.
    """

    @pytest.fixture(params=[True, False], ids=["authorized", "unauthorized"])
    def redis_hydrates(self, request, mock_hydrate, mock_api_fetch):
        """Make Redis answer for ``CLIENT_DIR`` with a live entry."""
        mock_hydrate.return_value = DirectoryContents.from_redis(
            authorized=request.param,
            files=set() if request.param else None,
            folders=set() if request.param else None,
            expires_at=time.time() + 100,
            prev={},
            dir_path=CLIENT_DIR,
        )

    async def test_reports_the_replaced_entrys_drops(
        self, redis_hydrates, unauthorized_entry, drop_reports
    ):
        unauthorized_entry.dropped_events = DROPPED

        await _fetch_dir_contents(CLIENT_DIR)

        [record] = drop_reports()
        assert record.dropped_events == DROPPED

    async def test_reports_the_drops_exactly_once(
        self, redis_hydrates, unauthorized_entry, reports_of_the_drops
    ):
        unauthorized_entry.dropped_events = DROPPED

        await _fetch_dir_contents(CLIENT_DIR)

        assert len(reports_of_the_drops()) == 1

    async def test_reports_nothing_when_no_events_were_dropped(
        self, redis_hydrates, unauthorized_entry, drop_reports
    ):
        await _fetch_dir_contents(CLIENT_DIR)

        assert drop_reports() == []


@pytest.mark.asyncio
class TestFetchDirContentsDropReportRace:
    """
    A fetch reads the cache, then awaits Redis or the storage API. While it
    waits, another task on the stream installs an unauthorized entry under
    the same key and the readers drop events against it. That entry is the
    one the waiting fetch's install replaces, so its drops are reported once
    when that install lands, not lost to the entry the fetch first read.
    """

    @pytest.fixture
    def racing_install(self, mock_api_fetch, mock_hydrate, cache_instance):
        """
        Start a fetch of ``CLIENT_DIR`` that parks on ``waits_on`` ("redis"
        or "storage_api"), complete a second fetch of the same directory
        that installs a 403 entry, drop ``DROPPED`` events against that
        entry, then let the parked fetch install ``parked_result``.
        """

        async def _run(waits_on, parked_result):
            parked = asyncio.Event()
            release = asyncio.Event()
            racing_entry = DirectoryContents(
                dir_path=CLIENT_DIR, outcome=ListingOutcome.FORBIDDEN
            )

            async def _park_first_call(*_args):
                if parked.is_set():
                    return None if waits_on == "redis" else racing_entry
                parked.set()
                await release.wait()
                return parked_result

            if waits_on == "redis":
                mock_hydrate.side_effect = _park_first_call
                mock_api_fetch.return_value = racing_entry
            else:
                mock_api_fetch.side_effect = _park_first_call

            parked_fetch = asyncio.create_task(_fetch_dir_contents(CLIENT_DIR))
            await asyncio.wait_for(parked.wait(), timeout=1.0)

            installed = await _fetch_dir_contents(CLIENT_DIR)
            assert installed is racing_entry
            installed.dropped_events += DROPPED

            release.set()
            await asyncio.wait_for(parked_fetch, timeout=1.0)

        return _run

    @staticmethod
    def _listed():
        return DirectoryContents(file_paths=[], folder_paths=[], dir_path=CLIENT_DIR)

    @staticmethod
    def _forbidden():
        return DirectoryContents(dir_path=CLIENT_DIR, outcome=ListingOutcome.FORBIDDEN)

    @staticmethod
    def _hydrated():
        return DirectoryContents.from_redis(
            authorized=True,
            files=set(),
            folders=set(),
            expires_at=time.time() + 100,
            prev={},
            dir_path=CLIENT_DIR,
        )

    async def test_an_authorized_storage_api_install_reports_the_drops_once(
        self, racing_install, reports_of_the_drops
    ):
        await racing_install("storage_api", self._listed())

        assert len(reports_of_the_drops()) == 1

    async def test_an_authorized_storage_api_install_reports_them_as_drops(
        self, racing_install, reports_of_the_drops
    ):
        await racing_install("storage_api", self._listed())

        [record] = reports_of_the_drops()
        assert record.getMessage() == DROP_REPORT_MESSAGE

    async def test_an_unauthorized_storage_api_install_reports_the_drops_once(
        self, racing_install, reports_of_the_drops
    ):
        await racing_install("storage_api", self._forbidden())

        assert len(reports_of_the_drops()) == 1

    async def test_an_unauthorized_storage_api_install_reports_them_at_fetch_time(
        self, racing_install, reports_of_the_drops
    ):
        await racing_install("storage_api", self._forbidden())

        [record] = reports_of_the_drops()
        assert record.getMessage() == UNAUTHORIZED_LISTING_MESSAGE

    async def test_a_redis_install_reports_the_drops_once(
        self, racing_install, reports_of_the_drops
    ):
        await racing_install("redis", self._hydrated())

        assert len(reports_of_the_drops()) == 1


@pytest.mark.asyncio
class TestInitCacheForUserDropReport:
    """
    Exit 4: a filter update that no longer names a directory removes its
    entry from the stream's cache, and the entry's drops are reported then.
    """

    #: A directory no filter in ``filters()`` names.
    DROPPED_DIR = "/no-longer-filtered/"

    async def test_reports_the_removed_entrys_drops(self, cache_instance, drop_reports):
        cache_instance[_cache_key(self.DROPPED_DIR)] = unauthorized_entry_with_drops(
            self.DROPPED_DIR
        )

        await init_cache_for_user(CACHE_SCOPE, filters())

        [record] = drop_reports()
        assert record.dropped_events == DROPPED

    async def test_reports_nothing_for_a_removed_entry_without_drops(
        self, cache_instance, drop_reports
    ):
        cache_instance[_cache_key(self.DROPPED_DIR)] = unauthorized_entry_with_drops(
            self.DROPPED_DIR, dropped=0
        )

        await init_cache_for_user(CACHE_SCOPE, filters())

        assert drop_reports() == []

    async def test_does_not_report_an_entry_a_filter_still_names(
        self, cache_instance, drop_reports
    ):
        # The entry stays, so its drops are reported when it does leave.
        cache_instance[_cache_key("a/")] = unauthorized_entry_with_drops("a/")

        await init_cache_for_user(CACHE_SCOPE, filters())

        assert drop_reports() == []
