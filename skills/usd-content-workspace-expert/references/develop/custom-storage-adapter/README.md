# Build a custom storage adapter

> **Applies to:** implementing the Storage API over your own backend · **Released:** `storage-api` NGC resource **1.0.0-beta.4** (protos, OpenAPI, Python reference, conformance suite) · **This repo's snapshot:** Storage API **unreleased (after 1.0.0-beta.4)** (OpenAPI `info.version` and the [`filesystem_example/pyproject.toml`](../../../../../storage/api/filesystem_example/pyproject.toml) version); Storage Core with unreleased changes ([`storage-core/CHANGELOG.md`](../../../../../storage/reference/storage-core/storage-core/CHANGELOG.md)), with unreleased breaking trait changes in the code ([storage-core-rust.md](storage-core-rust.md)) · **Sources:** [`storage/api/`](../../../../../storage/api/) (`README.md`, `AGENTS.md`, `PROMPTS.md`, `templates/`, `filesystem_example/`, `conformance_tests/`, `run_tests.sh`), [`storage/reference/storage-core/docs/`](../../../../../storage/reference/storage-core/docs/), the [source skill package](../../../NOTICE.md)

A storage adapter is a service that speaks the [Storage API](../../apis/storage.md) over a backend you choose. It is done when it passes the conformance suite, is packaged as an image, runs in your cluster with persistent storage and authentication, and is registered with Discovery so clients find it. An API sketch alone is not a deployable adapter.

Explain and recommend by default. Generate code, build images, or deploy only when asked.

## Choose a path

| Path | Use when | Read |
|---|---|---|
| **A. Python backend plugin** | Prototyping, Python-native backends, learning the contract. Reuses the reference service's gRPC and REST layers; you implement one backend class | [python-backend.md](python-backend.md) |
| **B. Production implementation, any language** | Go, Java, Rust, C++ or another stack; performance or operational requirements the Python reference does not meet | [production-implementation.md](production-implementation.md) |
| **C. Storage Core (Rust)** | Rust backends that want the same layer stack (caching, throttling, metrics, routing) as the Storage Service | [storage-core-rust.md](storage-core-rust.md) |

All three paths share:

- [capabilities-and-versioning.md](capabilities-and-versioning.md): which API versions to serve, what to advertise through `ListServices`, Discovery capabilities, folder modes, versioned versus unversioned storage, optimistic locking, native metadata.
- [packaging-and-deploying.md](packaging-and-deploying.md): image, Kubernetes resources, persistence, authentication, Discovery registration, in-cluster conformance.
- [Conformance](../../test/conformance.md#storage-api-conformance-suite): the pass criterion.

If you only need S3, S3-compatible or Azure Blob storage, use the NVIDIA Storage Service instead of writing an adapter ([components/storage-service.md](../../components/storage-service.md)).

## Before you start

1. Pick the contract version and keep it fixed: the released `storage-api` 1.0.0-beta.4 resource ([NGC](https://catalog.ngc.nvidia.com/orgs/nvidia/teams/omniverse/resources/storage-api); access in [deploy/artifact-access.md](../../deploy/artifact-access.md)) or this snapshot's unreleased definitions (after 1.0.0-beta.4) in [`storage/api/proto`](../../../../../storage/api/proto/) and [`storage/api/openapi`](../../../../../storage/api/openapi/). The released beta.4 OpenAPI files do not parse ([apis/storage.md](../../apis/storage.md#released-100-beta4-versus-the-unreleased-snapshot)); generate REST code from a corrected artifact or from the protos.
2. Read the `.feature` files for the groups you will implement: [`conformance_tests/src/conformance_tests/features/`](../../../../../storage/api/conformance_tests/src/conformance_tests/features/). They pin behavior the schema does not: error codes, idempotency, ordering, identity binding.
3. Decide the address scheme and identity encoding, the folder mode, and whether the backend is versioned. These shape every RPC.
4. Decide how the adapter will authenticate callers and authorize operations ([develop/add-your-service.md](../add-your-service.md#authentication-and-authorization)).

## Implementation order

From [`storage/api/AGENTS.md`](../../../../../storage/api/AGENTS.md), verified against the conformance features:

1. Capabilities (`ListServices`, `ListTopLevelAddresses`)
2. `Stat`
3. `Write`, then `Read` / `ReadFromAddress`
4. `Enumerate`, `List`, `ListStat`
5. `Delete`, `DeleteFolder`
6. `EnumerateVersions`
7. Redirect and multipart uploads, `FetchWriteTypeInfo`
8. Optional v1alpha: `CreateFolder`, `GetFolderMode`, `Copy`, `Move`, `ListRoutes`, metadata, capability queries

Run the matching conformance subset after each step (`./run_tests.sh -k stat`, see [test/conformance.md](../../test/conformance.md#storage-api-conformance-suite)).

## Material in the repository

| File | What it gives you |
|---|---|
| [`storage/api/README.md`](../../../../../storage/api/README.md) | Package overview, both paths, runner usage (its quick start runs `./run_tests.sh` from the wrong directory; see [python-backend.md](python-backend.md#run-the-conformance-suite)) |
| [`storage/api/AGENTS.md`](../../../../../storage/api/AGENTS.md), [`PROMPTS.md`](../../../../../storage/api/PROMPTS.md) | Agent-oriented implementation guide and prompts |
| [`templates/python_backend/`](../../../../../storage/api/templates/python_backend/) | Path A scaffold (has a CLI registration defect, see [python-backend.md](python-backend.md#template-defects)) |
| [`templates/production_impl/README.md`](../../../../../storage/api/templates/production_impl/README.md) | Path B checklist and Go/Rust/Java skeletons |
| [`filesystem_example/`](../../../../../storage/api/filesystem_example/) | Complete reference service, REST + gRPC, versioned, with multipart, redirects, metadata and native metadata |
| [`conformance_tests/`](../../../../../storage/api/conformance_tests/) | Black-box suite, runnable against any adapter |
| [`docs/`](../../../../../storage/api/docs/index.rst) | User manual, service and client walkthroughs and changelog as Sphinx sources (`index.rst`, `user_manual.md`, `service_example.md`, `client_example.md`, `changes.rst`) |
| [`storage/reference/storage-core/docs/backends.md`](../../../../../storage/reference/storage-core/docs/backends.md) | Path C backend guide |

The upstream storage-api skill in this repo, [`storage/api/skills/storage-api-contract-map/SKILL.md`](../../../../../storage/api/skills/storage-api-contract-map/SKILL.md), names source-repository paths (`examples/filesystem_server/`, `docs/*.md`, `Makefile`). This snapshot is the distribution layout; [components/storage-api.md](../../components/storage-api.md) maps one to the other.
