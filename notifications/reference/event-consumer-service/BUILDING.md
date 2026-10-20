<!-- SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Building the Event Consumer Service

This guide describes how to build and run the Event Consumer Service from source, without access to NVIDIA-internal package registries. The supported path is a from-source build with `uv`; the last section explains the limits of the published `Dockerfile`.

## Prerequisites

- Install `uv`: https://docs.astral.sh/uv/getting-started/installation/
- To start the service, a reachable RabbitMQ broker. For local use, run one in Docker:

  ```bash
  docker run -d -p 5672:5672 rabbitmq:4.1.3
  ```

## Building from source

Run these commands from this folder (`notifications/reference/event-consumer-service`).

Install the dependencies into a virtual environment:

```bash
uv sync
```

Start the service. It connects to RabbitMQ at `OMNI_EVENTS_RABBITMQ_URI`, which defaults to `amqp://127.0.0.1:5672` (the broker above); set it to point anywhere else. It also needs an authentication mode: the built-in default, `jwtUnverified`, is refused at startup without an explicit opt-in, so for a local run without tokens select `disabled` (requests then need no bearer token, and permission checks stay off). `README.md` lists the other settings, including the modes that verify tokens.

```bash
OMNI_EVENTS_PRINCIPAL_SOURCE=disabled uv run start
```

Optionally, run the test suite to verify the build. It does not need a broker:

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
docker build --build-context common=../common -t event-consumer-service .
```

The build installs the dependencies from the published `uv.lock` with `uv export --locked`, which refuses a lock that no longer matches `pyproject.toml`, so the image carries the same versions as the `uv` path above, and installs `notification-common` from `../common`. Named build contexts need BuildKit (the default builder in current Docker releases).

The production stage builds on the `nvcr.io/nvidia/distroless/python` base image, which is publicly pullable.
