# Discovery APIs: catalog, capabilities, auth bootstrap

> **Applies to:** Discovery Catalog API (served by Auto Discovery), the v1 compatibility catalog, the auth-config bootstrap, and the Capabilities API every discoverable service implements · **Released:** Auto Discovery 1.1.1 serves catalog v2 (`schema-version: 2`); no separate NGC discovery API resource is listed in the 1.3.0 inventory · **This repo's snapshot:** discovery protos untagged `main` at `a5f1df2` ([`SOURCE_VERSIONS.md`](../../../../SOURCE_VERSIONS.md)), latest release commit `Release discovery-protos 0.0.0-manual.a5f1df25` (changes only the Buf dependency; no `.proto` changes); `discovery/api` has only gRPC `nvidia.omniverse.discovery.capabilities.v2alpha`. The unreleased snapshot operator 1.2.0 adds `nvidia.omniverse.discovery.capabilities.v2beta` in its own `discovery-protos/` and serves the catalog at `/api/v2beta/services`; Capabilities REST OpenAPI `info.version: v1beta`; Catalog OpenAPI `info.version: v2beta` · **Sources:** [`discovery/api/proto/`](../../../../discovery/api/proto/); operator [`openapi/`](../../../../discovery/reference/discovery-service/operator/openapi/) and [`docs/api/docs/`](../../../../discovery/reference/discovery-service/operator/docs/api/docs/index.md); operator [`src/api/rest.rs`](../../../../discovery/reference/discovery-service/operator/src/api/rest.rs), [`src/client/capabilities.rs`](../../../../discovery/reference/discovery-service/operator/src/client/capabilities.rs); the [source skill package](../../NOTICE.md)

Three roles: a **target service** reports what it implements through the Capabilities API; a **discovery service** collects those reports, pairs them with reachable URLs, and serves the Catalog API; a **client** reads the catalog and picks an interface by capability. Deployment is in [../deploy/discovery.md](../deploy/discovery.md).

## Contract files

| Contract | Wire version | Definition in this repo |
| --- | --- | --- |
| Catalog API | REST `GET /api/v2/services` (released 1.1.1); snapshot primary route `GET /api/v2beta/services` with `/api/v2/services` as an alias returning the same body; `schema-version: 2` in both | [`openapi/discovery.openapi.yaml`](../../../../discovery/reference/discovery-service/operator/openapi/discovery.openapi.yaml) (the conformance runner calls its bundled copy a draft transcription of the docs), spec text [`discovery-catalog-api.md`](../../../../discovery/reference/discovery-service/operator/docs/api/docs/discovery-catalog-api.md) |
| Capabilities, gRPC | `nvidia.omniverse.discovery.capabilities.v2alpha.CapabilitiesService/ListServices` (`discovery/api`, deprecated in the operator's copy); `...capabilities.v2beta.CapabilitiesService/ListServices` (operator `discovery-protos/` only, wire-identical apart from the package) | [`capabilities.proto`](../../../../discovery/api/proto/nvidia/omniverse/discovery/capabilities/v2alpha/capabilities.proto), [`capabilities_service.proto`](../../../../discovery/api/proto/nvidia/omniverse/discovery/capabilities/v2alpha/capabilities_service.proto); Buf module `pb.api.nvidia.com/omniverse/discovery` ([`buf.yaml`](../../../../discovery/api/proto/buf.yaml)); v2beta in [`operator/discovery-protos/`](../../../../discovery/reference/discovery-service/operator/discovery-protos/README.md) |
| Capabilities, REST | `GET /v1beta/services` | [`openapi/capabilities_clean.openapi.yaml`](../../../../discovery/reference/discovery-service/operator/openapi/capabilities_clean.openapi.yaml), [`rest-api.md`](../../../../discovery/reference/discovery-service/operator/docs/api/docs/rest-api.md) |
| v1 compatibility catalog | `GET /api/v1/services`, `/api/v1alpha/services`, `schema-version: 1` | [`v1-response-schema.md`](../../../../discovery/reference/discovery-service/operator/docs/v1-response-schema.md) |
| Auth bootstrap | `GET /api/v1/auth-config`, `/api/v2/auth-config`; the snapshot operator also serves `/api/v1alpha/auth-config` and `/api/v2beta/auth-config` | operator chart helper `discovery.authConfigJson` ([`_helpers.tpl`](../../../../discovery/reference/discovery-service/operator/helm/auto-discovery-service/templates/_helpers.tpl)) |

The gRPC and REST Capabilities representations are versioned independently (`v2alpha`/`v2beta` vs `v1beta`) and use different field names; do not treat them as one wire format. Copies of the v2alpha protos also live in the operator (`discovery-protos/`, beside v2beta), Storage Service (`storage-service-protos/proto/nvidia/omniverse/discovery/`), Permission Service, notification-common, and Thumbnail Service; they are updated by hand. No service in this snapshot serves v2beta yet. Generate clients from the copy the deployed service was built from. [`discovery/api/README.md`](../../../../discovery/api/README.md) still names the package `v1beta`; the files are `v2alpha`.

## Catalog API v2

```json
{
  "schema-version": 2,
  "services": [
    {
      "name": "storage-service",
      "capability_source": "grpc",
      "rest": {
        "url": "https://storage.example.com",
        "endpoints": [
          { "name": "fileobject", "versions": { "v1beta": { "path": "/v1beta" }, "v1alpha": {} } }
        ]
      },
      "grpc": {
        "url": "https://grpc.storage.example.com",
        "endpoints": [ { "service": "nvidia.omniverse.storage.fileobject.v1beta.FileObjectService" } ]
      }
    }
  ]
}
```

The paths above are illustrative; real values come from each service's capability report. Rules clients must follow (from the spec):

- Reject any `schema-version` other than 2. Ignore unknown members. Do not rely on array or member order.
- `name` is assigned by the discovery service and identifies the target (the operator uses the Kubernetes Service name); it is not an API name.
- `rest` and `grpc` are optional; a service may have neither (no usable API).
- REST: pick an `endpoints[]` entry by `name`, then a key of its `versions` **object** (in the Capabilities API it is an array). Call `rest.url` + `path`. If `path` is absent, take the path from that API's own specification; do not invent one.
- gRPC: pick the entry whose `grpc.endpoints[].service` equals the fully qualified service your client was generated for, then connect to `grpc.url`. `https` means TLS, `http` plaintext. Never derive a gRPC name from a REST name.
- A `200` with an empty `services` array is a valid empty catalog; `404` must not mean "no services".
- The operator sends `Cache-Control: max-age=30, must-revalidate`; clients may cache that long.
- The operator serves two catalogs: internal (port 8080, cluster DNS URLs) and external (port 8081, client-facing URLs). Each audience must receive only URLs it can reach.

Selecting Storage over REST with `jq`:

```bash
curl -fsS "${DISCOVERY%/}/api/v2/services" | jq -r '
  select(."schema-version" == 2) | .services[]
  | select(.rest.endpoints[]? | .name == "fileobject" and (.versions | has("v1beta")))
  | .rest.url'
```

Storage Service note: since 1.2.1 ([`CHANGELOG.md`](../../../../storage/reference/storage-service/CHANGELOG.md)) its capability report advertises the REST version mount base (for example `/v1alpha`) as the `path`, and clients combine it with the resource paths from the Storage OpenAPI documents ([storage.md](storage.md)). Navigator 1.0.4 appends the resource name to the v2 path for that reason. The snapshot Navigator source also looks for endpoint `metadata` at `v1beta` before `v1alpha`, and for an optional endpoint named `archive` at `v1alpha` (v1 fallback: service `type: "archive"` plus `/v1alpha/`). No service in this repository advertises `archive`, and the operator's default v1 mappings do not include it ([component map](../components/navigator.md#archive-service-contract-client-side-only)).

## v1 compatibility catalog

```json
{ "schema-version": 1, "services": [ { "id": "storage-service", "name": "Storage Service", "type": "storage",
  "rest": "https://storage.example.com", "grpc": "https://grpc.storage.example.com" } ] }
```

`type` values in use: `storage`, `notification-aggregation`, `notification-consumer`, `permission`, `userinfo`, `thumbnail`, `live-edit`, `live-service` (legacy Live Edit). The operator builds this from v2 data with configurable mappings ([../deploy/discovery.md](../deploy/discovery.md#v1-compatibility)); ids get `-0`, `-1`, ... suffixes when several targets match one mapping. The legacy static chart emits the same shape from Helm values. Migration guidance: [`v1-to-v2-migration.md`](../../../../discovery/reference/discovery-service/operator/docs/v1-to-v2-migration.md).

## Auth bootstrap

```json
{
  "_comment": "OpenID Authentication",
  "openid_configuration": "https://idp.example.com/.well-known/openid-configuration",
  "clients": {
    "default":        { "client_id": "<id>", "scope": "<scopes>" },
    "client_library": { "client_id": "<id>", "scope": "<scopes>" },
    "navigator":      { "client_id": "<id>", "scope": "<scopes>" }
  }
}
```

Served unauthenticated so clients can learn how to sign in. A `404` means the deployment advertises no OIDC configuration. Client keys in use: `client_library` (Kit / Client Library), `navigator` (Navigator UI), falling back to `default`. These are response fields, not Helm keys. The response does not validate tokens or grant access.

## Capabilities API (what a target implements)

gRPC v2alpha (v2beta has the same messages under package `...capabilities.v2beta`):

```protobuf
service CapabilitiesService {
  rpc ListServices(ListServicesRequest) returns (ListServicesResponse);
}
message ListServicesRequest  { int32 page_size = 1; string page_token = 2; }
message ListServicesResponse { repeated ServiceEntry services = 1; string next_page_token = 2; }
message ServiceEntry { string service_name = 1; repeated GrpcService grpc_services = 2; repeated RestService rest_services = 3; }
message GrpcEndpoint { string service_name = 1; }                        // fully qualified gRPC service
message RestEndpoint { string name = 1; repeated RestVersion versions = 2; }
message RestVersion  { string version = 1; RestVersionDetail url_detail = 2; }
message RestVersionDetail { string path = 1; }
```

REST v1beta (`GET /v1beta/services`) carries the same hierarchy with JSON names `grpc`, `rest`, `service`, `detail.path`.

Requirements imposed by the snapshot operator, beyond the contract:

- It calls gRPC v2beta `ListServices` with no metadata and no `page_size`, a 5 s connect and 10 s request timeout. If the target answers `UNIMPLEMENTED`, it retries on the deprecated v2alpha method and logs a deprecation warning; that is how every service in this snapshot, all v2alpha-only, still registers. It follows `next_page_token` for up to 100 pages; a failed page, a repeated token, or more pages fail the whole query and keep the previous entry. Serve this RPC **without authentication**, or the target is never registered. Released operator 1.1.1 predates the v2beta client; serve v2alpha for it.
- It does not use the REST representation. The Storage Service additionally serves a REST form at `/v2alpha/services` and its storage-specific capabilities at `/v1alpha/capabilities/services` and `/v1beta/capabilities/services`; those are not the Capabilities REST v1beta contract.
- Catalog building merges REST endpoints with the same name across entries and de-duplicates gRPC names ([catalog spec "Field Mapping"](../../../../discovery/reference/discovery-service/operator/docs/api/docs/discovery-catalog-api.md)).

Implementing it in your own service: [../develop/add-your-service.md](../develop/add-your-service.md).

## Generating clients

```bash
cd discovery/api/proto
buf lint
buf generate --template <your buf.gen.yaml>      # or protoc with your language plugin
```

`buf.yaml` depends on `buf.build/protocolbuffers/wellknowntypes`, the well-known types on the public Buf Schema Registry, so `buf` needs no NVIDIA registry access to resolve it. Earlier imports of these protos named an NVIDIA-hosted copy of that dependency instead. No `buf.lock` is checked in, so `buf` resolves the dependency itself on first use (`buf dep update` writes a lock). The module's own name is still `pb.api.nvidia.com/omniverse/discovery`. With plain `protoc`, use the well-known types shipped with protoc. The operator's conformance runner ships generated Python clients and a script to regenerate them ([`conformance/scripts/generate-protos.sh`](../../../../discovery/reference/discovery-service/operator/conformance/scripts/generate-protos.sh)).

## Validating an implementation

The read-only runner checks a catalog (`/api/v2beta/services` or the `/api/v2/services` alias) against the schema-version 2 profile and, separately, explicit REST v1beta and gRPC **v2beta** capability endpoints. A target that serves only v2alpha, as every service in this snapshot does, fails the gRPC check with `UNIMPLEMENTED` even though the operator registers it through its fallback; exit code 0 means the selected checks passed, 1 a contract violation, 2 a configuration error, 3 unavailable prerequisites:

```bash
cd discovery/reference/discovery-service/operator
uv run --project conformance --locked discovery-conformance \
  --grpc-target backend.example.com:443 --rest-capabilities-url https://backend.example.com/v1beta/services \
  --capabilities-token-file /run/secrets/backend-token --json-report result.json
```

A schema pass does not prove inventory, freshness, ingress visibility, or authorization. See [../test/conformance.md](../test/conformance.md).
