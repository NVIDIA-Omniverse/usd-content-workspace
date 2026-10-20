# notification-common

Shared Python library (>= 3.12) for NVIDIA Omniverse notification services. It
provides the reusable infrastructure those services compose into a running
process:

- gRPC and REST (FastAPI) server scaffolding with a shared startup/shutdown flow
- RabbitMQ connection, channel pooling, and publishing helpers
- Authentication and permission middleware for both gRPC and REST
- A shared aiohttp HTTP client and Redis client
- OpenTelemetry-based tracing, metrics, and logging

On `SIGTERM`/`SIGINT` the library owns the whole shutdown sequence for both
servers: it holds for an optional endpoint-settle delay
(`OMNI_EVENTS_SHUTDOWN_SETTLE_DELAY_SECONDS`, default `0`) during which the
process keeps serving normally, then drains the non-durable channel pool, then
stops the gRPC and REST servers together. Deployments must size
`terminationGracePeriodSeconds >= settle delay + drain window + REST teardown
tail`. The design, settings, and rationale are documented in
[`docs/sdd.md`](docs/sdd.md) under the Graceful-drain lifecycle and Signal
ownership entries.

To build and install the library from this source tree, see
[`BUILDING.md`](BUILDING.md).

## Authentication and Permissions Configuration

The library exposes its auth model through `pydantic-settings`. All environment
variables are prefixed with `OMNI_EVENTS_`. Authentication is configured on
`AuthSettings` (`auth_settings`); permission checks are configured on
`CommonSettings` (`common_settings`).

### `principal_source`

`principal_source` (env `OMNI_EVENTS_PRINCIPAL_SOURCE`) is the single
first-class setting that selects how a caller's identity is derived and whether
a token is required. `auth_settings.authentication_enabled()` returns `true`
for every value except `disabled`.

| Value | Behavior |
|-------|----------|
| `disabled` | Authentication is off. No principal is derived and no token is required. |
| `jwtUnverified` | The bearer JWT's payload is decoded to extract the principal id, but the signature, issuer, audience, and expiry are **not** verified. Token validity is delegated to a trusted upstream component (e.g. an Envoy auth extension) that has already verified the token. An explicit opt-in: it is refused unless `allow_unverified_jwt` is also set (see below). Backed by `auth/auth_tokens/jwt_unverified_decoder.py`. |
| `jwt` | The bearer JWT is verified locally using OpenID discovery. Requires `openid_configuration_uri` and `jwt_audiences`. |
| `opaque` | The bearer token is validated against the OIDC provider's introspection/userinfo path. Requires `openid_configuration_uri`. |
| `userInfoService` | The bearer token is resolved to a principal id by calling a deployed user-info service over REST. Requires `user_info_service_uri`. Backed by `auth/auth_tokens/user_info_service_decoder.py`. |

The field default is `jwtUnverified`, but that mode is refused unless
`allow_unverified_jwt` is set, so a deployment that sets neither setting fails
at startup instead of accepting unverified tokens. Choose a mode explicitly:
`jwt` when the service verifies tokens itself, or `jwtUnverified` with
`allow_unverified_jwt` only behind a proxy that has already verified every
token. For a local, tokenless run, set `principal_source` to `disabled`.

### `allow_unverified_jwt`

`allow_unverified_jwt` (env `OMNI_EVENTS_ALLOW_UNVERIFIED_JWT`, default
`false`) is the explicit opt-in for `principal_source=jwtUnverified`. Set it to
`true` only when every request reaches the service through a proxy that has
already verified the bearer token's signature, issuer, audience, and expiry,
for example an Envoy auth extension in front of the service. Any path that
skips that proxy breaks the condition: in Kubernetes, an ingress-level auth
extension does not cover callers inside the cluster that connect to the
Service directly, so the opt-in also needs those connections blocked, for
example by a NetworkPolicy that admits only the proxy. Without that, any
caller that can reach the service can present a self-made token for any
identity. Where those conditions do not hold, use `principal_source=jwt` with
`openid_configuration_uri` and `jwt_audiences`. The flag has no effect in any
other mode.

> **Breaking change (7.0.0):** a deployment that relied on the default
> `principal_source` (or set `jwtUnverified`) without the new flag now fails at
> startup with a `ServerError`. Set `OMNI_EVENTS_ALLOW_UNVERIFIED_JWT=true` if
> an upstream proxy verifies tokens, or switch to `jwt`.

### Decoders backing these modes

- **No-verify JWT decoder** (`auth/auth_tokens/jwt_unverified_decoder.py`) backs
  the `jwtUnverified` mode. It decodes the JWT payload to read the principal-id
  claim (`principal_id_claim`, default `sub`) and the `exp` claim, performing no
  signature, audience, or expiry verification. It must only be used where a
  trusted upstream component has already validated the token, which is why the
  mode needs the `allow_unverified_jwt` opt-in.
- **REST user-info-service client**
  (`auth/auth_tokens/user_info_service_decoder.py`) backs the `userInfoService`
  mode. It forwards the caller's bearer token to
  `GET {user_info_service_uri}/api/v1alpha/users/me`, returns the response `id`
  field as the principal id, and derives a short-lived expiry. It retries on
  transient faults (5xx, 429, transport errors), bounds the response body at
  64 KiB, and scrubs errors so token material, URLs, and response bodies are
  never logged. A `401`/`403` surfaces as an authentication failure; other
  non-2xx responses, an unreachable backend, or a malformed/oversized body
  surface as a server error.

### `user_info_service_uri`

`user_info_service_uri` (env `OMNI_EVENTS_USER_INFO_SERVICE_URI`) is the base
URI of the user-info service. It is consulted only when `principal_source` is
`userInfoService`.

### `permissions_enabled`

`permissions_enabled` (env `OMNI_EVENTS_PERMISSIONS_ENABLED`, default `false`)
gates authorization checks. Permission checks run if and only if this flag is
set; behavior is keyed solely on the explicit flag, not on whether a permissions
endpoint value is configured.

> **Breaking rename (6.0.0):** `permissions_enabled` was renamed from
> `authorization_enabled`, and its environment variable changed from
> `OMNI_EVENTS_AUTHORIZATION_ENABLED` to `OMNI_EVENTS_PERMISSIONS_ENABLED`.
> Update any deployments that set the old variable; the old name is no longer
> read.

### Validation rules

`AuthSettings.validate_token_settings` enforces these rules at settings load.
The module-level `auth_settings` singleton is built when
`notification_common.config.auth_settings` is first imported, so a violation
raises `ServerError` at import, which stops the service before it serves
anything:

- `permissions_enabled=true` with `principal_source=disabled` is rejected —
  permissions require authentication.
- `principal_source=jwt` requires both `openid_configuration_uri` and
  `jwt_audiences`.
- `principal_source=opaque` requires `openid_configuration_uri`.
- `principal_source=userInfoService` requires `user_info_service_uri`.
- `principal_source=jwtUnverified` (the field default) requires
  `allow_unverified_jwt=true`.
- `disabled` imposes no additional requirements.

`openid_enabled` is an infra-only field and `token_verification_type` is the
OIDC datum distinguishing the `jwt` vs `opaque` decoders; neither independently
gates behavior.

## REST CORS

`NotificationRestServer.create_app` adds Starlette's `CORSMiddleware` only for
an explicit list of browser origins. `cors_allow_origins` (env
`OMNI_EVENTS_CORS_ALLOW_ORIGINS`, on `CommonSettings`) takes a comma-separated
list or a JSON array, for example
`OMNI_EVENTS_CORS_ALLOW_ORIGINS=https://app.example.com,https://admin.example.com`.

- Empty (the default): no CORS middleware is added, so browsers receive no
  `Access-Control-Allow-Origin` header and do not let cross-origin pages read
  the responses.
- A listed origin is echoed back in `Access-Control-Allow-Origin` with
  `Access-Control-Allow-Credentials: true`; any other origin is not.
- Each entry must be written exactly as browsers send the `Origin` header,
  because the middleware compares the two as exact strings: `http` or
  `https`, an ASCII host (give an internationalized domain in punycode, and
  IP addresses in canonical form), and a port only when it is not the
  scheme's default (`:80` for `http`, `:443` for `https`), with no user info,
  wildcard, trailing dot, path, or trailing slash. Entries are trimmed and
  lower-cased, and anything else raises `ServerError` when the settings load.
- `*` and `null` are rejected with `ServerError`: combined with credentials, a
  wildcard would let any web page make credentialed calls, and `null` is the
  origin that sandboxed iframes and `file://` pages send.

`create_app` also accepts a `cors_allow_origins` argument, which overrides the
setting and is checked by the same rules at startup, and `cors_enabled=False`
still turns CORS off whatever origins are configured.

> **Breaking change (7.0.0):** CORS used to answer every origin
> (`allow_origins=["*"]` with credentials) whenever `cors_enabled` was true.
> Deployments whose browser clients call the service's REST API directly
> must now list those origins in `OMNI_EVENTS_CORS_ALLOW_ORIGINS`.

## Discovery CapabilitiesService

`NotificationServer` can serve the v2 Discovery `CapabilitiesService`
(`nvidia.omniverse.discovery.capabilities.v2alpha.CapabilitiesService`) so the
Discovery Kubernetes operator can enumerate your service's APIs. The operator
calls `ListServices`, and the server returns a single, statically-configured
`ServiceEntry` describing your gRPC and REST APIs.

The capabilities service is **opt-in**. A subclass turns it on by overriding
`capabilities_service_entry()` to return a populated `ServiceEntry`; the default
returns `None`, which keeps the service off.

The ergonomic way to build the entry is the `build_capabilities_entry(...)`
convenience classmethod. It auto-derives the `grpc_services` portion of the
`ServiceEntry` from the server's own exposed gRPC service name(s) —
`cls.service_name()`, plus the capabilities service itself when
`grpc_capabilities_enabled` is on — so the override supplies only the discovery
type key (`service_name`) and its REST resources/versions, rather than
hand-listing every gRPC service name:

```python
from nvidia.omniverse.discovery.capabilities.v2alpha import capabilities_pb2

class MyServer(NotificationServer):
    @classmethod
    def capabilities_service_entry(cls) -> capabilities_pb2.ServiceEntry | None:
        return cls.build_capabilities_entry(
            service_name="notification-consumer",  # the discovery type key
            rest_services=[
                # ... describe this service's REST resources/versions ...
            ],
        )
```

`build_capabilities_entry(...)` is optional and additive. Hand-authoring a full
`ServiceEntry` still works, and remains the escape hatch when you need to
advertise gRPC service names beyond this server's own:

```python
class MyServer(NotificationServer):
    @classmethod
    def capabilities_service_entry(cls) -> capabilities_pb2.ServiceEntry | None:
        return capabilities_pb2.ServiceEntry(
            # ... describe this service's gRPC and REST APIs by hand ...
        )
```

When the hook returns an entry and `grpc_capabilities_enabled` is set,
`NotificationServer` registers the `ListServices` RPC, adds the service to
server reflection, and marks it SERVING in the health service. The `ListServices`
RPC is **unauthenticated by design** — so the discovery scan can reach it without
credentials — and, like the health and reflection endpoints, is exempted from
metrics and tracing instrumentation.

### `grpc_capabilities_enabled`

`grpc_capabilities_enabled` (env `OMNI_EVENTS_GRPC_CAPABILITIES_ENABLED`, default
`true`) is a `CommonSettings` (`common_settings`) flag, alongside
`grpc_health_checking_enabled` and `grpc_reflection_enabled`. The capabilities
service is served only when this flag is on **and** `capabilities_service_entry()`
returns a non-`None` entry; setting the flag to `false` disables the service even
for a subclass that overrides the hook.

## Scripts for Projects Using notification-common

The notification-common library provides several reusable scripts that projects can use via `[project.scripts]`.

### Available Scripts

#### `start`
Runs the FastAPI app in uvicorn server for development purposes.

#### `lint`
Runs the linter (black and flake8) on the project.

#### `test`
Runs the test suite with coverage reporting.

#### `generate_openapi_yaml`
Generates an OpenAPI YAML specification from a FastAPI application.

**Usage in your project's `pyproject.toml`:**
```toml
[project.scripts]
generate_openapi_yaml = "notification_common.scripts.app_scripts:generate_openapi_yaml"
```

**Command-line options:**
- `-m, --module`: Python module path to import the FastAPI app from (default: `app.rest.rest_server`)
- `-a, --app`: Name of the FastAPI app variable in the module (default: `rest_server`)
- `-o, --output`: Output file path for the generated OpenAPI YAML (default: `openapi.yaml`)
- `-v, --verbose`: Enable verbose logging

**Environment variables:**
- `FASTAPI_MODULE`: Override the default module path
- `FASTAPI_APP_VAR`: Override the default app variable name

**Examples:**
```bash
# Generate with defaults (looks for app.rest.rest_server:rest_server)
uv run generate_openapi_yaml

# Generate with custom module and app variable
uv run generate_openapi_yaml -m my_app.server -a api_app -o docs/openapi.yaml

# Generate with verbose logging
uv run generate_openapi_yaml -v

# Use environment variables
export FASTAPI_MODULE=my_app.server
export FASTAPI_APP_VAR=api_app
uv run generate_openapi_yaml
```

## Project Structure

- `notification_common/`: Main package directory
- `tests/`: Test files
- `tools/`: Local development tools

## License

This project is licensed under the Apache License 2.0. See the [`LICENSE`](LICENSE)
file for the full text.

## Contributing

This project is currently not accepting contributions.