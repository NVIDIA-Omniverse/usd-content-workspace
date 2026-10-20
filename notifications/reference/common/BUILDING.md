# Building notification-common from source

`notification-common` is a Python library (>=3.12) shared by NVIDIA Omniverse
notification services. This document explains how to build and install the
library from this source tree.

## Prerequisites

The build uses [uv](https://docs.astral.sh/uv/) for dependency management.
Install it if you haven't already:

```bash
curl -LsSf https://astral.sh/uv/install.sh | sh
```

## Build and install

From the repository root, run:

```bash
uv sync
```

`uv sync` resolves the dependencies declared in `pyproject.toml`, creates a
virtual environment, and installs the library into it.

## Dependency resolution

This published source resolves all of its third-party dependencies from public
PyPI. Every dependency the library declares is a public third-party package, so
`uv sync` on a fresh clone installs cleanly without access to NVIDIA's internal
Artifactory. The internal `[[tool.uv.index]]` registry entries that the library
uses inside NVIDIA are stripped from `pyproject.toml` at publish time, so the
mirrored `pyproject.toml` points only at public PyPI.

## Generated protobuf stubs

The generated protobuf and gRPC stubs under `nvidia/` are pre-generated and
shipped in this source tree. External consumers do not regenerate them. They are
produced from pinned public `buf.build` plugins (`protocolbuffers/python:v28.3`,
`grpc/python:v1.78.0`, and `protocolbuffers/pyi:v28.3`); the codegen
configuration and regeneration tooling are internal-maintainer-only and are not
part of this published mirror.

## Internal consumers

The library is also published to NVIDIA's internal PyPI for internal Omniverse
notification services. Internal consumers continue to depend on it from that
registry rather than building from this source.
