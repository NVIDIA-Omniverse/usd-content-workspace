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

import requests
import os
import logging
import sys
import json
from pydantic import BaseModel, Field, ValidationError, field_validator
from pydantic import HttpUrl
from pydantic import model_validator

logger = logging.getLogger(__name__)
logger.setLevel(logging.INFO)

# Add handler to print logs to stdout
handler = logging.StreamHandler(sys.stdout)
formatter = logging.Formatter('%(message)s')
handler.setFormatter(formatter)
logger.addHandler(handler)


PASS = "✅"
FAIL = "❌"
INFO = "ℹ️"
ROCKET = "🚀"
CHECK = "✔"
HINT = "💡"


class Service(BaseModel):
    id: str
    name: str
    type: str
    grpc: HttpUrl | None = None
    rest: HttpUrl | None = None

    model_config = {
        "extra": "forbid",
        "populate_by_name": False,
    }

    @model_validator(mode="after")
    def require_one(cls, m):
        # If both are None → raise ValidationError like a missing field
        if not (m.grpc or m.rest):
            raise ValueError("Either 'grpc' or 'rest' endpoints must be provided")
        return m


class DiscoveryPayload(BaseModel):
    schema_version: int = Field(alias="schema-version")
    services: list[Service]

    @field_validator("schema_version")
    @classmethod
    def _schema_version_must_be_1(cls, v: int) -> int:
        if v != 1:
            raise payload_invalid("schema-version must be 1")
        return v

    @field_validator("services")
    @classmethod
    def _services_non_empty(cls, v: list[Service]) -> list[Service]:
        if not v:
            raise payload_invalid("services must be a non-empty list")
        return v


def test_discovery_service(url: str, endpoint: str):
    try:
        response = requests.get(f"{url}{endpoint}", timeout=10)
    except requests.exceptions.ConnectionError as e:
        raise connection_error(f"{e}")
    except requests.exceptions.Timeout as e:
        raise connection_error(f"{e}")
    except requests.exceptions.RequestException as e:
        raise bad_request(f"{e}")
    except Exception as e:
        raise unexpected_error(f"when fetching {url}{endpoint} {e}")

    match response.status_code:
        case 200:
            logger.info(f"Discovery service successfully fetched at {url}{endpoint} {CHECK}")
        case 400:
            raise bad_request(response.text)
        case 404:
            raise not_found()
        case 503:
            raise service_unavailable()
        case 500:
            raise internal_error(response.text)
        case _:
            raise unexpected_error(f"status code: {response.status_code}, details: {response.text}")
    try:
        payload = response.json()
    except requests.exceptions.JSONDecodeError as e:
        logger.error(f"response text: {response.text}")
        raise payload_invalid(f"response is not valid JSON: {e}")
    validate_discovery_payload(payload)
    logger.info(f"Payload: {INFO}\n {json.dumps(payload, indent=4)}")
    logger.info(f"Payload has been validated successfully {CHECK}")


def validate_discovery_payload(payload: object) -> None:
    try:
        DiscoveryPayload.model_validate(payload)
    except ValidationError as e:
        errors = []
        for err in e.errors():
            errors.append(f"{err['msg']} at {'.'.join(map(str, err['loc']))}")
        raise payload_invalid(errors)


class ErrorMessage(Exception):
    def __init__(self, message: str, hint: str):
        self.message = message
        self.hint = hint


def unexpected_error(details: str) -> ErrorMessage:
    message = f"Unexpected error: {details}"
    hint = "This may indicate a bug in the smoke test or an incompatible API version"
    return ErrorMessage(message, hint)


def bad_request(details: str) -> ErrorMessage:
    message = f"Invalid request (400 Bad Request): {details}"
    hint = "This may indicate:\n"\
           "• A bug in the smoke test or incompatible API version\n"
    return ErrorMessage(message, hint)


def connection_error(details: str) -> ErrorMessage:
    message = f"Connection error: {details}"
    hint = "Check that:\n"\
           f"• {HINT} The service is running and accessible\n"\
           f"• {HINT} The service has finished starting up\n"\
           f"• {HINT} The service is listening on the correct port\n"\
           f"• {HINT} Health checks are passing: GET /health\n"
    return ErrorMessage(message, hint)


def service_unavailable() -> ErrorMessage:    
    message = "Service unavailable (503)"
    hint = "The service is not ready. Check that:\n"\
            f"• {HINT} The service has finished starting up\n"\
            f"• {HINT} Health checks are passing: GET /health\n"
    return ErrorMessage(message, hint)


def internal_error(details: str) -> ErrorMessage:
    message = f"Service error (500): {details}"
    hint = "Check that:\n"\
           f"• {HINT} Check logs: kubectl logs -l app.kubernetes.io/name=discovery-service"
    return ErrorMessage(message, hint)


def not_found() -> ErrorMessage:
    message = "Not found (404)"
    hint = "Check that:\n"\
           f"• {HINT} The resource was created successfully in a previous step\n"
    return ErrorMessage(message, hint)


def payload_invalid(messages: list[str]) -> ErrorMessage:
    message = f"Payload is invalid{messages}\n"
    hint = "Check that:\n"\
           f"• {HINT} 'discovery.services' section in values is configured correctly\n"\
           f"• {HINT} check configmap: 'kubectl get configmap -n <namespace> discovery-service -o yaml'\n"
    return ErrorMessage(message, hint)


def run_test(url: str, endpoint: str) -> bool:
    logger.info(f"{ROCKET} Running test for endpoint {endpoint}...")
    logger.info("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
    try:
        test_discovery_service(url, endpoint)
        logger.info(f"Test for endpoint {endpoint} has been completed successfully {PASS}")
    except ErrorMessage as e:
        logger.error(f"Test for endpoint {endpoint} has failed: {e.message} {FAIL}")
        logger.error(f"{e.hint}")
        return False
    return True


if __name__ == "__main__":
    logger.info("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
    logger.info(f"{ROCKET} Discovery service smoke tests...{ROCKET}")
    logger.info("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
    logger.info("\n\n")
    service_url = os.getenv("SERVICE_URL")
    if not service_url:
        logger.error(f"Please set the SERVICE_URL environment variable {FAIL}")
        sys.exit(1)
    passed_tests = 0
    passed_tests += run_test(service_url, "/api/v1alpha/services")
    logger.info("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
    logger.info("\n\n")
    passed_tests += run_test(service_url, "/api/v1/services")
    logger.info("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
    logger.info("\n\n")
    if passed_tests == 2:
        logger.info(f"All tests have been completed successfully {PASS}")
    else:
        logger.info(f"Some tests have failed {FAIL}")
        sys.exit(1)
