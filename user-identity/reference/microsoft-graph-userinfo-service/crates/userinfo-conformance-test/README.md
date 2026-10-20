# User Info Service Conformance Test

Validates a User Info Service implementation against the published REST and gRPC API contract, including the authentication and authorization model and the readiness behaviour. Designed to run against any service implementation that follows the API contract documented in [AGENTS.md](AGENTS.md).

The test only validates the **API contract** — paths, status codes, response shapes, filter semantics, pagination, and error mapping. It does not assert that any specific user, group, email, or display name exists in the directory; doing so would test the operator's directory configuration, not the service. Test fixtures are sampled from the live service itself.

## Usage

```shell
cargo run -p userinfo-conformance-test -- \
  --rest-url http://localhost:8080 \
  --grpc-url http://localhost:50051 \
  --bearer-token "$BEARER_TOKEN"
```

The directory pre-requisites are minimal but mandatory:

- at least one user
- at least one user with a non-empty `email` field within the first `--page-limit` users returned by the listing
- at least one group
- at least one group with at least one direct member within the first `--page-limit` groups returned by the listing

If any of these cannot be sampled, the test fails fast with a clear remediation hint. The conformance test never silently skips a case.

> The phase that selects test fixtures from the live service is called **fixture sampling** in this crate. It is deliberately not called "discovery" to avoid confusion with the unrelated "service discovery" concept commonly used in microservice infrastructure.

Membership sampling is **group-first**: the test lists groups, then for each group probes `/groups/{id}/users?limit=1` (or `ListGroupMembers` with `page_size=1`) to find one with at least one direct member. The first member's identifier and `display_name` are then used as the user side of all subsequent membership tests, including the user-side `/users/{id}/groups` lookup and a `display_name` prefix filter on the members listing.

## Parameters

| CLI flag | Env var | Required | Description |
|---|---|---|---|
| `--rest-url` | `REST_URL` | For REST/all suites | HTTP base URL (e.g. `http://localhost:8080`) |
| `--grpc-url` | `GRPC_URL` | For gRPC/all suites | gRPC endpoint (e.g. `http://localhost:50051`) |
| `--grpc-tls-enabled` | `GRPC_TLS_ENABLED` | No | Enable TLS on the gRPC channel (default: `false`) |
| `--test-suite` | `TEST_SUITE` | No | `all` (default), `rest`, or `grpc` |
| `--ready-path` | `READY_PATH` | No | REST path used by the readiness wait loop (default: `/readyz`) |
| `--page-limit` | `PAGE_LIMIT` | No | Page size used by the fixture-sampling probe (default: 20). Sampling scans up to this many users when looking for one with an `email`, and up to this many groups when looking for one with at least one direct member. Increase when probing a sparse directory. |
| `--http-timeout` | `HTTP_TIMEOUT` | No | Per-request timeout in seconds (default: 30) |
| `--wait-retries` | `WAIT_RETRIES` | No | Readiness probe retry count (default: 10) |
| `--wait-interval` | `WAIT_INTERVAL` | No | Seconds between readiness retries (default: 5) |

### Authentication

The modes mirror the `serviceIdentity` section of the Helm chart (which itself mirrors the permission-service chart), so the same deployment conventions apply. Provide a bearer token via exactly one of:

- **Direct token**: `--bearer-token` / `BEARER_TOKEN` — convenience for local development.
- **Access-token file**: `--access-token-file` / `ACCESS_TOKEN_FILE` — path to a JSON file with `access_token` and RFC3339 `expires_on` fields. Typically populated by a token-provisioner sidecar configured under `serviceIdentity.accessTokenFile.provisionerSidecar` in the chart. The file is read once at startup.
- **OAuth2 client-credentials with OpenID Connect Discovery**: `--oauth2-openid-configuration-uri`, `--oauth2-client-id`, `--oauth2-client-secret` (or `--oauth2-client-secret-file`), `--oauth2-client-scope`. The token endpoint is resolved dynamically from the identity provider's OpenID metadata document at the supplied URI (typically `.../.well-known/openid-configuration`); there is no separate `--oauth2-token-url` flag. Optional `--oauth2-extra-headers` and `--oauth2-extra-body-params` accept a JSON object of string values that are merged into the token exchange request.

If none are supplied the test runs unauthenticated (only valid when the target service has authentication disabled).

## Test coverage

Each suite first runs a **fixture sampling phase** that probes its own transport (REST suite → REST fixture sampling; gRPC suite → gRPC fixture sampling) for: a sample user, a user with an `email` field, a sample group, and a `(group, member)` pair (group-first sampling). The negative tests use generated identifiers that no directory entry can collide with.

### REST (`/api/v1alpha`)

- `GET /users/{id}` — returns the sampled user with the documented schema and matching `id`.
- `GET /users/{id}` — returns 404 / `not_found` for a generated unknown identifier.
- `GET /users?email=<email>` — exact-match filter contains the sampled email-bearing user, and every returned item has the requested email.
- `GET /users?display_name=<prefix>*` — prefix filter contains the sampled user, and every returned item has a `display_name` starting with the prefix.
- `GET /users?limit=1` — pagination cap is enforced and the page envelope is well-formed.
- `GET /users/{member}/groups` — direct memberships of the sampled member contain the sampled group.
- `GET /users/{member}/groups/{group}` — confirmed membership returns the group.
- `GET /users/{user}/groups/{unknown}` — 404 for a generated non-member group identifier.
- `GET /groups/{id}` — returns the sampled group with the documented schema.
- `GET /groups/{id}` — returns 404 for a generated unknown identifier.
- `GET /groups?display_name=<prefix>*` — prefix filter contains the sampled group, and every returned item has a `display_name` starting with the prefix.
- `GET /groups/{group}/users` — direct members of the sampled group contain the sampled member.
- `GET /groups/{group}/users?display_name=<member-display-name>*` — member-side prefix filter contains the sampled member, and every returned item has a `display_name` starting with the prefix.
- `GET /groups/{group}/users/{member}` — confirmed membership returns the user.
- `GET /groups/{group}/users/{unknown}` — 404 for a generated non-member user identifier.

### gRPC (`nvidia.omniverse.userinfo.v1alpha.UserInfoService`)

- `GetUser` — returns the sampled user; `NOT_FOUND` for a generated unknown identifier.
- `ListUsers` with `email` exact match — contains the sampled email-bearing user; every returned item satisfies the filter.
- `ListUsers` with `display_name` prefix — contains the sampled user; every returned item starts with the prefix.
- `ListUsers` with `page_size=1` — pagination cap is enforced.
- `ListUserGroups` — contains the sampled group for the sampled member.
- `GetUserGroup` — returns the group on confirmed membership; `NOT_FOUND` for a generated non-member group.
- `GetGroup` — returns the sampled group; `NOT_FOUND` for a generated unknown identifier.
- `ListGroups` with `display_name` prefix — contains the sampled group; every returned item starts with the prefix.
- `ListGroupMembers` — contains the sampled member.
- `ListGroupMembers` with `display_name` prefix built from the sampled member's `display_name` — contains the sampled member; every returned item starts with the prefix.
- `GetGroupMember` — returns the user on confirmed membership; `NOT_FOUND` for a generated non-member user.

Each test prints a structured remediation hint on failure that includes the request transcript, the response body (or gRPC status), and a link back to the relevant section of the documentation.

## Console output

The test prints a banner, a section separator for each phase (Authentication, Pre-flight, Fixture sampling per protocol, REST conformance, gRPC connection, gRPC conformance, Summary), and color-coded result markers (green `✔ PASS`, red `✘ FAIL`, cyan `ℹ INFO`). Color and Unicode box-drawing characters are emitted only when stdout is a terminal; pipe the test into a file or a CI log and the same output is rendered as plain text.

Override the autodetection with the standard environment variables:

- `NO_COLOR=1` — disable all styling (per <https://no-color.org/>).
- `FORCE_COLOR=1` — force styling on even when stdout is not a terminal (useful when the runner captures output but you still want ANSI codes for downstream rendering).
