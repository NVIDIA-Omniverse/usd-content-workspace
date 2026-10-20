# SPDX-FileCopyrightText: Copyright (c) 2024-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

import os
from typing import (
    Any,
    Dict,
    List,
    Optional,
    Tuple,
)
from urllib.parse import quote_plus

import requests

# Default timeout for HTTP requests (configurable via environment variable)
DEFAULT_TIMEOUT = int(os.getenv("STORAGEAPI_TEST_HTTP_TIMEOUT", "60"))


class RestClient:
    def __init__(self, base_url: str):
        self._base_url = base_url

    def list_services(self) -> requests.Response:
        for version in ("v1alpha", "v1beta", "v1"):
            resp = requests.get(
                f"{self._base_url}/{version}/capabilities/services",
                timeout=DEFAULT_TIMEOUT,
            )
            if resp.status_code != 404:
                return resp
        return resp  # last response (likely 404 from v1)

    def enumerate_data_objects(self, address: str, fileobject_version: str, **kwargs) -> requests.Response:
        kwargs.setdefault("timeout", DEFAULT_TIMEOUT)
        return requests.get(f"{self._base_url}/{fileobject_version}/fileobject/data-objects/{quote_plus(address)}", **kwargs)

    def stat_data_object(self, address: str, fileobject_version: str, **kwargs) -> requests.Response:
        kwargs.setdefault("timeout", DEFAULT_TIMEOUT)
        return requests.head(f"{self._base_url}/{fileobject_version}/fileobject/by-address/{quote_plus(address)}", **kwargs)

    def read_data_object_by_address(self, address: str, fileobject_version: str, **kwargs) -> requests.Response:
        kwargs.setdefault("timeout", DEFAULT_TIMEOUT)
        return requests.get(f"{self._base_url}/{fileobject_version}/fileobject/by-address/{quote_plus(address)}", **kwargs)

    def read_data_object_by_identity(self, identity: str, fileobject_version: str, **kwargs) -> requests.Response:
        kwargs.setdefault("timeout", DEFAULT_TIMEOUT)
        return requests.get(f"{self._base_url}/{fileobject_version}/fileobject/by-identity/{quote_plus(identity)}", **kwargs)

    def get_upload_options(self, address: str, write_version: str, **kwargs) -> requests.Response:
        kwargs.setdefault("timeout", DEFAULT_TIMEOUT)
        return requests.get(f"{self._base_url}/{write_version}/fileobject/upload-options/by-address/{quote_plus(address)}", **kwargs)

    def get_native_metadata_support(self, address: str, fileobject_version: str, **kwargs) -> requests.Response:
        kwargs.setdefault("timeout", DEFAULT_TIMEOUT)
        return requests.get(
            f"{self._base_url}/{fileobject_version}/fileobject/native-metadata-support/{quote_plus(address)}",
            **kwargs,
        )

    def is_read_only(self, resource_address: str, fileobject_version: str, **kwargs) -> requests.Response:
        kwargs.setdefault("timeout", DEFAULT_TIMEOUT)
        return requests.get(
            f"{self._base_url}/{fileobject_version}/fileobject/is-read-only/{quote_plus(resource_address)}",
            **kwargs,
        )

    def write_data_object(self, address: str, write_version: str, **kwargs) -> requests.Response:
        kwargs.setdefault("timeout", DEFAULT_TIMEOUT)
        return requests.put(f"{self._base_url}/{write_version}/fileobject/by-address/{quote_plus(address)}", **kwargs)

    def complete_redirect_upload(self, address: str, additional_headers: List[Tuple[str, str]], write_version: str) -> requests.Response:
        json_message = {"additional_headers": [{"name": h[0], "value": h[1]} for h in additional_headers]}
        return requests.post(
            f"{self._base_url}/{write_version}/fileobject/by-address/{quote_plus(address)}/redirect/complete",
            json=json_message,
            timeout=DEFAULT_TIMEOUT,
        )

    def create_multipart_part_upload_request(self, address: str, multipart_version: str, **kwargs) -> requests.Response:
        kwargs.setdefault("timeout", DEFAULT_TIMEOUT)
        return requests.post(
            f"{self._base_url}/{multipart_version}/fileobject/by-address/{quote_plus(address)}/multipart/prepare", **kwargs
        )

    def complete_multipart_upload(self, address: str, multipart_version: str, **kwargs) -> requests.Response:
        kwargs.setdefault("timeout", DEFAULT_TIMEOUT)
        return requests.post(
            f"{self._base_url}/{multipart_version}/fileobject/by-address/{quote_plus(address)}/multipart/complete", **kwargs
        )

    def abort_multipart_upload(self, address: str, upload_id: str, multipart_version: str, **kwargs) -> requests.Response:
        kwargs.setdefault("timeout", DEFAULT_TIMEOUT)
        return requests.post(
            f"{self._base_url}/{multipart_version}/fileobject/by-address/{quote_plus(address)}/multipart/abort",
            json={"upload_id": upload_id},
            **kwargs,
        )

    def enumerate_versions(self, address: str, versioning_version: str, **kwargs) -> requests.Response:
        kwargs.setdefault("timeout", DEFAULT_TIMEOUT)
        return requests.get(f"{self._base_url}/{versioning_version}/versioning/{quote_plus(address)}/versions", **kwargs)

    def is_versioned(self, resource_address: str, versioning_version: str, **kwargs) -> requests.Response:
        kwargs.setdefault("timeout", DEFAULT_TIMEOUT)
        return requests.get(
            f"{self._base_url}/{versioning_version}/versioning/is-versioned/{quote_plus(resource_address)}",
            **kwargs,
        )

    def list_data_objects(self, address: str, filefolder_version: str, **kwargs) -> requests.Response:
        kwargs.setdefault("timeout", DEFAULT_TIMEOUT)
        return requests.get(f"{self._base_url}/{filefolder_version}/filefolder/list/{quote_plus(address)}", **kwargs)

    def list_stat_data_objects(self, address: str, filefolder_version: str, **kwargs) -> requests.Response:
        kwargs.setdefault("timeout", DEFAULT_TIMEOUT)
        return requests.get(f"{self._base_url}/{filefolder_version}/filefolder/liststat/{quote_plus(address)}", **kwargs)

    def delete_data_object(self, address: str, fileobject_version: str, **kwargs) -> requests.Response:
        kwargs.setdefault("timeout", DEFAULT_TIMEOUT)
        return requests.delete(f"{self._base_url}/{fileobject_version}/fileobject/by-address/{quote_plus(address)}", **kwargs)

    def copy_object(
        self,
        source_identity: str,
        destination_address: str,
        previous_version: Optional[str],
        fileobject_version: str,
        fail_if_exists: Optional[bool] = None,
        **kwargs,
    ) -> requests.Response:
        json_message: Dict[str, Any] = {"destination_resource_address": destination_address}
        if previous_version is not None:
            json_message["previous_version"] = previous_version
        if fail_if_exists is not None:
            json_message["fail_if_exists"] = fail_if_exists
        kwargs.setdefault("timeout", DEFAULT_TIMEOUT)
        return requests.post(
            f"{self._base_url}/{fileobject_version}/fileobject/by-identity/{quote_plus(source_identity)}/copy", json=json_message, **kwargs
        )

    def delete_folder(self, address: str, filefolder_version: str, **kwargs) -> requests.Response:
        kwargs.setdefault("timeout", DEFAULT_TIMEOUT)
        return requests.delete(f"{self._base_url}/{filefolder_version}/filefolder/list/{quote_plus(address)}", **kwargs)

    def create_folder(self, address: str, filefolder_version: str, **kwargs) -> requests.Response:
        """Create a folder at the given address. Returns 201 when created, 200 if already exists (idempotent)."""
        kwargs.setdefault("timeout", DEFAULT_TIMEOUT)
        return requests.put(f"{self._base_url}/{filefolder_version}/filefolder/{quote_plus(address)}", **kwargs)

    def get_folder_mode(self, folder_address: str, filefolder_version: str, **kwargs) -> requests.Response:
        kwargs.setdefault("timeout", DEFAULT_TIMEOUT)
        return requests.get(f"{self._base_url}/{filefolder_version}/filefolder/get-folder-mode/{quote_plus(folder_address)}", **kwargs)

    def list_top_level_addresses(self, capabilities_version: str, **kwargs) -> requests.Response:
        kwargs.setdefault("timeout", DEFAULT_TIMEOUT)
        return requests.get(f"{self._base_url}/{capabilities_version}/capabilities/top-level-addresses", **kwargs)

    def move_data_object(self, source_address: str, move_version: str, **kwargs) -> requests.Response:
        kwargs.setdefault("timeout", DEFAULT_TIMEOUT)
        return requests.post(
            f"{self._base_url}/{move_version}/fileobject/by-address/{quote_plus(source_address)}/move",
            **kwargs,
        )

    def get_metadata(self, address: str, metadata_version: str, metadata_keys: Optional[List[str]] = None, **kwargs) -> requests.Response:
        """Fetch metadata keys for a resource via REST.

        This endpoint expects a JSON array body (e.g. `["key1", "key2"]`).
        To avoid accidental form/string payloads that trigger 422 validation
        errors, callers should pass `metadata_keys` or an explicit `json=...`
        kwarg.
        """
        kwargs.setdefault("timeout", DEFAULT_TIMEOUT)
        if metadata_keys is not None:
            if "json" in kwargs or "data" in kwargs:
                raise ValueError("metadata_keys cannot be combined with explicit body kwargs")
            kwargs["json"] = metadata_keys
        elif "data" in kwargs and "json" not in kwargs:
            raise ValueError("get_metadata requires JSON body; use metadata_keys=... or json=...")
        return requests.post(f"{self._base_url}/{metadata_version}/metadata/{quote_plus(address)}", **kwargs)

    def update_metadata(self, address: str, key: str, metadata_version: str, **kwargs) -> requests.Response:
        """Update a metadata key via REST with a JSON value body.

        This endpoint expects a JSON value body (string/number/object/etc).
        Callers should pass `json=...` so requests sets `Content-Type:
        application/json`. Passing raw `data=...` without `json` can trigger
        415 on strict deployments.
        """
        kwargs.setdefault("timeout", DEFAULT_TIMEOUT)
        if "data" in kwargs and "json" not in kwargs:
            raise ValueError("update_metadata requires JSON body; use json=...")
        return requests.put(f"{self._base_url}/{metadata_version}/metadata/{quote_plus(address)}/{quote_plus(key)}", **kwargs)

    def delete_metadata(self, address: str, key: str, metadata_version: str, **kwargs) -> requests.Response:
        kwargs.setdefault("timeout", DEFAULT_TIMEOUT)
        return requests.delete(f"{self._base_url}/{metadata_version}/metadata/{quote_plus(address)}/{quote_plus(key)}", **kwargs)

    def update_metadata_batch(self, metadata_version: str, **kwargs) -> requests.Response:
        """Apply a batch of metadata updates via REST.

        The body must be a JSON object of the form
        ``{"requests": [{"resource_location": {...}, "user_metadata_key": ...,
        "user_metadata": ..., "expected_etag": ...}, ...]}`` and is passed in
        via ``json=...``. Each response entry is correlated to its originating
        request by its zero-based ``request_index`` -- the position of the
        entry in the input ``requests`` array.
        """
        kwargs.setdefault("timeout", DEFAULT_TIMEOUT)
        if "data" in kwargs:
            raise ValueError("update_metadata_batch requires JSON body; use json=...")
        return requests.post(f"{self._base_url}/{metadata_version}/metadata/", **kwargs)

    def get_metadata_batch(self, metadata_version: str, **kwargs) -> requests.Response:
        """Fetch a batch of metadata values via REST.

        Body shape mirrors ``update_metadata_batch`` but each entry only carries
        a ``resource_location`` and ``user_metadata_key``.
        """
        kwargs.setdefault("timeout", DEFAULT_TIMEOUT)
        if "data" in kwargs:
            raise ValueError("get_metadata_batch requires JSON body; use json=...")
        return requests.post(f"{self._base_url}/{metadata_version}/metadata/get", **kwargs)

    def delete_metadata_batch(self, metadata_version: str, **kwargs) -> requests.Response:
        """Delete a batch of metadata values via REST."""
        kwargs.setdefault("timeout", DEFAULT_TIMEOUT)
        if "data" in kwargs:
            raise ValueError("delete_metadata_batch requires JSON body; use json=...")
        return requests.delete(f"{self._base_url}/{metadata_version}/metadata/", **kwargs)

    def query_metadata(self, metadata_version: str, **kwargs) -> requests.Response:
        """Query all metadata values for a resource via REST.

        Body shape is ``{"resource_location": {...}, "namespace": ...,
        "max_page_size": ..., "continuation_handle": ...}``.
        """
        kwargs.setdefault("timeout", DEFAULT_TIMEOUT)
        if "data" in kwargs:
            raise ValueError("query_metadata requires JSON body; use json=...")
        return requests.post(f"{self._base_url}/{metadata_version}/metadata/query", **kwargs)
