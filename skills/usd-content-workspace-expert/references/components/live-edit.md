# Component map: Live Edit (external)

> **Applies to:** Live Edit Service and its LiveService protocol · **Released:** `live-edit-service` chart and image 0.1.1 (Developer Guide 1.3.0 / NGC inventory) · **This repo's snapshot:** not present. [ovlive](https://github.com/NVIDIA-Omniverse/ovlive) holds the client runtime under `client/` (client 0.1.0 experimental, `client/CHANGELOG.md`) and the Live Edit Service under `service/`: source, `proto/nvidia/omniverse/live/v1alpha/live_service.proto`, and the `helm/live-edit-service` chart, replaced by each service release (`service/CHANGELOG.md` records 0.1.1 and later unreleased changes) · **Sources:** ovlive `README.md`, `client/source/live/protos/PROVENANCE.md`, `client/CHANGELOG.md`; the [source skill package](../../NOTICE.md) Live Edit references; this repo's Discovery values

Task pages in this tree: [deploy](../deploy/live-edit.md), [API](../apis/live-edit.md), [operations](../operate/live-edit.md), [known issues](../operate/known-issues.md#live-edit), [upgrades](../operate/upgrades.md#live-edit-external).

## Facts

| Item | Value | Where |
| --- | --- | --- |
| gRPC | `50001` (LiveService, health, reflection, capabilities) | source package (released chart) |
| Metrics | HTTP `9090` `/metrics` | source package |
| REST | none | source package |
| Proto package | `nvidia.omniverse.live.v1alpha`, service `LiveService` | vendored proto |
| Proto files | upstream `proto/nvidia/omniverse/live/v1alpha/live_service.proto` (service repo, main `13a3099`); vendored as `client/source/live/protos/nvidia/omniverse/live/v1alpha/live.proto` with additions | ovlive `PROVENANCE.md` |
| Discovery | `discovery.omniverse.nvidia.com/enabled: "true"` plus `discovery.omniverse.nvidia.com/grpc-port: "50001"`; `ListServices` advertises `LiveService` | source package; operator annotation rules in this repo's [operator README](../../../../discovery/reference/discovery-service/operator/README.md) |
| v1 compatibility types | `live-edit` and legacy `live-service`, both mapped from `nvidia.omniverse.live.v1alpha.LiveService` | [operator values `config.v1Endpoints`](../../../../discovery/reference/discovery-service/operator/helm/auto-discovery-service/values.yaml); static entry `live-edit-service-01` in the [legacy chart values](../../../../discovery/reference/discovery-service/chart/helm/values.yaml) |
| Backing store | Valkey 7.2+ or Redis 7.x, `noeviction`, customer-provided | source package |
| Access checks | Caller token forwarded to Storage (`HEAD /v1alpha/fileobject/by-address/{address}` or gRPC `Stat`) | source package |
| License | Apache-2.0 for the service, the client, and the vendored proto | ovlive `LICENSE`, `service/LICENSE.txt`, proto header |

## Where things are

| Need | Location |
| --- | --- |
| Released chart and its `values.yaml` | NGC `live-edit-service-0.1.1.tgz` ([deploy/live-edit.md](../deploy/live-edit.md#install)); extract and read before generating values |
| Service source and chart | ovlive `service/` (`service/README.md`, `service/docs/`, `service/helm/live-edit-service/`). Its `[Unreleased]` changelog entries are snapshot behavior; quote the 0.1.1 release for NGC installs. For anything beyond this page, hand off to the ovlive expert ([../use/related-experts.md](../use/related-experts.md#ovlive)) |
| Wire schema | Reflection on the running service; vendored copy at [`client/source/live/protos/.../live.proto`](https://github.com/NVIDIA-Omniverse/ovlive/blob/main/client/source/live/protos/nvidia/omniverse/live/v1alpha/live.proto) and its [`PROVENANCE.md`](https://github.com/NVIDIA-Omniverse/ovlive/blob/main/client/source/live/protos/PROVENANCE.md) |
| Client runtime (C ABI, C++ USD adapter, Python `omni.usd.live`, Kit extension `omni_usd_live`) | [ovlive README](https://github.com/NVIDIA-Omniverse/ovlive#readme), [`client/docs/ovlive-reference.md`](https://github.com/NVIDIA-Omniverse/ovlive/blob/main/client/docs/ovlive-reference.md), [`ovlive-kit.md`](https://github.com/NVIDIA-Omniverse/ovlive/blob/main/client/docs/ovlive-kit.md), [`ovlive-python.md`](https://github.com/NVIDIA-Omniverse/ovlive/blob/main/client/docs/ovlive-python.md) |
| OMPF/OODF delta encoding | ovlive `client/source/live/wire/` |
| Client support boundary and limits | [`client/docs/ovlive-support-boundary.md`](https://github.com/NVIDIA-Omniverse/ovlive/blob/main/client/docs/ovlive-support-boundary.md) |
| Client diagnostics | [`client/docs/ovlive-diagnostic-schema.md`](https://github.com/NVIDIA-Omniverse/ovlive/blob/main/client/docs/ovlive-diagnostic-schema.md) |
| Test reference service (fixture, not deployable) | ovlive `client/tests/reference_service/` |
| Integration tests against a deployed service | usd-content-workspace integration tests 1.0.23, marker `live_edit` ([test/integration-tests.md](../test/integration-tests.md)) |

## Version notes

- The released service is 0.1.1; an earlier preparation pin was 0.1.0.
- The vendored proto adds operation IDs, chained writes and client keepalive ahead of the service, to be reconciled when the service moves into the ovlive repository. A 0.1.1 server may ignore them; check `PingResponse.operation_ids` and `.chained_writes`.
- ovlive 0.1.0 release targets: Kit USD 25.11 (Linux x86-64, Linux aarch64, Windows x86-64) and stock USD 25.11 (Linux x86-64), Python 3.12; macOS arm64 is a development build. ovlive `.live` layers are separate from Nucleus live layers served by `omni.client`.

## Related areas

Storage holds base assets and makes the access decision ([components/storage-service.md](storage-service.md)); User Info resolves opaque tokens when enabled ([deploy/userinfo.md](../deploy/userinfo.md)); Discovery publishes the endpoint ([deploy/discovery.md](../deploy/discovery.md)); Client Library live sessions are covered in [components/client-library.md](client-library.md).
