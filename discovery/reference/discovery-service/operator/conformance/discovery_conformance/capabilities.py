# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
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

"""Wire-level checks of an explicitly selected backend CapabilitiesService."""

from urllib.parse import urlsplit

import grpc

from .generated.nvidia.omniverse.discovery.capabilities.v2beta import (
    capabilities_service_pb2,
    capabilities_service_pb2_grpc,
)


def check_capabilities(
    target: str,
    *,
    token: str | None = None,
    ca_bundle: str | bool = True,
    timeout: float = 10,
    plaintext: bool = False,
    max_pages: int = 100,
    page_size: int = 100,
) -> list[dict]:
    """Check protobuf decoding and bounded, complete inventory enumeration.

    The target is a backend authority (host:port), optionally an http(s) URL.
    Every RPC has its own deadline. No advertised application RPC is invoked.
    """
    wire_id = "capabilities.grpc.wire"
    inventory_id = "capabilities.grpc.inventory"

    def result(check_id, status, message):
        return {"id": check_id, "status": status, "message": message}

    def failure(status, message, wire_passed=False):
        return [
            result(wire_id, "PASS" if wire_passed else status,
                   "ListServices response decoded successfully." if wire_passed else message),
            result(inventory_id, status, message),
        ]

    try:
        if not target or timeout <= 0 or max_pages < 1 or not 0 <= page_size <= 2147483647:
            raise ValueError("invalid configuration")
        if "://" in target:
            parsed = urlsplit(target)
            if (parsed.scheme not in {"http", "https"} or not parsed.hostname
                    or parsed.username is not None or parsed.password is not None
                    or parsed.path not in {"", "/"} or parsed.query or parsed.fragment
                    or (parsed.scheme == "http" and not plaintext)
                    or (parsed.scheme == "https" and plaintext)):
                raise ValueError("invalid backend URL")
            authority = parsed.netloc
        else:
            if any(c in target for c in "/?#@"):
                raise ValueError("invalid backend authority")
            authority = target
        if token is not None and plaintext:
            raise ValueError("Bearer credentials require TLS")
        if ca_bundle is False:
            raise ValueError("TLS verification cannot be disabled")
        roots = None
        if isinstance(ca_bundle, str) and not plaintext:
            with open(ca_bundle, "rb") as bundle:
                roots = bundle.read()
        metadata = (("authorization", f"Bearer {token}"),) if token is not None else ()
        channel = (grpc.insecure_channel(authority) if plaintext else
                   grpc.secure_channel(authority, grpc.ssl_channel_credentials(root_certificates=roots)))
    except Exception:
        return failure("INCONCLUSIVE", "Invalid or unreadable gRPC check configuration.")

    decoded = False
    count = 0
    seen = set()
    page_token = ""
    try:
        with channel:
            stub = capabilities_service_pb2_grpc.CapabilitiesServiceStub(channel)
            for _ in range(max_pages):
                response = stub.ListServices(
                    capabilities_service_pb2.ListServicesRequest(
                        page_size=page_size, page_token=page_token),
                    timeout=timeout, metadata=metadata,
                )
                decoded = True
                count += len(response.services)
                page_token = response.next_page_token
                if not page_token:
                    return [
                        result(wire_id, "PASS", "ListServices response decoded successfully."),
                        result(inventory_id, "PASS", f"Complete inventory: {count} services."),
                    ]
                if page_token in seen:
                    return failure("FAIL", "Pagination returned a repeated token.", decoded)
                seen.add(page_token)
            return failure("INCONCLUSIVE", "Pagination exceeded the configured page bound.", decoded)
    except grpc.RpcError as error:
        code = error.code()
        status = "FAIL" if code == grpc.StatusCode.UNIMPLEMENTED else "INCONCLUSIVE"
        return failure(status, f"ListServices RPC did not complete ({code.name}).", decoded)
    except Exception:
        return failure("FAIL", "ListServices response could not be processed.", decoded)
