# Path A: Python backend plugin

> **Applies to:** the Python reference service `omniverse-filesystem-storage-service` and new backends plugged into it · **Released:** shipped as `filesystem_example/` inside the `storage-api` **1.0.0-beta.4** NGC resource · **This repo's snapshot:** unreleased, after 1.0.0-beta.4 ([`filesystem_example/pyproject.toml`](../../../../../storage/api/filesystem_example/pyproject.toml)); conformance package `omniverse-storageapi-conformance-tests` same version · **Sources:** [`filesystem_example/README.md`](../../../../../storage/api/filesystem_example/README.md), [`filesystem_example/src/local_filesystem_service/`](../../../../../storage/api/filesystem_example/src/local_filesystem_service/), [`templates/python_backend/`](../../../../../storage/api/templates/python_backend/), [`run_tests.sh`](../../../../../storage/api/run_tests.sh), [`AGENTS.md`](../../../../../storage/api/AGENTS.md)

The reference service separates transport from storage. The gRPC layer (`grpc_service/`) and REST layer (`rest_service/`, FastAPI) call one object that implements `StorageBackendInterface`. A new backend is a new implementation of that interface plus a CLI subcommand; the transports, multipart, redirects, metadata routing and capability endpoints come for free.

This service is a reference for learning and prototyping. It has no authentication, binds all interfaces, and serves the whole content directory without authorization under `/download` for redirect reads. Never expose it outside a trusted local network.

## Prerequisites

- Python 3.10, 3.11 or 3.12; Poetry.
- The generated protobuf stubs under `src/nvidia/` are **already committed** in this snapshot for both `filesystem_example` and `conformance_tests`. The "Build from source" note in both READMEs points to a root `README.md` section ("Generate Python protobuf stubs without Packman") and `make` targets that do not exist here; skip it unless you change a `.proto`, then regenerate with `grpc_tools.protoc` ([apis/storage.md](../../apis/storage.md#minimal-client-calls)).
- Both READMEs assume Poetry creates `.venv` inside each project. Poetry's default is a shared cache directory; set `POETRY_VIRTUALENVS_IN_PROJECT=true` (or `poetry config virtualenvs.in-project true`) first, or use `poetry env info` to find the environment.

## Install and run the reference service

```bash
cd storage/api/filesystem_example
python -m venv .poetry_venv
.poetry_venv/bin/pip install poetry
POETRY_VIRTUALENVS_IN_PROJECT=true .poetry_venv/bin/poetry install
source .venv/bin/activate

local-filesystem-service filesystem --static-dir /tmp/storage     # gRPC :50051, REST :8011
curl -s http://localhost:8011/v1beta/capabilities/services
curl -s http://localhost:8011/v1alpha/capabilities/services          # adds v1alpha and metadata
```

Interactive OpenAPI pages: `http://localhost:8011/v1beta/fileobject/docs` (one per mounted API). [`filesystem_example/README.md`](../../../../../storage/api/filesystem_example/README.md) shows the v1alpha query as `/v1alpha/capabilities/services/capabilities/services`; the route is `/v1alpha/capabilities/services`.

### Entry points

| Command | Serves |
|---|---|
| `local-filesystem-service` / `python -m local_filesystem_service` | gRPC and REST (disable either with `--no-grpc` / `--no-rest`) |
| `local-filesystem-grpc` / `python -m local_filesystem_service.grpc_service` | gRPC only |
| `local-filesystem-rest` / `python -m local_filesystem_service.rest_service` | REST only |
| `local-filesystem-service list-backends` | Registered backend names |

Usage is `COMMAND [COMMON_OPTIONS] BACKEND [BACKEND_OPTIONS]`; common options must precede the backend subcommand. With no subcommand the `filesystem` backend starts from environment variables.

### Options (from [`__main__.py`](../../../../../storage/api/filesystem_example/src/local_filesystem_service/__main__.py) and [`filesystem/__init__.py`](../../../../../storage/api/filesystem_example/src/local_filesystem_service/filesystem/__init__.py))

| Option | Env var | Default | Meaning |
|---|---|---|---|
| `--grpc-port` | `GRPC_SERVER_PORT` | `50051` | gRPC port |
| `--http-port` | `HTTP_SERVER_PORT` | `8011` | REST port |
| `--grpc/--no-grpc`, `--rest/--no-rest` | — | both on | Enable each transport |
| `--reload` | — | off | REST auto-reload (REST-only entry point) |
| `filesystem --base-uri` | `FILESERVICE_SERVER_BASE_URI` | `file-storage://fileservice` | Address prefix; also the top-level address |
| `filesystem --static-dir` | `FILESERVICE_STATIC_DIR` | `<system temp>/storage_api_test` | Storage root; mount persistent storage here |
| `filesystem --folder-mode` | `FILESERVICE_TEST_FOLDER_MODE` | `native` | `native`, `no_empty`, or `placeholder` (reported as `FOLDER_MODE_HYBRID`) |
| `filesystem --redirect-host` / `--redirect-port` | `REDIRECT_HOST` / `REDIRECT_PORT` | `http://localhost` / `8011` | Base of redirect URLs handed to clients; must be reachable from the client |
| `filesystem --native-metadata-mode` | `FILESERVICE_NATIVE_METADATA_MODE` | `enabled` | `disabled` exercises the unsupported-route contract |
| `filesystem --versioning-mode` | `FILESERVICE_VERSIONING_MODE` | `versioned` | `unversioned`: single current version, hard deletes |
| `filesystem --read-only/--no-read-only` | `FILESERVICE_READ_ONLY` | `false` | Reject mutations (gRPC `FAILED_PRECONDITION`, REST 412) |

CLI options take precedence over environment variables. The [source skill package](../../../NOTICE.md) omitted `--versioning-mode`, `--read-only`, `--no-grpc` and `--no-rest`, which were added after beta.4.

## The backend interface

[`backends/storage_backend_interface.py`](../../../../../storage/api/filesystem_example/src/local_filesystem_service/backends/storage_backend_interface.py) is the authority. Abstract methods you must implement:

| Area | Methods |
|---|---|
| Addressing | `base_uri` (property), `is_address_valid`, `is_version_address`, `create_identity_from_resource_address`, `address_from_identity` |
| Existence | `exists`, `is_file`, `is_dir` |
| Objects | `read_from_address`, `read_from_identity` (generators of `bytes`), `write_version`, `stat`, `stat_identity`, `remove_by_address`, `obliterate`, `copy`, `move` |
| Folders | `create_folder`, `list`, `list_stat`, `enumerate`, `remove_empty_folder` |
| Versions | `enumerate_versions` |
| Metadata | `get_metadata`, `update_metadata`, `delete_metadata` (each with a `namespace`, default `"default"`) |
| Authorization hooks | `check_read_permission_on_address` (and overridable `check_write_permission_on_address`) |
| Uploads | `construct_redirect_url`, `encode_upload_id`, `decode_upload_id` |

Overridable defaults cover `supports_redirect_download`, `supports_redirect_upload`, `supports_multipart_upload`, `supports_native_metadata(address)`, `folder_mode()`, `get_optimistic_locking_support()`, `is_version_latest`, `url_from_identity`, upload-session helpers, and `register_http_routes(app)` for backend-owned redirect endpoints. Read [`filesystem/file_system_provider.py`](../../../../../storage/api/filesystem_example/src/local_filesystem_service/filesystem/file_system_provider.py) for a complete implementation, including the versioned content tree and atomic `previous_version` checks.

Raise these and the transport layers map them (verified in `grpc_service/fileobject.py`):

| Raise | gRPC | REST |
|---|---|---|
| `FileNotFoundError` | `NOT_FOUND` | 404 |
| `PermissionError` | `PERMISSION_DENIED` | 403 |
| `ValueError` | `INVALID_ARGUMENT` | 400 |
| `FileExistsError` | `ALREADY_EXISTS` | 409 |
| `EtagMismatchError` | `FAILED_PRECONDITION` | 412 |
| `ReadOnlyError` | `FAILED_PRECONDITION` | 412 |
| `IsADirectoryError` | file operation on a folder; mapped per RPC | — |

Keep path handling inside the root: the reference helper [`backends/path_helpers.py`](../../../../../storage/api/filesystem_example/src/local_filesystem_service/backends/path_helpers.py) rejects addresses outside `base_uri` and `..` traversal above the root. A new backend needs equivalent checks for its own key space.

## Register a backend

The registry expects a factory decorated with `@register_backend(name)` and a **Typer-style** CLI function decorated with `@register_backend_cli(name)` whose parameters use `Annotated[..., typer.Option(...)]` and which **returns a `BackendConfig`**. `__main__.py` turns each registered CLI function into a subcommand by reading its signature and calls `_run_services(backend_config, ...)`, which reads `backend_config.base_uri`.

```python
# src/local_filesystem_service/mystorage/__init__.py
from dataclasses import dataclass
from typing import Optional

import typer
from typing_extensions import Annotated

from local_filesystem_service.backends import BackendConfig, StorageBackendInterface, register_backend, register_backend_cli
from .my_storage_provider import MyStorageBackend


@dataclass
class MyStorageConfig(BackendConfig):
    bucket: str = ""
    endpoint_url: Optional[str] = None


@register_backend("mystorage")
def create_mystorage_backend(config: MyStorageConfig) -> StorageBackendInterface:
    return MyStorageBackend(base_uri=config.base_uri, bucket=config.bucket, endpoint_url=config.endpoint_url)


@register_backend_cli("mystorage", "Start service with MyStorage")
def mystorage_cli(
    base_uri: Annotated[str, typer.Option("--base-uri", envvar="MYSTORAGE_BASE_URI")] = "mystorage://default",
    bucket: Annotated[str, typer.Option("--bucket", envvar="MYSTORAGE_BUCKET")] = "",
    endpoint_url: Annotated[Optional[str], typer.Option("--endpoint-url", envvar="MYSTORAGE_ENDPOINT_URL")] = None,
) -> MyStorageConfig:
    return MyStorageConfig(backend_type="mystorage", base_uri=base_uri, bucket=bucket, endpoint_url=endpoint_url)
```

Make sure the module is imported before the entry points build their subcommands. The built-in backend registers because each of `__main__.py`, `grpc_service/__main__.py` and `rest_service/__main__.py` imports `local_filesystem_service.filesystem`; add `import local_filesystem_service.mystorage  # noqa: F401` beside that import in the entry points you use. The template suggests importing from `backends/__init__.py` instead; your module imports from that same package while it is still initializing, so place such an import at the end of the file if you use it. Neither placement was executed for this reference. Then:

```bash
local-filesystem-service list-backends
local-filesystem-service mystorage --help
```

Take credentials from the environment, a mounted file, or workload identity; never from a CLI argument that lands in process listings or shell history.

### Template defects

[`templates/python_backend/__init__.py`](../../../../../storage/api/templates/python_backend/__init__.py) and the Path A example in [`AGENTS.md`](../../../../../storage/api/AGENTS.md) register the CLI with `@click.option` decorators and return a plain `dict`. That does not match the registry in this snapshot: the options are invisible to the Typer-built subcommand, and `_run_services` fails on the dict because it has no `base_uri` attribute. Use the pattern above. `click` is not a declared dependency of `filesystem_example` (it arrives only through Typer).

The method-by-method guidance in [`templates/python_backend/my_storage_provider.py`](../../../../../storage/api/templates/python_backend/my_storage_provider.py) is still useful; compare its signatures with the interface file before copying, because the interface gained namespace, native-metadata and read-only parameters in unreleased Storage API source. Keep the NVIDIA SPDX header and add your own when you copy template files ([`templates/python_backend/README.md`](../../../../../storage/api/templates/python_backend/README.md#licensing)).

## Run the conformance suite

[`storage/api/README.md`](../../../../../storage/api/README.md) runs `./run_tests.sh` after `cd ../conformance_tests`, but the script is in `storage/api/`, and it looks for `conformance_tests/.venv` relative to the working directory. Run it from `storage/api`:

With `poetry` on `PATH` (for example the `.poetry_venv/bin/poetry` installed above):

```bash
cd storage/api
export POETRY_VIRTUALENVS_IN_PROJECT=true
(cd filesystem_example && poetry install)
(cd conformance_tests && poetry install && .venv/bin/pip install -e ../filesystem_example)
source conformance_tests/.venv/bin/activate      # puts run-conformance-tests and local-filesystem-service on PATH

./run_tests.sh                                   # starts the filesystem backend on a temp dir, runs the suite, stops it
./run_tests.sh -k stat --no-parallel --verbose   # one area, serial
```

Against your backend (the runner passes `BACKEND_ARGS` after the backend name, and requires the resource base for any backend other than `filesystem`):

```bash
TEST_STORAGE_API_RESOURCE_BASE="mystorage://default" \
BACKEND_ARGS="--bucket conformance-tests --endpoint-url http://127.0.0.1:9000" \
./run_tests.sh mystorage
```

Other modes: `--service-only` (start and wait), `--test-only` (service already running), `--force` (kill whatever holds ports 8011/50051), `--parallel N`, `--clean`. Ports come from `GRPC_PORT` / `HTTP_PORT`, not from the conformance suite's own variables.

`run_tests.sh --help` and the README advertise `-m grpc` and `-m rest`. The suite registers no `grpc` or `rest` markers (only `conformance_tests`, `optional`, `unversioned`, `read_only`); protocol is a scenario parameter. Use `-k GRPC` or `-k REST` to select a transport, and check the collected count.

Mode-specific scenarios run only when the same environment variable is set for the service and the suite:

```bash
FILESERVICE_VERSIONING_MODE=unversioned ./run_tests.sh -k unversioned
FILESERVICE_READ_ONLY=true ./run_tests.sh -k read_only
FILESERVICE_NATIVE_METADATA_MODE=disabled ./run_tests.sh -k native_metadata
```

Everything else about the suite (variables, test data generators, OpenAPI comparison, what a pass covers) is in [test/conformance.md](../../test/conformance.md#storage-api-conformance-suite).

## Next

- Serve the right API versions and capabilities: [capabilities-and-versioning.md](capabilities-and-versioning.md).
- Image, Kubernetes and Discovery: [packaging-and-deploying.md](packaging-and-deploying.md).
- The Python reference does not implement the Discovery capabilities API the Auto Discovery operator calls. Add it before relying on the operator ([develop/add-your-service.md](../add-your-service.md#expose-discovery-capabilities)).
