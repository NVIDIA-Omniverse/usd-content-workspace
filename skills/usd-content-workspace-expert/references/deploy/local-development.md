# Local development

> **Applies to:** Python filesystem reference service (Storage API example), Auto Discovery operator, legacy Discovery chart, Storage Service, Navigator · **Released:** the example ships inside the NGC `storage-api` resource (1.0.0-beta.4 in the 1.3.0 inventory); Auto Discovery 1.1.1; legacy `discovery-service` 2.3.11 · **This repo's snapshot:** filesystem example, unreleased ([`pyproject.toml`](../../../../storage/api/filesystem_example/pyproject.toml)); operator 1.2.0 ([`Cargo.toml`](../../../../discovery/reference/discovery-service/operator/Cargo.toml)); legacy chart 2.4.1, unreleased ([`VERSION.toml`](../../../../discovery/reference/discovery-service/chart/VERSION.toml)) · **Sources:** the [source skill package](../../NOTICE.md); files linked below

Two kinds of local work:

- **A learning stack** — the Python filesystem reference service plus Discovery on a local cluster, so Kit, Navigator, or `curl` can reach a Storage API endpoint. Not a production backend.
- **Component development loops** — building, running, and testing one component from this repository against local dependencies.

To skip Kubernetes entirely, run Discovery, Storage, and Metadata as plain local processes instead: [local-process.md](local-process.md).

| Read | For |
| --- | --- |
| [local-development/filesystem-example-stack.md](local-development/filesystem-example-stack.md) | Run the filesystem service locally, containerize it, deploy it to any Kubernetes cluster, add Discovery, validate from `curl` and Kit |
| [local-development/microk8s-and-ingress.md](local-development/microk8s-and-ingress.md) | MicroK8s setup, image import, hostpath storage, NGINX ingress with TCP pass-through, troubleshooting, cleanup |
| [local-development/component-dev-loops.md](local-development/component-dev-loops.md) | Per-component local loops: Storage Service, Auto Discovery operator, legacy Discovery chart, Navigator UI, and where the other components document theirs |

## Port differences to remember

| Service | gRPC | REST | Other |
| --- | --- | --- | --- |
| Python filesystem example | 50051 | 8011 | — |
| NVIDIA Storage Service (production adapter) | 8011 | 8012 | metrics 8013 when `service.metrics.enabled`; gRPC health probe 8100 |
| Auto Discovery operator | — | internal 8080, external 8081 | — |
| Legacy Discovery chart (nginx) | — | 8080 | `/health`, `/metrics` (nginx `stub_status`) |
| Storage Navigator (chart) | — | 8080 | — |

Port 8011 is REST on the example and gRPC on the production adapter. Do not copy port numbers between the two.

## Discovery choice for the example stack

The operator learns what a target serves by calling the gRPC `CapabilitiesService/ListServices`: the snapshot operator tries `nvidia.omniverse.discovery.capabilities.v2beta` and falls back to the deprecated `v2alpha` on `UNIMPLEMENTED`; released 1.1.1 uses v2alpha. In this snapshot the filesystem example registers only the Storage capabilities services (`nvidia.omniverse.storage.capabilities.v1alpha` and `v1beta`, see [`grpc_service/server.py`](../../../../storage/api/filesystem_example/src/local_filesystem_service/grpc_service/server.py)), not either discovery capabilities version. A target whose capability query never succeeds is omitted from the catalog ([operator discovery doc](../../../../discovery/reference/discovery-service/operator/docs/service/docs/discovery.md#failure-behavior)), so **annotating the example's Service does not register it with Auto Discovery in this snapshot**. The source skill package example walkthrough says to annotate the Service; that does not work against this source.

Options, in order of preference for local work:

1. Use the **legacy Discovery chart** with a static entry for the example (v1 catalog). Navigator falls back to v1. See [filesystem-example-stack.md](local-development/filesystem-example-stack.md#add-discovery).
2. Point clients straight at the example's REST/gRPC URLs and skip Discovery.
3. Use the NVIDIA Storage Service (which serves discovery v2alpha) with Auto Discovery, as in [production-stack.md](production-stack.md).

If you are building your own storage service and want Auto Discovery to find it, implement discovery `ListServices`: v2alpha works with released 1.1.1 and, through the fallback, with the snapshot operator; v2beta (only in the operator's `discovery-protos/` so far) is what the snapshot calls first and what its conformance runner checks; see [../apis/discovery.md](../apis/discovery.md) and [../develop/add-your-service.md](../develop/add-your-service.md).

## Cleanup rules

Limit cleanup to resources you created. Confirm the kube context, namespace, and Helm release names first. Use the release name recorded at install time, not the chart name. Deleting a namespace removes every Secret, PVC, and service in it; do it only for a confirmed disposable namespace whose data you do not need.
