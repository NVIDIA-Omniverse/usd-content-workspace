<!-- SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Implement a compatible service

Each API area publishes its contract as Protobuf definitions and, where available, an OpenAPI specification. Implement against the contract, then check behavior with the area's conformance suite. A passing suite covers the checks it runs; it is not a certification of the whole API.

## Storage

The [Storage API](../../storage/api/README.md) includes the contracts, a conformance suite, a Python filesystem reference service, and templates. Run the suite against the filesystem reference:

```bash
cd storage/api/filesystem_example && poetry install && source .venv/bin/activate
cd ../conformance_tests && poetry install
cd .. && ./run_tests.sh
```

`./run_tests.sh azurite` runs it against an Azurite-backed service, and `./run_tests.sh --help` lists the options for selecting tests and running against an existing service.

To build a backend, start from the [Python templates](../../storage/api/templates/) for a prototype, or generate code from the protos in any language. [Storage Core](../../storage/reference/storage-core/README.md) is a Rust framework for storage backends and middleware that serve gRPC and REST.

## Other areas

| Area | Contract | Conformance |
| --- | --- | --- |
| Permissions | [Protos and OpenAPI](../../permissions/api/README.md) | [Cargo conformance crate](../../permissions/conformance/README.md) |
| User identity | [Protos and OpenAPI](../../user-identity/api/README.md) | [Cargo conformance crate](../../user-identity/conformance/README.md) |
| Metadata | [Protos and OpenAPI](../../metadata/api/README.md) | None in the current snapshot |
| Notifications | [Publisher and consumer protos](../../notifications/api/README.md) | `conformance_tests/` in each [service](../../notifications/README.md) |
| Discovery | [Protos](../../discovery/api/README.md) | [Read-only conformance runner](../../discovery/reference/discovery-service/operator/conformance/README.md) |
| Thumbnails | [Protos and OpenAPI](../../thumbnails/api/README.md) | [Cargo conformance crate](../../thumbnails/conformance/README.md) |

The Event Consumer Service's `AGENTS.md` is a guide to building a compatible implementation from its contract.
