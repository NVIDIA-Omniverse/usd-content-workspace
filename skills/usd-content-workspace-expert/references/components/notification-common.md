# notification-common — component map

> **Applies to:** `notification-common` Python library · **Released:** not in released inventory (a library, not an NGC artifact). Released Aggregation 1.6.0 and Consumer 1.11.0 were built against 6.12.0 · **This repo's snapshot:** 7.0.0 (`notifications/reference/common/pyproject.toml`, `CHANGELOG.md` top entry, `SOURCE_VERSIONS.md`; release commit "Release notification-common 7.0.0"); the unreleased Aggregation 1.7.0 and Consumer 1.12.0 build against it · **Sources:** [`notifications/reference/common/`](../../../../notifications/reference/common/README.md)

## What it is

The shared library (Python ≥ 3.12, Apache-2.0) that both notification services compose into a process. It owns:

| Area | Module | What to know |
|---|---|---|
| Settings | `notification_common/config/` | `pydantic-settings`, every variable prefixed `OMNI_EVENTS_`. `CommonSettings` (ports, pools, shutdown, routing, REST CORS origins), `AuthSettings` (principal source, OIDC, unverified-JWT opt-in) |
| Authentication | `auth/`, `interceptors/auth_interceptor.py`, `rest/middleware/auth_token_middleware.py` | `principal_source` modes `disabled`, `jwtUnverified` (still the field default; no signature, audience, or expiry check; since 7.0.0 refused at startup unless `allow_unverified_jwt` / `OMNI_EVENTS_ALLOW_UNVERIFIED_JWT=true`), `jwt`, `opaque`, `userInfoService` (`GET {uri}/api/v1alpha/users/me`) |
| REST CORS | `rest/notification_rest_server.py`, `config/common_settings.py` | Since 7.0.0, `cors_allow_origins` (`OMNI_EVENTS_CORS_ALLOW_ORIGINS`, comma-separated or JSON array, default empty). Empty adds no CORS middleware; listed origins are echoed with credentials. `*`, `null`, and origins not written as browsers send `Origin` are refused when settings load. Before 7.0.0 every origin was allowed with credentials |
| Authorization | `permissions/` | Calls the Permission Service REST path `/v1beta/authorization/` (and `/v1beta/authorization/batch/`) with action `{service: <service pyproject name>, name: <action>}` and the caller's token; results cached for `OMNI_EVENTS_PERMISSIONS_TTL_SECONDS`. Gate: `OMNI_EVENTS_PERMISSIONS_ENABLED` only (renamed from `OMNI_EVENTS_AUTHORIZATION_ENABLED` in 6.0.0) |
| RabbitMQ | `rabbitmq/` | Declares the durable topic exchange `events`; channel pool with overflow band; routing-key encoding (hashed segments; `routing_max_event_type_parts` default 5, `routing_max_prefix_depth` default 37) |
| Servers | `grpc/notification_server.py`, `rest/notification_rest_server.py`, `servers/` | Shared startup, gRPC health and reflection, optional Discovery `CapabilitiesService` (`OMNI_EVENTS_GRPC_CAPABILITIES_ENABLED`, default true, unauthenticated), signal-owned shutdown (settle, drain, stop both servers, close clients) |
| Telemetry | `observability/`, `interceptors/metrics/` | OTLP export of traces, metrics, logs |
| Autoscaler | `scaler/` | The channel-pool autoscaler used by the consumer chart |
| Scripts | `scripts/` | Entry points the services wire into their own `[project.scripts]` (`start`, `lint`, `test`, `generate_openapi_yaml`, `generate_attributions`); the library's own commands are `nc-test`, `nc-lint`, `nc-update-proto` (renamed from `update_proto` in 6.12.8) |

Settings validation rejects `permissions_enabled=true` with `principal_source=disabled`, rejects `jwtUnverified` without `allow_unverified_jwt` (7.0.0), and requires `openid_configuration_uri` + `jwt_audiences` for `jwt`, `openid_configuration_uri` for `opaque`, `user_info_service_uri` for `userInfoService`. Because the module-level settings are built on import, a service with neither a principal source nor the opt-in fails at startup.

## Route by task

| Question | Go to |
|---|---|
| Auth and permission settings, capabilities hook, scripts | [`README.md`](../../../../notifications/reference/common/README.md) |
| Building and installing; pre-generated Discovery stubs | [`BUILDING.md`](../../../../notifications/reference/common/BUILDING.md) |
| Behavior changes per release (7.0.0 auth and CORS defaults, PyJWT effects, shutdown, auth ordering) | [`CHANGELOG.md`](../../../../notifications/reference/common/CHANGELOG.md) |
| Local OTEL/Prometheus compose stack for development | [`tools/docker-compose/`](../../../../notifications/reference/common/tools/docker-compose/) |
| How a service uses it | [aggregation](event-aggregation-service.md), [consumer](event-consumer-service.md) |

## Version pinning in this snapshot

Both services declare `notification-common==7.0.0` in `pyproject.toml` and resolve it from `[tool.uv.sources] notification-common = { path = "../common" }`. Each service's published `uv.lock` records it as `source = { directory = "../common" }` at 7.0.0, which matches `../common` in this tree, so `uv sync --locked` installs the locked set. uv ignores the version pin for a path source: if `../common` holds another version, `uv lock --check` reports the lock out of date and `uv sync` re-locks (each service's `BUILDING.md`). The Docker builds run `uv export --locked`, so a stale lock fails the image build.

7.0.0 is a major release that changes what an unconfigured deployment does: `jwtUnverified` needs the explicit opt-in, and REST CORS answers only configured origins (both rows above). Consuming services and their deployments must act before moving to it; see [upgrades](../operate/upgrades.md#notifications). Earlier: 6.12.8 and 6.12.9 changed no runtime code; 6.12.7 raised `pyjwt[crypto]` to `>=2.15.1`, which changes `jwt` mode: a redirecting `jwks_uri` now fails every request, and a just-rotated signing key can return 401 for up to 30 seconds. Confirm your IdP's `jwks_uri` answers 200 without a redirect.

## Known defects

- `README.md` links `docs/sdd.md` for the shutdown design; it is not in this repository.
