# Storage API — component map

> **Applies to:** `storage/api` (the Storage API contract, Python filesystem reference, conformance suite) · **Released:** NGC API resource `storage-api` **1.0.0-beta.4**; HTML docs [1.0.0-beta.3](https://docs.omniverse.nvidia.com/ovstorage/storage-api/1.0.0-beta.3/index.html) · **This repo's snapshot:** **unreleased (after 1.0.0-beta.4)** (OpenAPI `info.version`, both `pyproject.toml` files; source commit in [`SOURCE_VERSIONS.md`](../../../../SOURCE_VERSIONS.md)) · **Sources:** [`storage/api/`](../../../../storage/api/), in-repo skill [`storage/api/skills/storage-api-contract-map/SKILL.md`](../../../../storage/api/skills/storage-api-contract-map/SKILL.md), upstream `storage-protos` skill (the [source skill package](../../NOTICE.md) pin)

This page is a table of contents. Read the linked file before answering, and confirm the version the deployment runs ([capture a deployment](../versions/capture-a-deployment.md)) before generating code.

## Start with the in-repo skill

[`storage/api/skills/storage-api-contract-map/SKILL.md`](../../../../storage/api/skills/storage-api-contract-map/SKILL.md) is the authoritative route map for the contract: what is normative (protos, OpenAPI and `.feature` files), how to implement, verify, call, and track changes. Use it, with the corrections below.

It is written for the **source repository** (`storage-protos`, default branch `trunk`). This directory is the **OSS distribution** layout. The skill has a translation table; this is the full mapping for this snapshot:

| The skill says | In this repo |
|---|---|
| `examples/filesystem_server/` | [`filesystem_example/`](../../../../storage/api/filesystem_example/) |
| `docs/user_manual.md`, `docs/service_example.md`, `docs/client_example.md` | [`docs/user_manual.md`](../../../../storage/api/docs/user_manual.md), [`docs/service_example.md`](../../../../storage/api/docs/service_example.md), [`docs/client_example.md`](../../../../storage/api/docs/client_example.md) (Sphinx sources; entry point [`docs/index.rst`](../../../../storage/api/docs/index.rst)) |
| `docs/changes.rst` | [`docs/changes.rst`](../../../../storage/api/docs/changes.rst) (also [`CHANGELOG.md`](../../../../storage/api/CHANGELOG.md)) |
| `oss/AGENTS.md`, `oss/PROMPTS.md`, `oss/README.md`, `oss/templates/` | [`AGENTS.md`](../../../../storage/api/AGENTS.md), [`PROMPTS.md`](../../../../storage/api/PROMPTS.md), [`README.md`](../../../../storage/api/README.md), [`templates/`](../../../../storage/api/templates/) |
| `make` test targets, `./run_conformance_test.sh` | [`run_tests.sh`](../../../../storage/api/run_tests.sh) |
| `examples/python_client/`, `examples/python_service/` | same paths: [`examples/python_client/`](../../../../storage/api/examples/python_client/) (gRPC and REST list, upload, download, delete; included by `docs/client_example.md`), [`examples/python_service/`](../../../../storage/api/examples/python_service/) (minimal gRPC and REST filesystem services; included by `docs/service_example.md`) |
| `conformance_tests/conformance_test_suite.md`, `docs/rdme/`, `Makefile`, `tools/`, `bootstrap.sh`, `RFCs/`, `buf` config | **not in this snapshot** |
| `openapi/*/v1test*` experimental versions | not in this snapshot (the Storage Service still vendors `metadata/v1testmetadata`, compiled but not served) |

## Route by task

| Question | Go to |
|---|---|
| Wire packages, RPCs, REST routes, released vs snapshot, client calls | [apis/storage.md](../apis/storage.md) |
| Metadata (storage-namespaced v1alpha and top-level v1beta) | [apis/metadata.md](../apis/metadata.md), [metadata-api.md](metadata-api.md) |
| gRPC definitions | [`proto/nvidia/omniverse/storage/`](../../../../storage/api/proto/nvidia/omniverse/storage/) (`fileobject`, `filefolder`, `versioning`, `capabilities` at v1alpha+v1beta; `metadata` v1alpha) |
| REST definitions | [`openapi/`](../../../../storage/api/openapi/) (nine files; all parse in this snapshot) |
| Normative behavior | [`conformance_tests/src/conformance_tests/features/`](../../../../storage/api/conformance_tests/src/conformance_tests/features/) |
| Running the conformance suite | [test/conformance.md](../test/conformance.md#storage-api-conformance-suite), [`conformance_tests/README.md`](../../../../storage/api/conformance_tests/README.md) |
| Building an adapter | [develop/custom-storage-adapter/](../develop/custom-storage-adapter/README.md) |
| Reference service usage and CLI | [`filesystem_example/README.md`](../../../../storage/api/filesystem_example/README.md), [python-backend.md](../develop/custom-storage-adapter/python-backend.md) |
| Templates | [`templates/python_backend/`](../../../../storage/api/templates/python_backend/README.md), [`templates/production_impl/`](../../../../storage/api/templates/production_impl/README.md) |
| Agent instructions and prompts | [`AGENTS.md`](../../../../storage/api/AGENTS.md), [`PROMPTS.md`](../../../../storage/api/PROMPTS.md) |
| What changed | [`CHANGELOG.md`](../../../../storage/api/CHANGELOG.md); `git log -- storage/api/proto storage/api/openapi` in this repo shows only snapshot imports |
| Security reporting, license | [`SECURITY.md`](../../../../storage/api/SECURITY.md), [`LICENSE.txt`](../../../../storage/api/LICENSE.txt) (Apache-2.0) |

## What the in-repo skill lacks

**Versions.** It names no version. Released resource 1.0.0-beta.4 (whose OpenAPI files are unparseable; see [apis/storage.md](../apis/storage.md#released-100-beta4-versus-the-unreleased-snapshot)); snapshot unreleased (after 1.0.0-beta.4). The `CHANGELOG.md` has no 1.0.0-beta.3 entry; the HTML docs version is 1.0.0-beta.3.

**Consumers and their pins.** Implementations compile copies, not this directory:

| Consumer | Copy and pin |
|---|---|
| Storage Core | [`storage-core-protos/proto/storage-protos/VERSION`](../../../../storage/reference/storage-core/storage-core-protos/proto/storage-protos/VERSION): storage-protos tag `0.28.0` + hand-applied native-metadata hunks from unreleased Storage API source + local `IsReadOnly`/`IsVersioned`. Its CI derives the conformance version from the `tag:` line |
| Storage Service | own v1alpha metadata copy ([`VERSION`](../../../../storage/reference/storage-service/storage-service-protos/proto/nvidia/omniverse/storage/VERSION): Buf commit `b191bc7…`); everything else through Storage Core |
| Thumbnail Service | pre-generated Rust bindings (Buf, [`crates/proto/storage/buf.gen.yaml`](../../../../thumbnails/reference/thumbnail-service/crates/proto/storage/buf.gen.yaml)) for `fileobject`, `filefolder` and `metadata` v1alpha only, generated from this repository's `storage/api/proto`; no `.proto` copy or `VERSION` pin, so the bindings match `storage/api/proto` as it was when they were last regenerated ([thumbnail-service.md](thumbnail-service.md)) |

Note the two version schemes: storage-protos tags (`0.28.0`) versus the Storage API package version (such as `1.0.0-beta.4`). The mapping between them is not recorded in this repo.

**Toolchain without the source repository.** Stubs under `filesystem_example/src/nvidia/` and `conformance_tests/src/nvidia/` are committed, so Poetry installs work without Packman. Both READMEs still send you to a root-README section and `make` targets that are absent here; regenerate with `python -m grpc_tools.protoc` only after changing a `.proto` ([apis/storage.md](../apis/storage.md#minimal-client-calls)).

**Deployment.** The reference service has no Discovery v2alpha capabilities, no auth, no health service; see [develop/custom-storage-adapter/packaging-and-deploying.md](../develop/custom-storage-adapter/packaging-and-deploying.md) and [deploy/local-development.md](../deploy/local-development.md).

## Changes in this snapshot

The previous snapshot shipped the developer guide only as rendered HTML under `docs/latest/`. This snapshot removes `docs/latest/` (64 generated files) and ships the Sphinx sources instead: [`docs/index.rst`](../../../../storage/api/docs/index.rst) (table of contents), `user_manual.md`, `service_example.md`, `client_example.md`, and `changes.rst` (which includes `CHANGELOG.md`). The walkthroughs `literalinclude` code from the restored [`examples/`](../../../../storage/api/examples/) directory. No Sphinx `conf.py` ships, so read the Markdown directly rather than expecting to render it. [`README.md`](../../../../storage/api/README.md), [`AGENTS.md`](../../../../storage/api/AGENTS.md) and the in-repo skill's translation table now point at these sources. `AGENTS.md`, `PROMPTS.md`, `templates/` and `skills/` are still present.

Nothing normative changed: the protos are unchanged, the OpenAPI files differ only in `info.version` and a removed `tags` block, and the conformance feature files differ only in comments. `CHANGELOG.md` has no entry for this build.

## Known defects in the in-repo docs

- [`README.md`](../../../../storage/api/README.md) quick start: `cd ../conformance_tests && poetry install`, then `./run_tests.sh`; the script is in `storage/api/` and must run from there (it looks for `conformance_tests/.venv`). Correct sequence in [python-backend.md](../develop/custom-storage-adapter/python-backend.md#run-the-conformance-suite).
- `README.md`, `run_tests.sh --help` and `AGENTS.md`: `-m grpc` / `-m rest` select nothing; the suite registers no such markers. Use `-k GRPC` / `-k REST`.
- `AGENTS.md` manual-testing block runs `./run_tests.sh --test-only` from inside `conformance_tests/`; same working-directory problem.
- `AGENTS.md` and `templates/production_impl/README.md` list `CreateFolder` as a required v1beta RPC; v1beta FileFolder has `List`, `ListStat`, `DeleteFolder` only.
- `AGENTS.md` and `README.md` codegen: `protoc ... proto/nvidia/omniverse/storage/**/*.proto` without `-I proto`; imports do not resolve.
- `AGENTS.md` Path A and `templates/python_backend/__init__.py` register CLIs with `click` returning a `dict`; the registry is Typer-based and needs a `BackendConfig` ([python-backend.md](../develop/custom-storage-adapter/python-backend.md#template-defects)).
- `filesystem_example/README.md`: v1alpha capabilities example URL `/v1alpha/capabilities/services/capabilities/services` should be `/v1alpha/capabilities/services`; "the image default command includes the backend subcommand" is false (`CMD []`), though the env-driven default works; the default storage dir is `<tmp>/storage_api_test`, not the bare temp dir.
- `filesystem_example/README.md` and `conformance_tests/README.md` "Build from source" point to a root `README.md` section and `make sync-*` targets not in this snapshot; `conformance_tests/README.md` native-metadata examples call `./run_conformance_test.sh fixtures.filesystem_service`, which does not exist here (use `FILESERVICE_NATIVE_METADATA_MODE=disabled ./run_tests.sh -k native_metadata`).
- The in-repo skill itself routes to source-repo paths (table above) and says to put the Metadata API split-out location in `CHANGELOG.md` "if ever split"; the split has happened ([`metadata/api/`](../../../../metadata/api/README.md)) and `CHANGELOG.md` does not record it.

Related: [storage-core.md](storage-core.md), [storage-service.md](storage-service.md), [test/conformance.md](../test/conformance.md).
