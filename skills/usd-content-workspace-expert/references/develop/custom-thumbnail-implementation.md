# Build your own Thumbnail API implementation

> **Applies to:** Thumbnail API `v1alpha` (`nvidia.omniverse.thumbnail.v1alpha`) and the thumbnail conformance suite · **Released:** Thumbnail Service 0.1.0; conformance image and chart are published to NGC by CI with no version pinned in the Developer Guide 1.3.0 inventory · **This repo's snapshot:** conformance crate and chart from service 0.1.0 (`SOURCE_VERSIONS.md`); `helm/thumbnail-conformance-test/Chart.yaml` holds the `0.0.0` placeholder · **Sources:** [`crates/thumbnail-conformance-test/README.md`](../../../../thumbnails/reference/thumbnail-service/crates/thumbnail-conformance-test/README.md) and [`src/`](../../../../thumbnails/reference/thumbnail-service/crates/thumbnail-conformance-test/src/), [`helm/thumbnail-conformance-test/values.yaml`](../../../../thumbnails/reference/thumbnail-service/helm/thumbnail-conformance-test/values.yaml), the [proto](../../../../thumbnails/reference/thumbnail-service/proto/nvidia/omniverse/thumbnail/v1alpha/thumbnail_service.proto) and [OpenAPI](../../../../thumbnails/reference/thumbnail-service/thumbnail.openapi.yaml); adapted from the [source skill package](../../NOTICE.md)

Read [apis/thumbnails.md](../apis/thumbnails.md) for the wire surface first. This page covers what a compatible implementation must do, what is your choice, and how to certify it. The NVIDIA Thumbnail Service, its conformance suite and chart are Apache-2.0; keep the license and notice files if you copy code from them. General guidance for adding a service to the stack (Helm, auth, discovery, observability) is in [develop/add-your-service.md](add-your-service.md).

The source package calls this an *implementation*, not an adapter: you build the service, not a backend behind a fixed service.

## Required versus your choice

| Your choice | Required |
| --- | --- |
| Processes, queues, workers | All four operations on REST and gRPC, with the same validation on both |
| Which processors you generate | Reject unsupported processors, formats and encodings with `400` / `INVALID_ARGUMENT` naming the field, before accepting work; never substitute a different one |
| How you persist associations | The read API returns `target_type`, `association_target`, `thumbnail`, `specification`, `create_time` |
| Where thumbnails are written | A deterministic function of source address, source identity and resolved specification; callers cannot override it |
| How you validate tokens | Refuse requests without a bearer before parsing the body; `401`/`UNAUTHENTICATED` versus `403`/`PERMISSION_DENIED` split exactly |
| Whether you publish change notifications | If you do: only after the change is readable, best-effort, never affecting the response |

The NVIDIA service is one valid shape (orchestrator plus queue-driven worker, associations stored as Storage metadata), not the contract.

## Step 1: choose a completion mode

The Thumbnail API design (source skill package, which is not published in this repository) permits:

| Mode | REST success | gRPC | Failure statuses |
| --- | --- | --- | --- |
| Asynchronous | `202` | `OK`, `succeeded` entries without `resource_identity` | `502` / `INTERNAL` when nothing could be handed downstream or the source lookup failed |
| Synchronous | `200` | `OK`, `succeeded` entries with `resource_identity` | `422` / `FAILED_PRECONDITION` unprocessable source; `503` / `UNAVAILABLE` transient or shed |

The status sets do not overlap: never `502` from a synchronous implementation, never `422` from an asynchronous one. Do not expose the mode as a request option.

This repo's snapshot differs from that text in two ways that affect you:

- The proto and OpenAPI in this snapshot define only `failed` on the generation response; there is no `succeeded` field to populate.
- The snapshot conformance suite expects exactly `202` for its generation acceptance cases (`src/rest/thumbnails.rs`). A synchronous implementation answering `200` fails this suite version. Record that as a suite limitation, not a contract violation, if you build a synchronous service.

## Step 2: implement the four operations

### Generation (`POST /v1alpha/thumbnails`, `CreateThumbnails`)

Validate everything before contacting storage: non-blank `resource_address`; at most 1000 specifications; each entry is a kind you generate (`manual` and empty entries are rejected); width and height in `1..=8192`; `quality` in `1..=100` for `JPEG` and `WEBP` only. One invalid specification rejects the request.

Then resolve the source identity with a storage `Stat` using the caller's token (missing → `404` / `NOT_FOUND`). With no specifications, fan out over your configured default sizes, so API- and event-driven flows produce the same thumbnails. Resolve omitted `quality` and `preserve_aspect_ratio` once, before recording; an explicit `false` stays `false`. A partial result keeps its success status and lists the rest in `failed`.

Over gRPC require `processor_identity` (field 1 of `ImageToImageSpecification`, `UNSPECIFIED` is invalid) but do not store it; the stored specification carries `type`.

### Read (`GET /v1alpha/thumbnails`, `GetThumbnails`)

Require `association_target_resource_address` (≤ 2048). Treat other parameters as exact-match filters and reject unknown parameters or unsupported values. Return `200` / `OK` with an empty list when nothing matches, never not-found. Order by thumbnail address, cap at 1000, no continuation token. Skip and log unreadable entries.

### Associate (`POST /v1alpha/associations`, `AssociateThumbnail`)

Accept every specification kind the type system defines, including kinds you cannot generate (`MANUAL`). `thumbnail` needs address and identity; a `FILE` target needs identity; a `FOLDER` target must not have one. REST success is `204`. Re-associating the same thumbnail, target and specification replaces the entry. Over gRPC report post-validation failures in-band (`success = false`, `error_message`, `failure_kind`); only a missing or rejected token fails the RPC (`UNAUTHENTICATED`). If a two-step write fails halfway, roll back; an association that was not fully recorded must not read back as existing.

### Disassociate (`DELETE /v1alpha/associations`, `DisassociateThumbnail`)

Match only by addresses; ignore identity parameters. Remove only the entry whose stored thumbnail address matches. Delete no bytes. Not associated → `200` / `OK` with both arrays empty.

| Condition | REST | gRPC |
| --- | --- | --- |
| Entry found, removal failed | `502` with body | `OK`, one entry in `failed` |
| Target cannot be enumerated | `502`, both arrays empty | `UNAVAILABLE` |
| Not associated | `200`, both empty | `OK`, both empty |

If you sweep associations when a source is deleted, delete only bytes your service generated. The NVIDIA service distinguishes generated from caller-registered thumbnails by the stored specification kind and the template shape, because the sweep runs with service rights the caller may not have (`crates/common/tests/disassociate_all_caller_owned_blob.rs`).

## Step 3: addresses

Treat a raw address and its percent-encoded form as the same address for lookup. Never canonicalize or look up by `resource_identity`. Associations are keyed by target address for files and folders alike.

## Step 4: destinations and recursion

Choose destinations as a deterministic function of source address, identity and resolved specification, so regeneration overwrites. Whether a new source version gets a new thumbnail depends on whether you fold the identity into the destination. An event-driven implementation sees its own output arrive as storage CREATE events; recognize it (destination shape, writer principal, or both) before deciding eligibility. The NVIDIA service relies on template-shape matching because Storage does not yet propagate a writer principal ([ADR 0001](../../../../thumbnails/reference/thumbnail-service/docs/adr/0001-thumbnail-storage.md)).

## Step 5: authentication

Require `Authorization: Bearer` (REST) or `authorization` metadata (gRPC) on every API call. Return as little as possible on `401`/`403`: no reason the credential failed, no deciding component, no backend text, no `details`, nothing in `grpc-status-details-bin`. Delegating to Storage (as the NVIDIA service does) or validating JWTs yourself (issuer, audience, expiry, signature) are both acceptable. If you write on behalf of callers with a service identity, check the caller's access first so the service identity is not a privilege escalation path.

## Step 6: register with Discovery

Serve `nvidia.omniverse.discovery.capabilities.v2alpha.CapabilitiesService/ListServices` without authentication, returning one entry of service type `thumbnail` that advertises gRPC `nvidia.omniverse.thumbnail.v1alpha.ThumbnailService` and REST endpoint `thumbnail` version `v1alpha` path `/v1alpha/`. Annotate the Kubernetes Service with `discovery.omniverse.nvidia.com/enabled: "true"`; name the gRPC port `grpc` or add `discovery.omniverse.nvidia.com/grpc-port`; name the REST port `http` or `rest` (operator README). Contracts: [apis/discovery.md](../apis/discovery.md). Deployment: [deploy/discovery.md](../deploy/discovery.md). Confirm the entry in `GET /api/v2/services` with `"schema-version": 2`, then test the advertised URLs from the client network.

## Certify with the conformance suite

A black-box suite in [`crates/thumbnail-conformance-test`](../../../../thumbnails/reference/thumbnail-service/crates/thumbnail-conformance-test/) checks paths, statuses, shapes and error mapping on both transports. It prints `[PASS]`/`[FAIL]` per case tagged `[REST]` or `[gRPC]` and exits `0` only when every selected case passed; any failure or skip exits `1`. It does not check that a thumbnail was generated. Repository-wide suite guidance: [test/conformance.md](../test/conformance.md).

Before the first run:

- `--rest-url` is required for every suite, including `grpc`: preflight reads `specifications.maxItems` from `/api-docs/openapi.json` and requires exactly `1000` before sending the over-bound probe of 1001. A missing or malformed document makes the run unverifiable.
- `--source-file` must exist in storage; generation cases `Stat` it and fail `404` otherwise.
- Three acceptance cases per transport (omitted quality, PNG quality 200, GIF quality 200) can generate real thumbnails that the suite cannot remove.
- Against a deployment that does not enforce the 1000 bound, the over-bound probe can write 1001 associations onto the source until its metadata becomes unreadable (`502` on later reads). Use a source you can abandon.
- The suite always sends `processor_identity = IMAGE_TO_IMAGE` and has no unsupported-processor case; test processor rejection yourself.

| Flag | Env | Default / requirement |
| --- | --- | --- |
| `--rest-url` | `REST_URL` | Required for every suite |
| `--grpc-url` | `GRPC_URL` | Required for `grpc` and `all` |
| `--grpc-tls-enabled` | `GRPC_TLS_ENABLED` | `false` |
| `--test-suite` | `TEST_SUITE` | `all`; or `rest`, `grpc` |
| `--source-file` | `SOURCE_FILE` | Required |
| `--ready-path` | `READY_PATH` | `/health` |
| `--http-timeout` | `HTTP_TIMEOUT` | `30s`; bounds REST requests and gRPC connect and calls |
| `--wait-retries` / `--wait-interval` | `WAIT_RETRIES` / `WAIT_INTERVAL` | `10` / `5s` |

Authentication: exactly one of `--bearer-token` (`BEARER_TOKEN`), `--access-token-file` (`ACCESS_TOKEN_FILE`, JSON with `access_token`), or OAuth2 client credentials (`--oauth2-openid-configuration-uri`, `--oauth2-client-id`, `--oauth2-client-secret` or `--oauth2-client-secret-file`, `--oauth2-client-scope`). Omit all three only against a deployment without authentication. Prefer the token file or secret file over passing a secret on the command line.

```bash
# From the service workspace (the crate path-depends on ../../../common and ../../../permissions)
cd thumbnails/reference/thumbnail-service
cargo run -p thumbnail-conformance-test -- \
  --rest-url http://localhost:3000 --grpc-url http://localhost:50051 \
  --source-file "https://<account>.blob.core.windows.net/<test-container>/conformance/source.png" \
  --access-token-file /path/to/token.json
```

The image is built from `Dockerfile.conformance-test` with the repository root as context.

### In-cluster Job

The chart renders a one-shot Job (`backoffLimit: 0`, `ttlSecondsAfterFinished: 600`) and a ServiceAccount.

```yaml
conformanceTest:
  restUrl: "http://thumbnail-service.<namespace>.svc.cluster.local:3000"
  grpcUrl: "http://thumbnail-service.<namespace>.svc.cluster.local:50051"
  testSuite: "all"
  sourceFile: "https://<account>.blob.core.windows.net/<test-container>/conformance/source.png"
image:
  tag: "<version you were given>"     # no released version is pinned; do not guess
serviceIdentity:
  bearerToken:
    enabled: true
    secretRef:
      name: thumbnail-conformance-token   # existing Secret
      key: BEARER_TOKEN
```

```bash
helm template conformance thumbnails/reference/thumbnail-service/helm/thumbnail-conformance-test -f conformance-values.yaml
helm install conformance thumbnails/reference/thumbnail-service/helm/thumbnail-conformance-test -n <namespace> -f conformance-values.yaml
kubectl -n <namespace> logs job/conformance-thumbnail-conformance-test
```

`serviceAccount.automount` defaults to `true` in this chart; set it to `false` unless a token provisioner needs the Kubernetes API.

### Run faults versus contract failures

The summary adds a `[WARN]` block per cause. None of these certify conformance: source not found (`404` with the API's error body), credentials rejected (`401`/`UNAUTHENTICATED`), access denied (`403`/`PERMISSION_DENIED`), endpoint not serving the API (`404` without the body, or `UNIMPLEMENTED`), client timeout, unverifiable OpenAPI. Skipped dependent cases also block certification.

| Common mistake | Required behavior |
| --- | --- |
| `404` for a target with no thumbnails | `200` / `OK`, empty list |
| `404` when disassociating something not associated | `200` / `OK`, both arrays empty |
| Range-checking `quality` for PNG and GIF | Check JPEG and WebP only |
| `200` from associate | `204` |
| Identity required on a `FOLDER` target | Must be omitted |
| Ignoring unknown query parameters | Reject |
| Accepting more than 1000 specifications | `400` / `INVALID_ARGUMENT` |
| Accepting `manual` for generation | Reject |
| Mixing the two REST specification shapes | Flat `type`-tagged for associate; outer-keyed for generate |
| Failing the RPC on an association write error | In-band `success = false` with `failure_kind` |
| Requiring auth on `ListServices` | Must be unauthenticated |
