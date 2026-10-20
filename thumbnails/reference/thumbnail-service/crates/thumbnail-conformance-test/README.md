# Thumbnail Service Conformance Test

A standalone, black-box test that validates a deployed `thumbnail-service` implementation against the
published REST and gRPC API contract — the wire paths, status codes, response shapes, and documented
error mapping described in [AGENTS.md](AGENTS.md). It is separate from
[`thumbnail-smoke-test`](../thumbnail-smoke-test/README.md): the smoke test proves a deployment came up
after install/upgrade and stops at the first failure; this suite runs every case and exists to certify
API-contract conformance, independent of any particular release.

This image and its Helm chart (`helm/thumbnail-conformance-test/`) are published to NGC so customers can
run it standalone against their own deployment.

The suite only validates the **API contract**. It does not assert that any requested thumbnail was
actually generated — `CreateThumbnails` triggers asynchronous work this suite deliberately does not wait
on. See AGENTS.md for the full behavioral contract and what is explicitly out of scope.

## Usage

```shell
cargo run -p thumbnail-conformance-test -- \
  --rest-url http://localhost:3000 \
  --grpc-url http://localhost:50051 \
  --source-file "https://storage.example.test/conformance/source.png" \
  --bearer-token "$BEARER_TOKEN"
```

`--source-file` is required, and it **must be an object that already exists in storage**. The
association cases only write and read metadata keyed on an address, so they need no data seeded —
each creates and tears down its own fixture against this one, and a deployment with no existing
associations behaves the same as a long-lived one. `CreateThumbnails` is the exception: it resolves
the source's identity with a storage `Stat`, so its cases fail with `404` / `NOT_FOUND` unless the
address resolves. When they do, the run's summary says so and names the flag, rather than reporting a
bare failure count that reads the same as a non-conformant deployment. See AGENTS.md's "Fixture
sampling" section.

### The run leaves generated thumbnails behind

**Three cases per transport can generate real thumbnails, and the suite cannot remove them** —
the valid-specification, PNG quality 200, and GIF quality 200 acceptance cases.
Point `--source-file` at an object in a location where that is acceptable — a test container or a
prefix someone cleans up on a schedule — not at production content.

They are unremovable through this API rather than merely not-yet-cleaned-up: the destination address
is rendered by the deployment's own path template from values a black-box client does not have,
finding it afterwards would mean waiting on asynchronous generation and then guessing which
associations this run caused, and `DisassociateThumbnail` removes association metadata without
deleting a blob. AGENTS.md's "Fixture sampling" section has the full reasoning. The cases are not
skippable either — they are the only ones that exercise generation, and a run that certifies a
deployment while quietly omitting them is worse than one that leaves artefacts.

### On a non-conformant deployment, the bound case can make the source unreadable

**Against a conformant deployment this costs nothing** — the specification-count case is refused with
`400` / `INVALID_ARGUMENT` and writes nothing at all. The paragraph below is about what happens when
the deployment you are certifying turns out not to enforce the bound.

Preflight reads `specifications.maxItems` from `/api-docs/openapi.json` and compares it to the
independent documented bound of 1000. Only a match allows an `advertised + 1` enforcement probe.
A deployment accepting that request may generate that many thumbnails and write **one association per specification onto `--source-file`**. The specifications are identical,
so they collapse to a single destination address and the leftover blobs are not the problem — the
source's association metadata is. Past enough entries it exceeds what storage returns in one
response, and from then on `GET /v1alpha/thumbnails` and `DELETE /v1alpha/associations` for that
address fail. Observed while developing this case: storage answers `InvalidArgument: response is too
large`, which the service reports as `502`, and every read and disassociate case against that source
failed on subsequent runs until the object was abandoned.

So when you pick `--source-file`, pick one you are willing to lose. A dedicated test object is
right; a real asset whose thumbnails matter is not. The suite reports this in the failing case as
well, because an accepting response may already have started the work.

Both transports run this case, so a default `--test-suite=all` run does the above **twice** — once
per transport — against the same source.

## Parameters

| CLI flag | Env var | Required | Description |
|---|---|---|---|
| `--rest-url` | `REST_URL` | Every suite, including `grpc` | HTTP base URL for OpenAPI preflight, e.g. `http://localhost:3000` |
| `--grpc-url` | `GRPC_URL` | For `grpc`/`all` suites | gRPC endpoint, e.g. `http://localhost:50051` |
| `--grpc-tls-enabled` | `GRPC_TLS_ENABLED` | No | Enable TLS on the gRPC channel (default: `false`) |
| `--test-suite` | `TEST_SUITE` | No | `all` (default), `rest`, or `grpc` |
| `--source-file` | `SOURCE_FILE` | Yes | Source file the suite associates/disassociates its own fixtures against. **Must already exist in storage**, and the run generates thumbnails from it that it cannot remove — see below |
| `--ready-path` | `READY_PATH` | No | REST path used by the readiness wait loop (default: `/health`) |
| `--http-timeout` | `HTTP_TIMEOUT` | No | Per-request timeout, humantime duration (default: `30s`). Bounds **both** transports: every REST request, and the gRPC channel's connect and per-call deadlines |
| `--wait-retries` | `WAIT_RETRIES` | No | Readiness probe retry count (default: `10`) |
| `--wait-interval` | `WAIT_INTERVAL` | No | Interval between readiness retries, humantime duration (default: `5s`) |

### Authentication

Provide a bearer token via exactly one of the following, or omit all three for an unauthenticated run
(only valid against a deployment with authentication disabled):

- **Direct token**: `--bearer-token` / `BEARER_TOKEN`.
- **Access-token file**: `--access-token-file` / `ACCESS_TOKEN_FILE` — path to a JSON file with an
  `access_token` field. Typically populated by a token-provisioner sidecar configured under
  `serviceIdentity.accessTokenFile.provisionerSidecar` in the Helm chart.
- **OAuth2 client-credentials with OpenID Connect Discovery**: `--oauth2-openid-configuration-uri`,
  `--oauth2-client-id`, `--oauth2-client-secret` (or `--oauth2-client-secret-file`),
  `--oauth2-client-scope`. The token endpoint is resolved dynamically from the discovery document.

## Test coverage

Every request-level rejection is asserted on **both** transports, because the two are supposed to
agree about which requests are legal and a suite testing different things on each side cannot detect
them diverging. AGENTS.md's "Rejection parity between the transports" table is the list.

### REST (`/v1alpha`)

- `POST /v1alpha/associations` — 204 on success; 400 on a blank resource address, and 400 on an
  omitted `specification`.
- `GET /v1alpha/thumbnails` — contains the association after associate; no longer contains it after
  disassociate.
- `DELETE /v1alpha/associations` — 200 on success; 200 with empty arrays on an idempotent re-delete
  (never a not-found error).
- `POST /v1alpha/thumbnails` (`CreateThumbnails`) — 202 with empty `failed` for a valid
  specification, PNG quality 200, and GIF quality 200. Request-level 400 for JPEG/WebP quality 200,
  width 0, blank `resource_address`, and one specification beyond the verified advertised bound.
  Quality and width errors name the specification and field. Uses outer-keyed
  `{"image_to_image": {...}}` specifications and never polls for generation completion.
- `GET /health`, `GET /health/live`.

### gRPC (`nvidia.omniverse.thumbnail.v1alpha.ThumbnailService`)

- `AssociateThumbnail` — success; `INVALID_ARGUMENT` for a missing specification, and for a blank
  resource address.
- `DisassociateThumbnail` — success; idempotent `OK` with empty arrays on re-delete.
- `CreateThumbnails` — OK with empty `failed` for the same three acceptance cases;
  INVALID_ARGUMENT for the same five rejections. Quality and dimension errors name the
  specification and field. No generation-completion assertion.

The valid-specification acceptance case omits quality on both transports to exercise configured
defaults; the PNG/GIF quality cases supply 200 explicitly.

Before cases run, preflight requires a usable OpenAPI route, including for gRPC-only invocations.
Missing/malformed/inaccessible schemas produce an unverifiable nonzero run; advertised bounds above
or below 1000 fail naming both numbers without sending a probe. This detects declared drift, not a
smaller secretly enforced bound. Nothing tests at the bound: that could enqueue 1000 conversions.
HTTP failures on the OpenAPI route retain credential, access, endpoint, or server-fault diagnostics
alongside the unverifiable-check message; malformed successful responses remain schema failures.

Genuine partial/total publisher failures, defaults fan-out, and failed source Stat mappings are
covered by service tests with injected failures. Invalid quality no longer induces those envelopes.
The public `failed` field remains: valid requests can still encounter dispatch failures.

### Not covered

- **Whether an accepted specification is ever generated.** Out of scope by design; see AGENTS.md.

## Console output

Prints a banner, then a `[PASS]`/`[FAIL]` line per case (tagged `[REST]` or `[gRPC]`) as it runs, and a
final summary with total pass/fail counts and per-case timing. Unlike `thumbnail-smoke-test`, this suite
does not stop at the first failure — every independent case runs so a single invocation reports the full
extent of any non-conformance. The process exits `0` when every case passes and `1` if any case fails or is skipped.

A run always prints a summary, including when the gRPC endpoint cannot be connected to at all — that
is recorded as a failed case rather than aborting the process, so the REST results already gathered
survive. Both the connect and every gRPC call are bounded by `--http-timeout`, because a deployment
that accepts a connection and never answers would otherwise hang the run and produce no
certification at all.

When failures are the invocation's fault rather than the deployment's, the summary adds a `[WARN]`
block per cause, naming the flag to change and listing the cases it accounts for. The following are classified:

- **`--source-file` did not resolve in storage** — a `404` that still carries this API's
  `ErrorResponse`, so the route exists and the object does not.
- **Credentials were rejected** — `401` or `UNAUTHENTICATED` over gRPC. Check for a missing,
  expired, or refused token.
- **Access was denied** — `403` or `PERMISSION_DENIED` over gRPC. Check caller permissions and
  token scopes for the API and source; this does not establish that the credentials are invalid.
- **The endpoint is not serving this API** — a `404` *without* that body, or `UNIMPLEMENTED` over
  gRPC. Distinguished by the documented body shape rather than by message text, because otherwise a
  wrong `--rest-url` gets reported as a missing object and sends you to check storage.

Those cases still fail and the run still exits `1`. The classification says where to look, which a
bare count cannot: a misconfigured run and a non-conformant deployment produce the same number.

- **Client timeout** — an incomplete observation naming `--http-timeout`, distinct from a
  server-returned cancellation or error.
- **Unverifiable OpenAPI contract** — names `/api-docs/openapi.json` and `--rest-url`.

REST readiness and the standard gRPC health service are checked for each selected transport before
its cases. Exhausted readiness reports an incomplete run. In `all` mode, failed REST readiness
prevents REST cases but gRPC cases still run if OpenAPI preflight and gRPC readiness succeed.
The overall run still exits nonzero. A dependent association case whose
prerequisite failed is reported as skipped with its reason, never counted passed. Any selected skip
prevents a successful run even without an accompanying failure.
