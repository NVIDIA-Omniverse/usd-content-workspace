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

"""Customer entry point. Only explicitly supplied endpoints are contacted."""

import argparse
import hashlib
import json
import math
from importlib.resources import files
from pathlib import Path
from urllib.parse import urlsplit

from .reports import exit_code, write_reports


def positive_float(value):
    number = float(value)
    if not math.isfinite(number) or number <= 0:
        raise argparse.ArgumentTypeError("must be a finite positive number")
    return number


def positive_int(value):
    number = int(value)
    if number <= 0:
        raise argparse.ArgumentTypeError("must be positive")
    return number


def http_url(value):
    try:
        url = urlsplit(value)
        valid = url.scheme in {"http", "https"} and url.hostname and url.port != 0
    except ValueError:
        valid = False
    if not valid or url.username is not None or url.password is not None or url.query or url.fragment:
        raise argparse.ArgumentTypeError("use an absolute HTTP(S) URL without credentials, query or fragment")
    return value


def grpc_target(value):
    try:
        target = urlsplit("//" + value)
        valid = (target.hostname and target.port and not target.path and
                 target.username is None and target.password is None and
                 not target.query and not target.fragment)
    except ValueError:
        valid = False
    if not valid:
        raise argparse.ArgumentTypeError("use a host:port without a URL scheme or credentials")
    return value


def parser():
    result = argparse.ArgumentParser(description="Read-only Discovery contract checks")
    result.add_argument("--catalog-url", type=http_url, help="Full /api/v2beta/services (or /api/v2/services alias) URL")
    result.add_argument("--rest-capabilities-url", type=http_url, help="Full backend /v1beta/services URL")
    result.add_argument("--grpc-target", type=grpc_target, help="Backend host:port (not the operator)")
    result.add_argument("--plaintext-grpc", action="store_true")
    result.add_argument("--token-file", type=Path, help="Bearer token for the catalog only")
    result.add_argument("--capabilities-token-file", type=Path, help="Bearer token for explicit backend targets only")
    result.add_argument("--ca-bundle", type=Path)
    result.add_argument("--timeout", type=positive_float, default=10.0)
    result.add_argument("--max-pages", type=positive_int, default=100)
    result.add_argument("--page-size", type=positive_int, default=100)
    result.add_argument("--expect-service", action="append", default=[])
    result.add_argument("--json-report", type=Path)
    result.add_argument("--junit-report", type=Path)
    return result


def read_token(path):
    if path is None:
        return None
    token = path.read_text(encoding="utf-8").strip()
    if not token or any(character.isspace() for character in token):
        raise ValueError("Token file must contain a nonempty single token")
    return token


def provenance():
    result = json.loads(files("discovery_conformance.specs").joinpath("provenance.json").read_text())
    result["bundled_openapi_sha256"] = {
        name: hashlib.sha256(files("discovery_conformance.specs").joinpath(name).read_bytes()).hexdigest()
        for name in ("discovery_catalog.openapi.yaml", "capabilities_clean.openapi.yaml")
    }
    return result


def main(argv=None):
    argument_parser = parser()
    args = argument_parser.parse_args(argv)
    if not any((args.catalog_url, args.rest_capabilities_url, args.grpc_target)):
        argument_parser.error("select at least one explicit target")
    if args.expect_service and not args.catalog_url:
        argument_parser.error("--expect-service requires --catalog-url")
    if args.token_file and not args.catalog_url:
        argument_parser.error("--token-file requires --catalog-url")
    if args.capabilities_token_file and not (args.rest_capabilities_url or args.grpc_target):
        argument_parser.error("--capabilities-token-file requires a backend target")
    if args.plaintext_grpc and not args.grpc_target:
        argument_parser.error("--plaintext-grpc requires --grpc-target")
    if args.page_size > 2147483647:
        argument_parser.error("--page-size must fit protobuf int32")
    try:
        catalog_token = read_token(args.token_file)
        backend_token = read_token(args.capabilities_token_file)
        if args.ca_bundle:
            args.ca_bundle.read_bytes()
    except (OSError, UnicodeError, ValueError):
        argument_parser.error("Cannot read token or CA file, or token format is invalid")
    if catalog_token and args.catalog_url.startswith("http:"):
        argument_parser.error("Catalog bearer credentials require HTTPS")
    if backend_token and (args.plaintext_grpc or (args.rest_capabilities_url and args.rest_capabilities_url.startswith("http:"))):
        argument_parser.error("Backend bearer credentials require TLS")
    ca_bundle = str(args.ca_bundle) if args.ca_bundle else True
    checks = []
    if args.catalog_url:
        from .catalog import check_catalog
        checks.extend(check_catalog(args.catalog_url, token=catalog_token, ca_bundle=ca_bundle,
                                    timeout=args.timeout, expected_names=args.expect_service or None))
    if args.rest_capabilities_url:
        from .rest_capabilities import check_rest_capabilities
        checks.extend(check_rest_capabilities(args.rest_capabilities_url, token=backend_token,
                                             ca_bundle=ca_bundle, timeout=args.timeout))
    if args.grpc_target:
        from .capabilities import check_capabilities
        checks.extend(check_capabilities(args.grpc_target, token=backend_token, ca_bundle=ca_bundle,
                                         timeout=args.timeout, plaintext=args.plaintext_grpc,
                                         max_pages=args.max_pages, page_size=args.page_size))
    for check in checks:
        print(f'{check["status"]:12} {check["id"]}: {check["message"]}')
    print("Scope: selected read-only contracts only; this is not full deployment certification.")
    try:
        write_reports(checks, provenance(), args.json_report, args.junit_report)
    except OSError:
        print("ERROR: Unable to write report output.")
        return 2
    return exit_code(checks)
