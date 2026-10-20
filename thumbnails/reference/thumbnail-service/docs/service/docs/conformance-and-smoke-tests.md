(thumbnail-service-conformance-and-smoke-tests-conformance-and-smoke-tests)=
# Conformance and smoke tests

Two standalone test binaries ship with the Thumbnail Service, each with its own job. The **smoke test** proves a deployment came up after an install or upgrade and stops at the first failure; it runs as a Helm test hook from the service image. The **conformance test** certifies a running deployment against the published REST and gRPC contract — every case runs so one invocation reports the full extent of any non-conformance — and is published to NGC as its own image and Helm chart so customers can run it against their own deployments.

Neither test asserts that a requested thumbnail was actually generated. Generation is asynchronous work on the worker, and both tests deliberately stop at the immediate API response.

(thumbnail-service-conformance-and-smoke-tests-smoke-test)=
## Smoke test

The smoke test executes a compact end-to-end API flow against the REST surface:

1. Health check — `GET /health`.
2. Associate a synthetic thumbnail with the configured source file — `POST /v1alpha/associations`.
3. Read the source file's thumbnails back and verify the association is returned — `GET /v1alpha/thumbnails`.
4. Remove the association — `DELETE /v1alpha/associations`.

It exits `0` when every check passes and `1` when any mandatory check fails. Because it never calls `CreateThumbnails`, the source file it is pointed at does **not** need to exist in storage — the association endpoints only write and read metadata keyed on an address.

(thumbnail-service-conformance-and-smoke-tests-running-the-smoke-test-as-a-helm-hook)=
### Running the smoke test as a Helm hook

The chart renders an opt-in Job annotated for the `test`, `post-install`, and `post-upgrade` hooks. It runs `/app/thumbnail-smoke-test` from the service image, so no separate image is needed:

```{code-block} yaml
:caption: thumbnail-values.yaml

smokeTest:
  enabled: true
  sourceFile: "https://acct.blob.core.windows.net/assets/smoke/source-image.png"
  waitRetries: 10
  waitInterval: 5
  requestTimeout: 15
  ttlSecondsAfterFinished: 1800
```

```bash
# run the hook against an installed release
helm test thumbnail-service -n storage-apis

# inspect the Job's output
kubectl logs -n storage-apis job/thumbnail-service-smoke-test
```

| Value | Default | Description |
|---|---|---|
| `smokeTest.enabled` | `false` | Render the smoke-test Job. |
| `smokeTest.image.repository` / `.pullPolicy` / `.tag` / `.pullSecrets` | `""` / `""` / `""` / `[]` | Optional overrides; each falls back to the service image and the root `image.*` settings. |
| `smokeTest.serviceUrl` | `""` | Explicit API base URL. Empty uses the in-cluster Service, `http://<service>.<namespace>.svc.cluster.local:<restPort>`. |
| `smokeTest.serviceAccountName` | `""` | Dedicated ServiceAccount; empty uses the service's. |
| `smokeTest.sourceFile` | `""` | Source address the test associates against. |
| `smokeTest.waitRetries` / `smokeTest.waitInterval` | `10` / `5` | Readiness retries and the interval between them, in seconds. |
| `smokeTest.requestTimeout` | `15` | Per-request timeout, in seconds. |
| `smokeTest.ttlSecondsAfterFinished` | `1800` | Completed Job cleanup TTL; `0` disables. |
| `smokeTest.extraAnnotations` / `.podAnnotations` / `.podLabels` | `{}` | Additional Job annotations (hook keys are stripped), pod annotations, and pod labels. |
| `smokeTest.resources` | requests `250m` / `64Mi`, limits `1` / `256Mi` | Container resources; matches the orchestrator's own defaults, since the test exercises the same REST/gRPC surface. |

In access-token-file identity mode the Job runs the same provisioner sidecar as the service and reads the token from the shared volume; in client-credentials mode it mounts the same Secret. See {ref}`Outbound service identity <thumbnail-service-authentication-outbound-service-identity-serviceidentity>`.

(thumbnail-service-conformance-and-smoke-tests-running-the-smoke-test-directly)=
### Running the smoke test directly

```bash
# against a port-forwarded or externally reachable REST endpoint
thumbnail-smoke-test \
  --service-url http://localhost:3000 \
  --source-file https://acct.blob.core.windows.net/assets/smoke/source-image.png \
  --auth-token "$BEARER_TOKEN"
```

| Argument | Environment variable | Default |
|---|---|---|
| `--service-url` | `SERVICE_URL` | required |
| `--auth-token` | `AUTH_TOKEN` | empty |
| `--auth-token-file` | `AUTH_TOKEN_FILE` | empty — a raw bearer string, or JSON with an `access_token` field as a provisioner sidecar writes it; takes precedence over `AUTH_TOKEN` |
| `--source-file` | `SOURCE_FILE` | required |
| `--wait-retries` | `WAIT_RETRIES` | `10` |
| `--wait-interval` | `WAIT_INTERVAL` | `5` (seconds) |
| `--request-timeout` | `REQUEST_TIMEOUT` | `15` (seconds) |

(thumbnail-service-conformance-and-smoke-tests-conformance-test)=
## Conformance test

The conformance test is a **black-box** suite: it validates wire paths, status codes, response shapes, and the documented error mapping of a deployed Thumbnail Service on both transports. It runs every independent case, prints a `[PASS]`/`[FAIL]` line per case tagged `[REST]` or `[gRPC]`, and ends with a summary of pass/fail counts and per-case timing. The process exits `0` only when every selected case passed; a failure **or a skip** exits `1`.

The image `nvcr.io/nvidia/omniverse/thumbnail-conformance-test` and the chart `thumbnail-conformance-test` are built and published to NGC by CI but are never run automatically against a live deployment — you run them when you want to certify one.

(thumbnail-service-conformance-and-smoke-tests-before-you-run-it)=
### Before you run it

- **`--rest-url` is required for every suite, including gRPC-only runs.** Before any case runs, the suite fetches the served OpenAPI document from `/api-docs/openapi.json`, reads `specifications.maxItems`, and compares it with the independently documented bound of `1000`. Only a match allows the over-bound probe (`advertised + 1` specifications). A lower or higher advertised value fails naming both numbers and sends no probe; a missing or malformed document makes the run **unverifiable** with a nonzero exit. Nothing is ever tested *at* the bound — that could enqueue 1,000 conversions.
- **`--source-file` must be an object that really exists in storage.** The associate, read, and disassociate cases pass against any address, but `CreateThumbnails` resolves the source with a storage `Stat`, so its acceptance cases fail with `404` / `NOT_FOUND` unless the address resolves. The summary then names `--source-file` as the cause rather than reporting a bare failure count.
- **The run leaves generated thumbnails behind.** Three acceptance cases per transport — a valid specification with omitted quality, PNG quality `200`, and GIF quality `200` — can generate real thumbnails, and the suite cannot remove them: the destination is rendered by the deployment's own `THUMBNAIL_PATH_TEMPLATE` from values the suite does not have, finding it would mean waiting on asynchronous generation, and `DisassociateThumbnail` removes metadata without deleting a blob. Point `--source-file` at a test object in a location where leftover artifacts are acceptable.
- **Against a non-conformant deployment, the bound case can make the source unreadable.** A deployment that accepts the over-bound probe may write one association per specification onto `--source-file`; past enough entries, storage refuses to return the metadata in one response and every later `GET /v1alpha/thumbnails` and `DELETE /v1alpha/associations` for that address fails with `502`. Against a conformant deployment the request is refused with `400` / `INVALID_ARGUMENT` and nothing is written. Pick a source you are willing to lose; a default `--test-suite=all` run exercises this case once per transport.

(thumbnail-service-conformance-and-smoke-tests-parameters)=
### Parameters

| CLI flag | Environment variable | Required | Description |
|---|---|---|---|
| `--rest-url` | `REST_URL` | Every suite, including `grpc` | HTTP base URL, for example `http://localhost:3000`. Also used for the OpenAPI preflight. |
| `--grpc-url` | `GRPC_URL` | For `grpc` and `all` | gRPC endpoint, for example `http://localhost:50051`. |
| `--grpc-tls-enabled` | `GRPC_TLS_ENABLED` | No | Enable TLS on the gRPC channel. Default `false`. |
| `--test-suite` | `TEST_SUITE` | No | `all` (default), `rest`, or `grpc`. |
| `--source-file` | `SOURCE_FILE` | Yes | Existing storage object the suite associates against and generates from. |
| `--ready-path` | `READY_PATH` | No | REST path polled by the readiness wait. Default `/health`. |
| `--http-timeout` | `HTTP_TIMEOUT` | No | Per-request timeout (humantime). Default `30s`. Bounds every REST request and the gRPC connect and per-call deadlines. |
| `--wait-retries` | `WAIT_RETRIES` | No | Readiness retry count. Default `10`. |
| `--wait-interval` | `WAIT_INTERVAL` | No | Interval between readiness retries (humantime). Default `5s`. |

(thumbnail-service-conformance-and-smoke-tests-authentication-modes)=
### Authentication modes

Provide a bearer token through exactly one of the following, or omit all three to run unauthenticated — valid only against a deployment whose storage does not enforce authorization:

1. **Direct bearer token** — `--bearer-token` / `BEARER_TOKEN`.
2. **Access-token file** — `--access-token-file` / `ACCESS_TOKEN_FILE`, a JSON file with an `access_token` field as written by a token-provisioner sidecar. Read once at startup.
3. **OAuth2 client credentials via OpenID Connect discovery** — `--oauth2-openid-configuration-uri`, `--oauth2-client-id`, `--oauth2-client-secret` (or `--oauth2-client-secret-file`), and `--oauth2-client-scope`. The token endpoint is resolved from the discovery document.

(thumbnail-service-conformance-and-smoke-tests-what-the-suite-covers)=
### What the suite covers

Every request-level rejection is asserted on **both** transports, because the two are supposed to agree about which requests are legal and a suite that tested different things on each side could not detect them diverging.

| Case | REST | gRPC |
|---|---|---|
| Associate a thumbnail, read it back, disassociate it | `204`, then listed by `GET /v1alpha/thumbnails`, then `200` | `AssociateThumbnail` success, `DisassociateThumbnail` `OK` |
| Disassociate again (idempotent) | `200` with both arrays empty | `OK` with both arrays empty — never `NOT_FOUND` |
| Associate with a blank `resource_address` | `400` | `INVALID_ARGUMENT` |
| Associate with no `specification` | `400` | `INVALID_ARGUMENT` |
| `CreateThumbnails`, valid specification with omitted quality | `202`, empty `failed` | `OK`, empty `failed` |
| `CreateThumbnails`, PNG quality `200` and GIF quality `200` | `202`, empty `failed` | `OK`, empty `failed` |
| `CreateThumbnails`, JPEG quality `200` and WebP quality `200` | `400` naming the specification and field | `INVALID_ARGUMENT` naming the specification and field |
| `CreateThumbnails`, `width` `0` | `400` | `INVALID_ARGUMENT` |
| `CreateThumbnails`, blank `resource_address` | `400` | `INVALID_ARGUMENT` |
| `CreateThumbnails`, one specification beyond the verified advertised bound | `400` | `INVALID_ARGUMENT` |
| Health | `GET /health`, `GET /health/live` return `200` with a non-empty status | standard `grpc.health.v1` readiness |

Readiness is checked on every selected transport before its cases — REST through `--ready-path`, gRPC through the standard health service — within `--wait-retries` and `--wait-interval`. In an `all` run, failed REST readiness prevents the REST cases but preserves gRPC coverage when the OpenAPI preflight and gRPC readiness succeed; the overall run still fails.

(thumbnail-service-conformance-and-smoke-tests-run-faults-versus-contract-failures)=
### Run faults versus contract failures

A misconfigured run and a non-conformant deployment produce the same failure count, so the summary adds a `[WARN]` block per cause naming the flag to change and the cases it accounts for. Classified faults, none of which certify conformance:

- **`--source-file` did not resolve** — a `404` that carries the API's `ErrorResponse` body, so the route exists and the object does not.
- **Credentials were rejected** — `401` or `UNAUTHENTICATED`.
- **Access was denied** — `403` or `PERMISSION_DENIED`; check caller permissions and token scopes for the API and the source, this does not mean the token is invalid.
- **The endpoint is not serving this API** — a `404` *without* the `ErrorResponse` body, or `UNIMPLEMENTED` over gRPC.
- **Client timeout** — an incomplete observation naming `--http-timeout`, distinct from a server-returned error.
- **Unverifiable OpenAPI contract** — names `/api-docs/openapi.json` and `--rest-url`.

Association cases whose prerequisite failed are reported as **skipped** with their reason. Skips are excluded from the pass count and prevent certification even when nothing else failed.

(thumbnail-service-conformance-and-smoke-tests-running-the-conformance-chart)=
## Running the conformance chart

The `thumbnail-conformance-test` chart renders a one-shot Job and a ServiceAccount. Install it on demand into the namespace that can reach the deployment under test:

```yaml
conformanceTest:
  restUrl: "http://thumbnail-service.storage-apis.svc.cluster.local:3000"
  grpcUrl: "http://thumbnail-service.storage-apis.svc.cluster.local:50051"
  testSuite: "all"
  sourceFile: "https://acct.blob.core.windows.net/assets/conformance/source.png"
  httpTimeout: "30s"
  waitRetries: 10
  waitInterval: "5s"

# Exactly one serviceIdentity mode, or none for an unauthenticated deployment
serviceIdentity:
  bearerToken:
    enabled: true
    secretRef:
      name: "thumbnail-conformance-token"
      key: "BEARER_TOKEN"
```

conformance-values.yaml

```bash
# pull the chart from NGC
helm fetch https://helm.ngc.nvidia.com/nvidia/omniverse/charts/thumbnail-conformance-test-{VERSION}.tgz --username='$oauthtoken' --password=${NGC_API_KEY}
tar -xvf thumbnail-conformance-test-{VERSION}.tgz

# render and run
helm template conformance ./thumbnail-conformance-test -f conformance-values.yaml
helm install conformance ./thumbnail-conformance-test -f conformance-values.yaml --namespace storage-apis

# read the verdict; the Job has backoffLimit 0 and is cleaned up after 600 seconds
kubectl logs -n storage-apis job/conformance-thumbnail-conformance-test
```

`restUrl` and `sourceFile` are required at render time; `grpcUrl` is required unless `testSuite` is `rest`. At most one authentication block may be set — a Secret-backed bearer token, client credentials, or an access-token file — and none is valid against a deployment that runs without storage authorization. The chart's post-install notes repeat the over-bound cost disclosure above.

(thumbnail-service-conformance-and-smoke-tests-running-the-conformance-test-directly)=
### Running the conformance test directly

```bash
thumbnail-conformance-test \
  --rest-url http://localhost:3000 \
  --grpc-url http://localhost:50051 \
  --source-file "https://acct.blob.core.windows.net/assets/conformance/source.png" \
  --bearer-token "$BEARER_TOKEN"
```

(thumbnail-service-conformance-and-smoke-tests-see-also)=
## See also

- {ref}`Thumbnail Service Configuration <thumbnail-service-deployment-thumbnail-service-configuration>` — installing the release the tests run against, and the manual end-to-end verification that does prove a thumbnail was generated.
- {ref}`Authenticating with an OpenID Connect Identity Provider <thumbnail-service-authentication-authenticating-with-an-openid-connect-identity-provider>` — the identity modes both tests share with the service.
- {ref}`Association metadata and queries <thumbnail-service-associations-association-metadata-and-queries>` — why an over-bound acceptance writes one association per specification onto the source.
- {ref}`Troubleshooting <thumbnail-service-troubleshooting-troubleshooting>` — interpreting `401`, `403`, and `404` outcomes.
