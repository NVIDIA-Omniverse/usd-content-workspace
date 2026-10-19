# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""
Error bodies the storage API's directory-listing endpoint answers with.

The consumer classifies a non-200 listing by status and, for a 404 or a
400, by the body's ``detail`` text. These are the bodies the storage-permission
tests feed it, spelled as the storage service sends them (an RFC 9457 problem
document) so a test exercises the same parse the production path does.
"""

import json


def _problem(status: int, title: str, detail: str) -> str:
    return json.dumps(
        {"type": "about:blank", "status": status, "title": title, "detail": detail}
    )


#: The Azure backend's answer for a prefix that holds no blobs.
AZURE_EMPTY_LISTING_BODY = _problem(
    404, "Not Found", "not found: No elements found for folder address"
)

#: The S3 backend's answer for a prefix that holds no blobs.
S3_EMPTY_LISTING_BODY = _problem(
    404, "Not Found", "sdk error source=not found: No elements found"
)

#: The empty-listing answer as a bare text body rather than JSON.
PLAIN_TEXT_EMPTY_LISTING_BODY = "not found: No elements found for folder address"

#: A 404 that is not the empty-listing answer: a missing container.
CONTAINER_NOT_FOUND_BODY = _problem(404, "Not Found", "ContainerNotFound")

#: The 400 storage sends when an address routes to no configured backend.
UNSUPPORTED_ADDRESS_BODY = _problem(
    400, "Bad Request", "resource address is not supported"
)

#: Every empty-listing body form the consumer must accept as authorized.
EMPTY_LISTING_BODIES = [
    AZURE_EMPTY_LISTING_BODY,
    S3_EMPTY_LISTING_BODY,
    PLAIN_TEXT_EMPTY_LISTING_BODY,
]
EMPTY_LISTING_IDS = ["azure", "s3", "plain-text"]

#: 404 bodies that must stay unauthorized: an empty body (an unmatched route
#: or a gateway) and a JSON detail that is not the empty-listing answer.
UNRECOGNIZED_NOT_FOUND_BODIES = ["", CONTAINER_NOT_FOUND_BODY]
UNRECOGNIZED_NOT_FOUND_IDS = ["empty-body", "container-not-found"]
