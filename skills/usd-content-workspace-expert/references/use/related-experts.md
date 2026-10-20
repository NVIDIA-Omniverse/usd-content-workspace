# Related experts in other repositories

> **Applies to:** the other Omniverse Content Services products and the agent skills they ship · **Released:** each product has its own versions and release notes · **This repo's snapshot:** none of these products is in this repository · **Sources:** each product's README and skills, read 2026-10-14

usd-content-workspace supplies the services. Four sibling products build on it or sit beside it, and each ships its own agent skills. Use this page to answer the basic question, then hand the caller to that product's expert for anything deeper. Their repositories are the authority for their own behavior; don't answer detailed questions about them from this page.

| Product | What it is | Hand off when the caller asks about | Expert |
| --- | --- | --- | --- |
| [ovstorage](https://github.com/NVIDIA-Omniverse/ovstorage) | Client-side data-access library: one object API over local, cloud, and Storage Service backends | Reading, writing, listing, or materializing objects from application code, the CLI, or MCP; backend plugins; the broker daemon | Role skills in [`skills/`](https://github.com/NVIDIA-Omniverse/ovstorage/tree/main/skills) (`ovstorage-user-*`, `ovstorage-operator-*`, `ovstorage-contributor-*`) |
| [ovlive](https://github.com/NVIDIA-Omniverse/ovlive) | Live OpenUSD collaboration: the client runtime and the Live Edit Service | Adding live editing to a USD host, Python tool, or Kit app; `live://` and `.live` layers; deploying or operating the Live Edit Service | `ovlive-expert` in `skills/ovlive-expert/` |
| [ovpackage](https://github.com/NVIDIA-Omniverse/ovpackage) | CLI and async Python API for versioned, reproducible asset packages (formerly WRAPP) | Creating, publishing, installing, mirroring, diffing, freezing, or exporting packages | `ovpackage-cli` and `ovpackage-api` in [`.agents/skills/`](https://github.com/NVIDIA-Omniverse/ovpackage/tree/main/.agents/skills) |
| [usd-caches](https://github.com/NVIDIA-Omniverse/usd-caches) | Local cache (Hub Workstation Cache), shared content cache (OVCC), and shared derived-data cache (OVDC) | Which caches a workflow needs, sizing, deploying, configuring, validating, or troubleshooting a cache | `usd-caches-expert` in `skills/usd-caches-expert/` |

When you hand off, give the caller the product, the expert's path in that product's repository, and the facts from this stack it will need: the Discovery URL, the services and API versions deployed, and how tokens are issued. Each expert needs its own repository checked out.

## ovstorage

- **Knows:** the `ovstorage::Stack` composition of Layers (backends, routers, caches, retries, aliases); the plugin contract and the first-party plugins (`file`, `http`, `s3`, `gcs`, `azure`, `huggingface`, `opendal`, `nucleus`, and the Storage Service client); host surfaces in Rust, the C ABI, the header-only C++20 wrapper, the Python wheel, the CLI, and an MCP server; and the `ovstorage-broker` daemon with listener authentication, a built-in authorization policy, and Prometheus metrics.
- **Its skills by role:** users (getting started, choosing a backend, authenticating, reading, writing safely, listing, materializing, deleting safely, handling errors, connecting over MCP); operators (deploying, configuring policy for, monitoring, and debugging the broker); contributors (plugins, the authorization layer, C headers, integration tests, Storage API client conformance, the pre-merge gate).
- **Meets this stack at:** the `omniverse-storage-service` backend, which takes either a Discovery root serving `/api/v1/services` and `/api/v1/auth-config` with OIDC credentials, or a `grpc://` / `grpcs://` Storage endpoint dialed directly. The deployment needs Discovery v1 output for the first form.
- **Status:** pre-release (Early Access); APIs may change before 1.0. Built from source; there is no NGC artifact.
- **Name:** "ovstorage" means only this library, never this stack.

## ovlive

- **Holds:** the client under `client/` and the Live Edit Service source and Helm chart under `service/`, which each service release replaces.
- **Knows:** the client runtime (`omni_usd_live`) with its C ABI `Live.h`, C++ `LiveUsd.h` adapter, Python `omni.usd.live` package, and `omni.kit.live_services` Kit extension; `live://` URIs, `.live` layers, sessions, presence, and edit targets; building and testing the client; running and deploying the Live Edit Service, its configuration, scaling, monitoring, and upgrades; and validating that two participants really exchange edits.
- **Meets this stack at:** the Live Edit Service (gRPC `nvidia.omniverse.live.v1alpha.LiveService` on port 50001, metrics on 9090, backed by Valkey), which forwards the caller's token to a Storage Service to authorize access to the session's base asset. Anyone who can read the base asset can modify its live sessions. The service serves the Discovery capabilities API (`discovery.capabilities.v2alpha`), so Auto Discovery can list it; the client registers the service endpoint it uses.
- **Prebuilt:** the Live Edit Service chart and image are in the [usd-content-workspace NGC collection](https://catalog.ngc.nvidia.com/orgs/nvidia/teams/omniverse/collections/storage_apis) (released 0.1.1). The client has no public prebuilt package; it ships as source.
- **Boundaries it enforces:** ovlive's `.live` format is not the Nucleus live-layer format of `omni.client`; one process cannot register both. Deploying the service alone makes no application collaborative; each participant integrates the client.
- This repository's Live Edit pages: [../deploy/live-edit.md](../deploy/live-edit.md), [../components/live-edit.md](../components/live-edit.md).

## ovpackage

- **Knows:** the package model (packages, immutable versions, repositories holding a `.packages` directory, install locations, `.<package>.wrapp` metadata files, `.wrappignore` rules, content-hash catalogs); every CLI command (create, install, uninstall, mirror, export and import, freeze, catalog, diff, check, verify, status, patches); and the async Python API, which mirrors the CLI and needs a context manager or explicit initialize and shutdown.
- **Meets this stack at:** the optional `storage-api` backend, configured with `OVPACKAGE_STORAGE_API_DISCOVERY_URL` (a Discovery URL) or `OVPACKAGE_STORAGE_API_GRPC_ADDRESS`. Other backends are local disk, S3, Azure Blob, GCS, and Nucleus. `ovpackage version` reports the Storage API backend as "ENABLED but not configured" until one is set; that is harmless for other backends.
- **Status:** pre-release, not enterprise-supported, not accepting contributions. Installed from source (`python -m pip install .[local,storage-api]`); the `wrapp` command name remains.

## usd-caches

- **Knows:** choosing, sizing, and placing caches; the local cache (Hub, on every workstation or compute node, Protobuf RPC and HTTP); the shared content cache OVCC (an HTTP caching reverse proxy for S3, Azure Blob, GCS, and Nucleus on Kubernetes); and the shared derived-data cache OVDC (gRPC `KeyValue` with `Get`, `Put`, `Touch`, for RTX and UJITSO derived data). It covers building, installing, Helm deployment, every configuration field, API calls, validation, and troubleshooting.
- **Traps it checks:** OVDC pods are independent stores, so the client picks the pod for each key; OVCC with `requiresVersioning: true` bypasses the cache for GETs without `If-Match`; Hub's `cache.garbage_collection_interval` has no effect.
- **Meets this stack at:** content reads by Kit and Client Library workloads. Caches reduce repeated retrieval near consumers; the storage behind this stack stays the source of truth.
- **Prebuilt:** NGC collections for [Hub Workstation Cache](https://catalog.ngc.nvidia.com/orgs/nvidia/teams/omniverse/collections/hub-workstation-cache) and Omniverse Caching Services (OVCC and OVDC charts and images). Older names: USD Content Cache (UCC) is OVCC; Derived Data Cache (DDCS) is OVDC.
- Recorded cache versions and a known Kit interaction: [companion-products.md](companion-products.md).

## Typical combinations

| Workflow | Products |
| --- | --- |
| An application reads and writes content on a deployment of this stack without Kit | This stack (Discovery and Storage) plus ovstorage |
| Teams publish and install versioned asset sets on this stack | This stack plus ovpackage with its `storage-api` backend |
| Live collaborative USD editing | This stack (Discovery, Storage, authorization) plus ovlive's client and Live Edit Service |
| Many users or render nodes read the same content | This stack plus usd-caches (Hub on each node; OVCC and OVDC when shared) |
