# Discovery Conformance Runner

This optional, read-only utility checks an existing deployment. It does not
deploy services, mutate customer data, or certify unselected API surfaces.

## Run From Source

Install [uv](https://docs.astral.sh/uv/getting-started/installation/).
Python 3.11 or newer is required; uv can provision Python if needed.
Run from the repository root without manually creating or activating an environment:

```sh
uv sync --project conformance --locked
uv run --project conformance --locked discovery-conformance \
  --catalog-url https://discovery.example.com/api/v2beta/services \
  --expect-service fileobject \
  --json-report result.json --junit-report result.xml
```

Supply full operation URLs, including any ingress prefix. Each target is
explicit; the runner never follows advertised catalog URLs or redirects.

```sh
uv run --project conformance --locked discovery-conformance \
  --rest-capabilities-url https://backend.example.com/v1beta/services \
  --grpc-target backend.example.com:443 \
  --capabilities-token-file /run/secrets/backend-token \
  --ca-bundle /run/secrets/company-ca.pem
```

`--token-file` authenticates only the catalog. `--capabilities-token-file`
authenticates only the explicitly supplied backend targets. Bearer credentials
require TLS. Tokens are not accepted on the command line or printed in reports.
Ambient `.netrc` credentials are ignored; environment proxy routing is preserved.
For an unauthenticated local gRPC fixture, use `--plaintext-grpc` explicitly.

## Contract Profiles

- Catalog: `/api/v2beta/services`, schema-version 2. The `/api/v2/services`
  compatibility alias returns the same response and is validated against the
  same profile. The bundled OpenAPI document is
  a **draft transcription** of existing service documentation, not a newly
  approved normative specification. Optional REST/gRPC blocks may be absent;
  an empty service inventory is valid. Use `--expect-service` to test presence.
- REST Capabilities: the existing repository OpenAPI 3.0.3 v1beta contract.
  Its permissive schema is honored; missing optional fields are not failures.
- gRPC Capabilities: the pinned protobuf v2beta `ListServices` operation.
  Backends that implement only the deprecated v2alpha API fail with
  `UNIMPLEMENTED`.
  Generated clients require neither reflection nor protoc at runtime. Pagination
  is bounded by `--max-pages` and repeated tokens fail. Protobuf scalar defaults
  are legal; successful decoding alone does not validate every application API.

The REST v1beta and gRPC v2beta profiles are intentionally independent. No
equivalence between these versions is assumed. Reports include spec revisions
and hashes. Schema-only success does not prove expected inventory, freshness,
ingress visibility, authorization policy, or advertised application APIs work.

Statuses are `PASS`, `FAIL`, `SKIP`, and `INCONCLUSIVE`. Exit codes: `0` selected
checks passed, `1` contract violation, `2` configuration/report error, `3`
unavailable prerequisites or incomplete execution. No selected targets is a
configuration error, and a run with no checks cannot pass.

## Test And Package

```sh
uv run --project conformance --locked --extra test python -m pytest conformance/tests
uv build conformance
docker build -f conformance/Dockerfile -t discovery-conformance:dev .
docker run --rm discovery-conformance:dev \
  --catalog-url https://discovery.example.com/api/v2beta/services
```

For reports or credentials in a container, mount explicit directories and secret
files and supply their container paths; the process runs as UID 10001. Container
build is development packaging, not a signed/pinned
production release. Production publishing still requires normal security scans,
spec review and release automation. Dependencies are pinned by `uv.lock`;
run `uv lock --project conformance` when intentionally updating them.

The bundled REST Capabilities schema is an unchanged copy of
`openapi/capabilities_clean.openapi.yaml`; tests detect drift. Refresh this copy
and provenance when updating the contract. Regenerate protobuf clients with
`conformance/scripts/generate-protos.sh`; it runs through uv with the locked
`generate` extra. Override the uv executable with `UV` if needed.

Deployment-policy checks (authentication challenges, public auth-config,
internal/external visibility), REST/gRPC inventory parity, malformed-request
semantics, and automated backend enumeration remain follow-up profiles.
