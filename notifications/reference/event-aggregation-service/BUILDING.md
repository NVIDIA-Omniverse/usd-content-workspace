<!-- SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Building the Event Aggregation Service

This guide describes how to build and run the Event Aggregation Service from source, without access to NVIDIA-internal package registries. The supported path is a from-source build with `uv`; the last section explains the limits of the published `Dockerfile`.

## Prerequisites

- Install `uv`: https://docs.astral.sh/uv/getting-started/installation/
- To start the service, a reachable RabbitMQ broker. For local use, run one in Docker:

  ```bash
  docker run -d -p 5672:5672 rabbitmq:4.1.3
  ```

## Building from source

Run these commands from this folder (`notifications/reference/event-aggregation-service`).

Install the dependencies into a virtual environment:

```bash
uv sync
```

Start the service. It connects to RabbitMQ at `OMNI_EVENTS_RABBITMQ_URI`, which defaults to `amqp://127.0.0.1:5672` (the broker above); set it to point anywhere else. `README.md` lists the other settings.

The service refuses to start until an authentication mode is chosen. For a local run that accepts requests without a bearer token, turn authentication off:

```bash
OMNI_EVENTS_PRINCIPAL_SOURCE=disabled uv run start
```

To verify tokens instead, set `OMNI_EVENTS_PRINCIPAL_SOURCE=jwt` with `OMNI_EVENTS_OPENID_CONFIGURATION_URI` and `OMNI_EVENTS_JWT_AUDIENCES`; see the authentication section of `README.md`.

Optionally, run the test suite (it needs no broker) to verify the build:

```bash
uv run test
```

### Dependency resolution

The published source is externally resolvable and depends on no NVIDIA-internal package registry:

- The first-party `notification-common` dependency resolves from the sibling `../common` directory (`notifications/reference/common` in this repository), not from an internal PyPI index, so build from a checkout of the whole repository.
- The build uses whatever version of `notification-common` is checked out in `../common`. uv ignores the `notification-common==X.Y.Z` pin in `pyproject.toml` for a path source, so the pin records the version this service was released against rather than enforcing it.
- `uv.lock` is published. It pins every dependency to the version this service release was built and tested with, resolved from public PyPI, and records `notification-common` as the `../common` directory at the version the release pins. `uv sync --locked` installs exactly that set.
- If `../common` holds a different `notification-common` version than the lock records, `uv lock --check` reports the lock as out of date; `uv sync` (without `--locked`) re-locks against the checked-out `../common`.
- Every third-party dependency resolves from public PyPI.

## Building with Docker

Build the image from this folder, passing the sibling `../common` directory as a named build context (the published `Dockerfile` copies it in from there, because it lies outside this folder's build context):

```bash
docker build --build-context common=../common -t event-aggregation-service .
```

The build installs the dependencies from the published `uv.lock` with `uv export --locked`, so the image carries the same versions as the `uv` path above, and installs `notification-common` from `../common`. Because the export is `--locked`, the build fails when `uv.lock` is out of date with `pyproject.toml` or with the `notification-common` checked out in `../common`, rather than producing an image from a stale lock. Named build contexts need BuildKit (the default builder in current Docker releases).

The production stage builds on the `nvcr.io/nvidia/distroless/python` base image, which is publicly pullable.
