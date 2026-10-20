# Changelog

All notable changes to `notification-common` are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [7.0.2]

A PATCH release with packaging changes only. No library code, public API, or behavior changes.

### Packaging

- **Published files name the public repository.** The changelog named the private staging repository
  the publish job pushes to; it now names `NVIDIA-Omniverse/usd-content-workspace`, the public
  repository tagged releases are mirrored to. The publish leakage guard now also fails on the private
  repository's name. The `github-repo:` publish target is unchanged.

## [7.0.1]

A PATCH release that adds the Apache-2.0 SPDX header to `pyproject.toml`. No
library code, public API, or behavior changes.

### Packaging

- **Apache-2.0 SPDX header on `pyproject.toml` (OSRB 6875374).** The OSRB scan of the GitHub mirror counts
  every published file without an SPDX header; `pyproject.toml` was the last one. No library code changes.

## [7.0.0]

A MAJOR release that closes two insecure defaults (OMPE-105840, SAST-001 and
SAST-002). Both change what an unconfigured deployment does, so consuming
services and their deployments must act before moving to this version.

### Breaking

- **`jwtUnverified` needs an explicit opt-in.** `AuthSettings` gains
  `allow_unverified_jwt` (env `OMNI_EVENTS_ALLOW_UNVERIFIED_JWT`, default
  `false`). `validate_token_settings` raises `ServerError` when
  `principal_source` is `jwtUnverified` (still the field default) and the flag
  is not set, so a deployment that configures neither now fails at startup
  (the module-level `auth_settings` is built on import) instead of accepting a
  bearer token whose signature, issuer, audience, and expiry nobody checks.
  Migration: set `OMNI_EVENTS_ALLOW_UNVERIFIED_JWT=true` only where every path
  to the service passes a proxy that already verifies the token (for example
  an Envoy auth extension), including in-cluster callers, which an
  ingress-level extension does not cover unless direct connections to the
  Service are blocked; otherwise set `OMNI_EVENTS_PRINCIPAL_SOURCE=jwt` with
  `OMNI_EVENTS_OPENID_CONFIGURATION_URI` and `OMNI_EVENTS_JWT_AUDIENCES`, or
  `disabled` for a local tokenless run. The `jwtUnverified` decoder itself, the
  verifying `jwt` decoder, and the decoder dispatch are unchanged.
- **REST CORS answers only configured origins.** `CommonSettings` gains
  `cors_allow_origins` (env `OMNI_EVENTS_CORS_ALLOW_ORIGINS`, comma-separated
  or a JSON array, default empty), and `NotificationRestServer.create_app`
  gains a matching `cors_allow_origins` argument that overrides it.
  `create_app` used to add `CORSMiddleware` with `allow_origins=["*"]` and
  `allow_credentials=True` whenever `cors_enabled` was true (the default). It
  now adds the middleware only for a non-empty origin list and echoes only
  those origins (still with credentials). Entries are trimmed and lower-cased,
  and `ServerError` is raised when the settings load, or when `create_app`
  gets the list as an argument, if an entry is `*`, is `null` (the origin of
  sandboxed iframes and `file://` pages), or is not written the way browsers
  send the `Origin` header (for example a trailing slash, a default port, a
  wildcard host, or a non-punycode domain). Migration:
  a deployment whose browser clients call the REST API directly must list
  their origins in `OMNI_EVENTS_CORS_ALLOW_ORIGINS`.

### Changed

- **`generate_openapi_yaml` selects `principal_source=jwt` when it is unset**,
  next to the OpenID placeholders it already supplies, so generating a
  consumer's OpenAPI document still imports the app under the new validation.
- **`test_tools.mock_auth_settings` sets the `allow_unverified_jwt` opt-in**
  when called with `principal_source=PrincipalSource.JWT_UNVERIFIED`, and
  clears it for every other source, so consumers' tests can still select that
  mode.

### Packaging

- The package metadata names `NVIDIA` as the author instead of an individual
  (OMPE-105840).
- Earlier changelog entries no longer name the retired GitHub publish targets.

## [6.12.9]

A PATCH release that retargets the source-available GitHub publish. No library
code, public API, or behavior changes.

### Packaging

- **GitHub publish retargeted at
  `NVIDIA-Omniverse/usd-content-workspace`.** The previous publish
  target repository was transferred and renamed. Git follows
  GitHub's rename redirect, so a release branch still reaches the moved
  repository, but the `github-publish` component opens the release pull request
  with `curl` without `--location`: its lookup for an existing pull request reads
  the redirect as "none", and the create call fails, so the job exits after
  pushing. The `github-repo:` input of
  `github:publish` now names the new repository, the same target the event
  services already publish to.

## [6.12.8]

A PATCH release that puts the Apache-2.0 SPDX header on the generated Discovery
stubs and renames the internal stub-regeneration command. No runtime library
code or public API changes.

### Changed

- **Apache-2.0 SPDX headers on the generated Discovery stubs.** The six
  `nvidia/omniverse/discovery/capabilities/v2alpha` stubs (`*_pb2.py`,
  `*_pb2_grpc.py`, `*_pb2.pyi`) carry the same Apache-2.0 header as the rest of
  the published source. `uv run nc-update-proto` stamps them after
  `buf generate`, and `nc-lint`'s header check now covers the `nvidia/` tree and
  `.pyi` files. `header_lint` gains `stamp_header`; no other library code or
  public API changes.
- **The internal stub-regeneration command is `nc-update-proto`** (was
  `update_proto`). This package's console scripts are installed into every
  consuming service's venv, where `update_proto` collided with the event
  services' own `update_proto` command, so `uv run update_proto` in a service
  could run this package's `buf generate` instead of the service's script.
  After a service moves to this release, an existing local venv loses the
  service's own `update_proto` too (uninstalling the old version removes
  `bin/update_proto`); run `uv sync --reinstall-package <service>` once, or
  recreate the venv. Fresh CI and Docker builds are unaffected.

## [6.12.7]

A PATCH release that raises security floors on three dependencies, so consuming
services cannot resolve a version with a known advisory. No library code or
public API changes, but the PyJWT floor brings behavior changes from PyJWT
2.14 (see below).

### Changed

- **`pyjwt[crypto]>=2.15.1`** (was `>=2.13.0`), past BDSA-2026-35552. PyJWT
  2.14 changes behavior in the `jwt` principal source: the JWKS client no
  longer follows redirects, so a `jwks_uri` that redirects now fails every
  request with a server error; and after an unknown `kid` it refetches the key
  set only once 30 seconds have passed since the last fetch, so a token signed
  with a just-rotated key can get a 401 for up to 30 seconds. Every principal
  source also parses tokens more strictly (compact JWS encoding rules).
  Deployments using `jwt` should confirm their `jwks_uri` answers 200 without
  a redirect.
- **`urllib3>=2.8.0`** (was `>=2.7.0`), past BDSA-2026-35864 and BDSA-2026-35861.
- **`anyio>=4.15.1`** (new floor on a transitive dependency, reached through
  FastAPI and Starlette), past BDSA-2026-33862.
- `uv.lock` relocked accordingly; `typing-extensions` moves to 4.16.0 with anyio.

### Version

- Bumped `6.12.6` -> `6.12.7`.

## [6.12.6]

A PATCH release of contributor and agent guidance only. No library code, public
API, or behavior changes.

### Changed

- **`AGENTS.md`.** A root `AGENTS.md` points coding agents at `CLAUDE.md`.
  Like `CLAUDE.md`, it is excluded from the GitHub publish.
- **`CLAUDE.md` trimmed.** Its header rule now names the Apache-2.0 SPDX header,
  the obsolete `.cursorrules` file is removed, and the architecture section
  points at the SDD.
- **Comment guidance.** `docs/engineering-best-practices.md` gains a Comments
  section: put machine-checkable facts in types, named constants, assertion
  messages, and test names; keep comments for what the code cannot say; and fix
  every comment a behavior change makes false.

### Version

- Bumped `6.12.5` -> `6.12.6`.

## [6.12.5]

A PATCH release that finishes the Apache-2.0 relicensing. License headers only;
no library code, public API, or behavior changes.

### Changed

- **Apache-2.0 headers on the remaining YAML and shell files.** `buf.gen.yaml` and
  `scripts/prepare-github-publish.sh` replace their `LicenseRef-NvidiaProprietary`
  preamble with the Apache-2.0 SPDX header; both are excluded from the GitHub
  publish. The published `tools/docker-compose/` files (`docker-compose.yaml`,
  `otel-collector-config.yaml`, `prometheus.yaml`) gain the header they lacked.
- **Contributor guides.** `docs/doc-writing-guide.md` and
  `docs/engineering-best-practices.md` told contributors to use the proprietary
  header; they now name the Apache-2.0 header.

### Version

- Bumped `6.12.4` -> `6.12.5`.

## [6.12.4]

A PATCH release, test-only: no runtime code changes. Twenty-one log-capture
assertions failed on pytest 9.1 or newer, which the GitHub mirror resolves
because it ships no lockfile; this repository's lock holds pytest at 9.0.2.

### Fixed

- **Log-capture tests on pytest 9.1.** The autouse `before_each` fixture mocks
  `logging.getLogger`, which kept `caplog`'s handler off the real root logger;
  these tests only saw records through a handler that pytest 9.0 leaked and 9.1
  no longer does. A shared `real_get_logger` fixture in `tests/conftest.py`
  restores the real function for the tests that read log records, and replaces
  the file-local copies in `tests/rabbitmq/test_channel_pool.py` and
  `tests/auth/auth_tokens/test_user_info_service_decoder.py`. Its docstring
  explains the pytest mechanism.
- **Two vacuous assertions.** `test_missing_bearer_prefix_logs_no_error` and
  `test_a_fleet_at_the_ceiling_below_the_floor_does_not_warn_each_poll` had been
  passing over an empty record list; they now read real records.

### Version

- Bumped `6.12.3` -> `6.12.4`.

## [6.12.3]

A PATCH release that returns the `github-publish` CI component to its `main`
branch, which the permissions and discovery repositories also track, and
publishes `.gitignore` as part of the source. No library code, public API, or behavior
changes.

### Packaging

- **`github-publish@main`.** The component's `main` branch publishes
  `.gitignore` (ci-components!7), and `.publishignore` no longer excludes it.
  The publish commit's `git add -A` honors that file, so
  `github:dry-run-resolve` now fails if it would drop any published file. It
  drops none in 6.12.3.
- **`github:dry-run-resolve` snapshot.** The job rebuilt the published tree with
  `git check-ignore`, which also applied this repository's own `.gitignore`.
  That filter would have hidden exactly the files the new check exists to
  catch, so the job now applies only `.publishignore`, as the component's tar
  filter does. No tracked file matches `.gitignore`, so the file set is
  unchanged.

### Version

- Bumped `6.12.2` -> `6.12.3`.

## [6.12.2]

A PATCH release that pins the `github-publish` CI component to the released tag
`0.3.0` (the ref storage-service and notification_protos also use) instead of
tracking its `main` branch. No library code, public API, or behavior changes.

### Packaging

- **`github-publish@0.3.0`.** The component's `main` branch now ships
  `.gitignore` in the published snapshot, and the target commit honors it, so any
  file it ignores would be silently left out of a release. No file was affected
  in 6.12.1, whose `.gitignore` ignores nothing that ships. Version `0.3.0` drops
  `.gitignore`, and `.publishignore` now lists it as well, so a later bump cannot
  reintroduce the problem through this repository's own `.gitignore`.
- **`.syncignore` excluded from the publish.** It is the developer sandbox's
  file-share configuration, not part of the library.
- The `.gitignore` and `.syncignore` that 6.12.1 published are removed by this
  release's publish, which replaces the subfolder's contents.

### Version

- Bumped `6.12.1` -> `6.12.2`.

## [6.12.1]

A PATCH release that moves the source-available GitHub publish from a
per-component repository into the consolidated repository (now
`NVIDIA-Omniverse/usd-content-workspace`), under
`notifications/reference/common`. No library code, public API, or behavior
changes; internal consumers keep depending on the internal-PyPI package as before.

### Packaging

- **`github:publish` target.** The job now publishes into the consolidated
  repository under the `notifications/reference/common`
  subfolder, so each release replaces only that subfolder's contents and leaves
  the rest of the repository untouched.

### Removed

- **Auto-tagging workflow.** `.github/workflows/tag-release.yml` is removed.
  GitHub runs workflows only from the repository root, so a copy published into
  the `notifications/reference/common` subfolder would never run.

### Version

- Bumped `6.12.0` -> `6.12.1`.

## [6.12.0]

A MINOR release, behaviour-changing but not API-breaking: both transports now
validate the bearer token before any handler runs.

A behaviour change to both transports' authentication layer, plus two decoder
fixes it depends on. The gRPC `AuthInterceptor` and the REST
`auth_token_middleware` used to only *extract* the bearer token: they refused a
missing `Authorization` header, bad UTF-8, or a missing `Bearer ` prefix, then
stashed the raw string on the request context. Any Bearer-shaped string reached
the handler, and a request was validated only if something on its own code path
happened to await `principal_id.principal_id()` — in the consuming services that
was a side effect of stamping the principal onto an event or into a queue name,
so deleting an unrelated call could silently remove authentication. Both layers
now resolve the token through the configured `principal_source` before any
handler runs.

Resolution is not the same as verification in every mode, and this release does
not change which modes verify: `jwt` (JWKS signature, audience, expiry),
`opaque` (introspection) and `userInfoService` verify the token as it is
resolved, while `jwtUnverified` — the default — decodes the payload, verifies
nothing, and continues to treat an upstream validator such as Envoy as the trust
anchor. What changes is *when* the resolution happens and what a failure looks
like.

One upgrade impact for consumers beyond the status codes below: validation now
precedes **every** handler statement, so a service with a must-be-first
pre-handler refusal must re-check its ordering at the pin bump. The Event
Consumer Service's capacity guard is the known case — it must answer
`RESOURCE_EXHAUSTED` first for Envoy's retry policy, and an over-capacity pod
receiving a bad token now returns `UNAUTHENTICATED` instead.

### Changed

- **`AuthInterceptor._set_auth_context` and `auth_token_middleware` resolve the
  token before the handler.** After setting `auth_token_var`, each awaits
  `principal_id.principal_id()` when authentication is enabled. An
  `AuthenticationError` is refused in place — gRPC aborts with
  `UNAUTHENTICATED`, REST returns `401` with a `{"detail": "<message>"}` body
  (e.g. `Invalid or expired JWT`) — and a `WARNING` line is logged
  (`Rejecting request with an unresolvable token: …`). On REST that is the only
  line; on gRPC the abort raises `AbortError`, which
  `ExceptionInterceptor.handle_exception` already logs via `logger.exception`
  before passing it through, so a gRPC refusal emits the `WARNING` plus a
  pre-existing `ERROR`-level traceback naming `AbortError`. Any other failure,
  notably a `ServerError` from an unreachable identity provider or user-info
  service, propagates to `ExceptionInterceptor` / the error-handling middleware
  as `INTERNAL` / `500`, so a provider outage is not reported to callers as a bad
  token. Exclusions are unchanged and stay authoritative: `EXCLUDE_METHODS`
  bypasses `_set_auth_context` entirely (health, reflection, capabilities
  `ListServices`), as do the REST `excluded_paths`. **Metrics move with the
  status codes:** gRPC auth refusals leave the `UNKNOWN` + error bucket for the
  `UNAUTHENTICATED` bucket, and JWKS-fetch outages land in `INTERNAL`, so
  dashboards and alerts keyed on those buckets shift at the pin bump. gRPC
  keeps its span and its metrics either way, because the tracing and metrics
  interceptors sit *outside* the auth interceptor. **REST loses both its span
  and its traceback:** the tracing middleware is registered *inside* the auth
  middleware, so a request refused by auth never reaches tracing and produces
  **no trace span** where a handler-level refusal produced one, and its log
  drops from an `ERROR` with a traceback to the single `WARNING` above.
- **`/redoc` and `/docs/oauth2-redirect` join the default REST
  `excluded_paths`,** beside `/docs` and `/openapi.json`. FastAPI generates all
  four; the two new ones previously required a Bearer-shaped header to serve
  their static HTML, and would now require a *resolvable* token. A consumer that
  passes its own `excluded_paths` list does **not** inherit this and must add
  them itself.
- **`principal_id()` now refuses a principal claim that is present but empty.**
  It raises the same `AuthenticationError("Token did not resolve to a principal
  id")` an absent claim does, so the caller receives a `401` (REST) /
  `UNAUTHENTICATED` (gRPC) instead of being admitted with an empty identity. An
  empty claim carries no identity, would be stamped onto published events as
  `principal_identity=""`, and would collapse every such caller into a single
  per-principal cache scope downstream; refusing it surfaces a misconfigured
  `principal_id_claim` setting rather than hiding it. The refusal happens before
  the cache write, so an empty principal is never cached, never served to a
  later in-service caller, and never stamped onto an event. Upgrade impact: a
  caller whose token carries an empty claim is admitted today — with the empty
  principal passed through to the consuming service — and is refused after the
  pin bump.
- **The REST auth middleware's missing-`Bearer` / malformed-`Authorization`
  refusal now logs at `WARNING` rather than `ERROR`,** matching the
  unresolvable-token refusal beside it and the gRPC interceptor: a refused
  request is routine traffic, not a service fault. The refusal itself — `401`
  with `{"detail": "Invalid Authorization header"}` — is unchanged.

### Fixed

- **`jwtUnverified` no longer stores the token's `exp` as its cache lifetime.**
  `jwt_unverified_decoder.get_user_id` returns `time.time() + 300` as the second
  tuple element instead of the token's own `exp`. The value `principal_id()`
  caches is a statement about when the cached principal goes stale, not about
  when the token stops being valid — returning the token's `exp` let the caching
  layer refuse, on a later call within the same request, a token this mode had
  deliberately accepted. That is now reachable on every request, since the
  middleware and interceptor resolve first and the handler path resolves again.
  The set of tokens the mode admits is unchanged: an `exp` claim is still
  required, its value is simply not enforced, so the documented "no signature,
  audience, or expiry verification" behaviour still holds. This matches what the
  `opaque` and `userInfoService` decoders already returned for tokens whose
  expiry they cannot judge.
- **A JWKS connection failure under `principal_source=jwt` is a `ServerError`,
  not an authentication failure.** `jwt_decoder._decode_token` catches
  `jwt.exceptions.PyJWKClientConnectionError` and raises `ServerError`
  (`500` / `INTERNAL`); failing to reach the IdP is the server's fault, not the
  caller's, and reporting it as a `401` sent callers chasing their credentials
  during an IdP outage. Two cases deliberately stay `401`: an unknown key id,
  which is attacker-controllable, and — residually — a malformed or empty JWKS
  document, which PyJWT raises as the same `PyJWKClientError` type as an unknown
  `kid` and is therefore type-indistinguishable from it. Widening the carve-out
  to `PyJWKClientError` would make an attacker-controllable input a `500`, so it
  is not the fix.

## [6.11.0]

A MINOR release with two behaviour changes and no public API break: the
autoscaler's replica floor is enforced in both directions, and the graceful
shutdown covers the REST server and gains an optional app-side settle delay.

Makes `scaler_min_replicas` a true replica floor. It was enforced only as a
scale-down clamp, so nothing ever raised a fleet that sat below it — a
deployment created at one replica, or hand-scaled under the minimum, simply
stayed there, and the setting's name promised a guarantee the scaler did not
keep. The scaling decision now also scales up when the fleet is below the
floor, through the same gates and the same replica ceiling as any other
scale-up. Scale-up on capacity pressure, the scale-down deadband, the smoothing
window, and every default are unchanged; nothing public in the scaler surface
was added, removed, or renamed. One upgrade impact: a `scaler_min_replicas`
below `1` is now rejected at startup, so a deployment currently running
`OMNI_EVENTS_SCALER_MIN_REPLICAS=0` (chart `minReplicas: 0`) has its
**autoscaler** fail to start after upgrading until the value is raised to 1
or more; service pods do not load the scaler settings and are unaffected.

Also makes the graceful drain cover the REST server, and moves the
endpoint-settle delay out of the deployment chart and into the library. The
REST server used to leave the drain at `SIGTERM`: stock
`uvicorn.Server.serve()` installs its own `SIGTERM`/`SIGINT` handlers, which
ran alongside the asyncio handlers `start_servers` registers, so uvicorn
closed its listening socket as the drain was only beginning and new REST calls
routed to a terminating pod failed for the whole drain window while gRPC calls
on the same pod succeeded. `start_servers` now owns both signals for the whole
process and both servers stop together at the end of the drain. On top of that
the drain gains an optional first phase — an endpoint-settle delay during
which the process keeps serving exactly as before — so a deployment no longer
needs the Kubernetes `lifecycle.preStop.sleep` handler, which is gated by the
`PodLifecycleSleepAction` feature gate (off by default on 1.29) and therefore
cannot be installed on every cluster. The settle ships **off** (default `0`)
and takes effect only once a deployment's chart templates the new environment
variable. Shutdown *timing*, however, does change on the pin bump alone, even
with the settle at `0`: uvicorn's teardown — `Server.shutdown()`'s wait for
open connections to close, followed by the lifespan shutdown, which sleeps 5
seconds in the library's default lifespan — used to begin at `SIGTERM` and run
concurrently with the drain, and now begins only when `shutdown_event` is set
at the end of the drain. Wall-clock shutdown therefore grows by that REST
teardown tail at every pod stop, and for a service holding SSE connections the
*start* of the (unbounded) connection wait moves later by up to a full drain
window. Deployments must size `terminationGracePeriodSeconds` for the tail —
see the grace-period invariant under **Added**. One behavior loss is
deliberate: uvicorn's repeated-Ctrl-C force-quit escape is gone (see
**Changed**).

The signal-handler fix relies on overriding uvicorn's undocumented
`Server.capture_signals()`. Because the library declares no direct uvicorn
bound, `notification_common.rest.notification_rest_server` now checks at
import that the installed uvicorn still defines that method on `Server` itself
and raises `ImportError` otherwise, so a future uvicorn that moves its signal
handling fails loudly at process start instead of silently reopening the
defect.

### Fixed

- **A fleet below `scaler_min_replicas` is now raised toward the floor**
  instead of the minimum being enforced only when shrinking.
  `_make_scaling_decision` carries two scale-up triggers, evaluated in order:
  capacity pressure (`total_available < scaler_min_available_capacity`,
  unchanged and still claiming the iteration first, since both triggers take
  the same action and the pressing reason is the one worth logging), then
  `current_replicas < effective_floor`, where `effective_floor =
  min(scaler_min_replicas, scaler_max_replicas)`. Both delegate to the same
  `_try_scale_up`, so a floor-driven scale-up is refused while any pod is still
  starting, honours the scale-up cooldown, is capped by `scaler_max_replicas`,
  steps one replica per poll, and stamps the last-scale-up time and the
  post-scale-up stabilization anchor on success (inert at the floor, where the
  scale-down clamp refuses to shrink anyway). The scale-down clamp itself
  (`Already at min replicas (N)`, read at the configured minimum) is unchanged
  in code; outside the `min > max` band below, the line is now only reachable at
  exactly the floor, since a fleet below it — which previously fell through to
  the clamp whenever it also had sustained excess capacity — is claimed by the
  floor branch first.
- **The floor keys on the Deployment's `spec.replicas` and only on iterations
  that reach a decision.** A restarted scaler enforces it on its first observed
  iteration — up-path amnesia is fail-safe upward, the same reasoning that
  keeps `_last_scale_up_time` at `0`. An iteration with no Running pod, or in
  which no pod returns metrics, still takes no action and records no window
  sample. So `kubectl scale --replicas=0` remains an effective off switch and
  is never reverted, and an entirely crash-looping or unreachable fleet is not
  raised; a manual scale to a *nonzero* count below the floor is reverted
  upward, one replica per poll.
- **A distinct scale-up log reason.** `_try_scale_up` now requires the reason
  rather than deriving one, so the single `SCALE UP: {from} -> {to} ({reason})`
  line never asserts a capacity shortfall for a scale-up the floor asked for.
  The capacity-pressure line is byte-identical to before
  (`SCALE UP: 4 -> 5 (available 760 < min 800)`); the floor variant reads
  `SCALE UP: 2 -> 3 (below min replicas 3 (available 1438))`. The signature
  change is private: `_try_scale_up` drops its `total_available` parameter and
  gains a required `reason`.
- **A once-per-process startup WARNING when `scaler_min_replicas >
  scaler_max_replicas`.** `effective_floor` is clamped to the ceiling so the
  misconfiguration cannot ask for a scale-up that the max check would refuse,
  logging `Already at max replicas` at WARNING on every poll for the life of
  the process. The startup line names it once instead: the two directions are
  enforced at different values — upward enforcement capped at `max_replicas`,
  scale-down still refusing to shrink below the configured `min_replicas` — so
  the fleet holds wherever it already sits within `[max, min]`.
- **`scaler_min_replicas` is validated to be at least 1 at load**, mirroring
  the existing `scaler_min_available_capacity` bound; a value below 1 raises
  `scaler_min_replicas must be at least 1` while the settings singleton is
  constructed at import, so a misconfigured scaler fails at startup rather
  than running degraded. The rejection is about the off switch rather than
  about capacity: scaling the deployment to zero replicas is the documented
  operator off switch, and the scaler never reverts it, because it acts only
  on iterations that observe a running, reporting fleet — a fleet at zero
  offers nothing to observe and stays down until an operator scales it back
  up. At an effective floor of zero the upward floor branch is inert, since
  every replica count already sits at or above the floor, so nothing stands
  between the fleet and that unrecoverable state. The constraint closes the
  `scaler_min_replicas` route to such a floor; the `scaler_max_replicas`
  route stays open and is surfaced by the `min > max` startup WARNING
  instead.
- **Operational consequence: a deployment left at the defaults idles at
  `scaler_min_replicas` pods**, where it previously idled wherever the
  capacity signal settled, which could be below the minimum. No default value
  changed.
- **The REST server now stops at the end of the drain, not at `SIGTERM`.**
  `rest/notification_rest_server.py` gains a module-level
  `_NoSignalUvicornServer(uvicorn.Server)` whose `capture_signals()` is a
  no-op context manager, and `NotificationRestServer.start_server` constructs
  that instead of a stock `uvicorn.Server`. Everything else is inherited —
  `serve()`, `_serve()`, `main_loop()` and `shutdown()` are unchanged, so the
  `Date`/`Server` response headers are exactly as before and `shutdown()` still
  waits for open connections to close and then runs the lifespan shutdown, by
  the same mechanism as before. What changed is *when* that teardown starts: it
  used to begin at `SIGTERM`, overlapping the drain, and now begins only at the
  end of the drain, so it adds its tail to every pod's wall-clock shutdown
  (see the grace-period invariant under **Added**). uvicorn now installs **no**
  process signal handlers; `start_servers` owns `SIGINT`/`SIGTERM` for the whole
  process and the REST server stops only when the shared `shutdown_event` is set
  at the end of the drain — the same contract the gRPC server already had.
  Before this change a terminating pod served gRPC unary calls for the whole
  drain window while new REST calls routed to it failed, so the documented
  "keep serving while draining" contract held for gRPC only.

### Added

- **`OMNI_EVENTS_SHUTDOWN_SETTLE_DELAY_SECONDS`** (`CommonSettings`
  `shutdown_settle_delay_seconds`, float, `ge=0`, default `0.0`) — an optional
  endpoint-settle delay that runs as the first phase of the graceful shutdown,
  before `begin_draining()`. For the configured delay the process keeps
  serving normally — readiness stays SERVING and the capacity guard still
  admits new streams — which gives the routers and endpoint controllers in
  front of the pod time to converge on its removal while it can still serve
  what they send. At `0`, the default, the phase is skipped entirely,
  including its log lines, so the setting has no effect until a deployment's
  chart templates it, the same way the chart already templates
  `OMNI_EVENTS_SHUTDOWN_DRAIN_WINDOW_SECONDS`. The shutdown sequence becomes:
  settle → `begin_draining()` → `drain_non_durable()` →
  `prepare_to_shutdown()` → set the shutdown event, at which point both
  servers stop. The settle phase swallows its own exceptions, so a settle that
  fails is logged and falls through into the drain; cancellation still reaches
  the `finally` that stops the servers, and a second signal during the settle
  does not shorten it (the once-only scheduling guard is unchanged).
  Deployments can use this in place of the Kubernetes `lifecycle.preStop.sleep`
  handler, which the `PodLifecycleSleepAction` feature gate leaves unavailable
  on clusters below 1.30 with default gates. A chart that adopts it should
  drop its `preStop` sleep in the same change, because the grace-period
  invariant becomes `terminationGracePeriodSeconds >= settle delay + drain
  window + REST teardown tail`, where the tail is the lifespan shutdown (5
  seconds in the library's default lifespan) plus however long open connections
  take to close; leaving both the `preStop` sleep and the settle in place
  stacks the two delays inside one grace period. The tail is a new term —
  before this change it ran concurrently with the drain — and it applies even
  at a settle of `0`, so charts that never adopt the setting must still check
  the invariant. Concretely, a chart sitting at its guard minimum (the
  consumer's 15 s `preStop` + 30 s drain = 45 s) is now `SIGKILL`ed the
  moment the drain ends, before the REST teardown gets any of its tail,
  when the drain runs to its full window — where the 5-second lifespan
  sleep previously fit inside the drain window; a pod whose non-durable
  streams drain early finishes 15 + drain + 5 well inside 45 s. The
  consumer's shipped 60 s default leaves headroom for the 5-second lifespan
  sleep, but not for an open-connection wait that runs long — for a service
  holding SSE streams that wait is unbounded.

### Changed

- **`NotificationRestServer.start_server` has a new stopping contract.** This
  is public API: it used to stop on `SIGTERM` (via uvicorn's own handler) and
  now stops **only** when the `shutdown_event` passed to it is set. Callers
  that drive it through `start_servers` get that for free — the
  graceful-shutdown sequence sets the event at the end of the drain. A caller
  driving `start_server` outside `start_servers` must now set the event
  itself; nothing else will stop the server.
- **uvicorn's repeated-Ctrl-C force-quit escape is gone.** Because uvicorn
  installs no handlers, nothing sets its `force_exit` any more, and a second
  `SIGINT`/`SIGTERM` hits the library's once-only scheduling guard and is a
  no-op. A REST connection wait that hangs after the drain therefore ends only
  at the kubelet's grace deadline (`SIGKILL`) or at an external kill —
  locally, at the developer's own kill of the process. Bounding that wait
  (uvicorn's `timeout_graceful_shutdown` is not set) is deliberately deferred
  to its own Issue.
- **Consumer test suites that patch `uvicorn.Server` no longer intercept the
  REST server.** A suite doing `mocker.patch.object(uvicorn, "Server", ...)`
  to stand in for the server intercepts nothing now and will construct and run
  a real server. The class `NotificationRestServer.start_server` instantiates
  today is `_NoSignalUvicornServer` on
  `notification_common.rest.notification_rest_server`, so that is the practical
  patch target — for example `mocker.patch.object(notification_rest_server,
  "_NoSignalUvicornServer", ...)`. This repo's own `tests/rest/conftest.py` is
  updated the same way.

## [6.10.1]

A PATCH release that raises the `cryptography` dependency floor from
`>=46.0.6` to `>=50.0.1`. Container OSS scans of the consuming services flag
three HIGH advisories in `cryptography` 46.0.7 (fixed in 48.0.1, 49.0.0 and
50.0.0). This library is the package that actually imports `cryptography`
(through `pyjwt`), so the floor lives here as the single source of truth and a
future relock in any consuming service cannot resolve below the fixed version.
No public API or behavior change.

### Changed

- **`cryptography>=50.0.1`** (was `>=46.0.6`) in `pyproject.toml`; `uv.lock`
  relocked accordingly.

## [6.10.0]

A MINOR release with one behavior change to the shared REST tracing
middleware: the `http.url` span attribute is now recorded **without the
request's query string** (scheme, host and path only). Query strings routinely
carry credentials and other sensitive values — the Event Consumer Service's
non-durable `reconnect_token`, a bearer capability, travels there — and the
OpenTelemetry semantic conventions warn that `http.url` / `url.full` may hold
sensitive data. A shared middleware cannot enumerate its consumers' parameter
names for per-parameter redaction, so path-only is the defensible default.
`http.method`, `http.route` and `http.status_code` are unchanged, so per-route
trace grouping is unaffected. This applies to every service that mounts the
middleware (Event Consumer Service, Event Aggregation Service).

### Changed

- **`http.url` span attribute no longer includes the query string.**
  `rest/middleware/tracing_middleware.py` sets `http.url` from
  `request.url.replace(query="")`. Consumers that filtered traces on a query
  parameter via `http.url` must use a route- or handler-level attribute
  instead.

### Fixed

- `uv.lock` now records the package's own version (it had lagged behind
  `pyproject.toml` since 6.9.0).

## [6.8.1]

A robustness-only patch over the 6.8.0 graceful-drain shutdown path. The
`_graceful_shutdown` coroutine now runs its server-stop steps in a `finally`, so
a drain that raises can no longer hang the process until SIGKILL, and the two
`shutdown_drain_*` settings gain positive-value validators so a misconfigured
non-positive poll interval is rejected at load instead of turning the drain into
a tight busy-loop. The autoscaler scenario tests were re-driven at stream-unit
capacity (test-only). This is a PATCH: no public API was added, removed, or
renamed; the behavior change is limited to shutdown robustness and settings
validation.

### Changed

- **Exception-safe graceful shutdown.** `servers/start_servers.py`
  `_graceful_shutdown` now wraps the drain (`begin_draining()` ->
  `drain_non_durable()`) in a `try` whose `finally` always runs the server-stop
  steps (`prepare_to_shutdown()` -> set the shutdown event). If the drain raises
  for any reason it is logged and shutdown still completes, rather than hanging
  until the kubelet SIGKILLs at the grace deadline.

### Added

- **Positive-value validators on the drain settings.** `CommonSettings`
  now constrains `shutdown_drain_window_seconds` to `ge=0` and
  `shutdown_drain_poll_interval_seconds` to `gt=0`, so a non-positive poll
  interval — which would turn the drain loop into a tight busy-loop for the whole
  window — is rejected at settings-load time instead of at runtime.

### Version

- Bumped `6.8.0` -> `6.8.1`.

## [6.8.0]

Reworks the shared shutdown path so a pod on `SIGTERM` drains its in-flight
non-durable streams before the connector closes, instead of force-dropping them
at the scale-down / shutdown boundary. On `SIGTERM` (or `SIGINT`) the
`RabbitMQConnector` now enters a distinct **draining** state, allows any
in-flight non-durable stream checkouts a bounded, configurable drain window to
wind down, and only then marks the connector closing and stops the servers so
the existing `close()` -> `close_all()` path reclaims whatever is still held.
Close happens strictly **after** the window, never before. The durable path and
the non-streaming clients (`http_client`, redis) are untouched; nothing public
was removed or renamed — two settings were **added** and the shutdown behavior
**changed** only.

### Changed

- **Drain-before-close shutdown sequence.** `servers/start_servers.py` now
  routes `SIGTERM`/`SIGINT` through a single-shot `_graceful_shutdown` coroutine
  — `begin_draining()` -> `drain_non_durable()` -> `prepare_to_shutdown()` ->
  stop the gRPC/REST servers — rather than the previous immediate
  `_prepare_shutdown` (retained only as a test-covered fast-path helper with no
  production callers). The connector waits up to the configured window for the
  non-durable pool's in-flight checkout count to reach zero, then proceeds to
  close. Non-durable checkouts still held at window expiry — including the
  hung-orphan case that blocks at `messages_queue.get()` — are reclaimed by the
  subsequent `close()` -> `close_all()`, so no live stream is force-dropped. The
  wait is keyed on the non-durable pool only; the durable pool and its checkouts
  are not part of the drain accounting and its behavior is unchanged.

### Added

- **`OMNI_EVENTS_SHUTDOWN_DRAIN_WINDOW_SECONDS`** (`CommonSettings`, default
  `30.0`) — the bounded graceful-drain window, in seconds, that the connector
  waits on `SIGTERM` for in-flight non-durable streams to wind down before
  closing.
- **`OMNI_EVENTS_SHUTDOWN_DRAIN_POLL_INTERVAL_SECONDS`** (`CommonSettings`,
  default `0.5`) — the interval, in seconds, at which the drain loop re-reads the
  non-durable pool's active checkout count while waiting out the window.
- **`draining` boolean attribute on `RabbitMQConnector`** (module
  `notification_common.rabbitmq.rabbitmq_connector`) — a plain, synchronously
  readable attribute, set `True` by `begin_draining()` and independent of
  `closing` (`begin_draining()` never sets `closing` and tears nothing down).
  Services read `connector.draining` per request to report `NOT_SERVING` on the
  readiness probe and to refuse new-stream admission while the pod is winding
  down.

Cross-service note: `servers/start_servers.py` is shared, so every library
consumer inherits the reworked shutdown path and the two new settings **at its
next pin bump**, not on publication — the same inheritance model as prior
shared-shutdown changes. `event_aggregation_service` pins
`notification-common==6.3.2` and is a publisher expected to hold zero non-durable
consumer checkouts, so the drain window is a no-op there: the drain returns
near-immediately and the pod closes at once. Any consumer can tune or effectively
disable the wait per-deployment via
`OMNI_EVENTS_SHUTDOWN_DRAIN_WINDOW_SECONDS=0`.

### Version

- Bumped `6.7.0` -> `6.8.0`.

## [6.7.0]

Raises the non-durable `ChannelPool`'s two default packing/prefetch globals now
that the overflow band (6.6.0) makes packing safe. `rabbitmq_channel_client_limit`
goes `1 → 8` and `rabbitmq_channel_prefetch_count` goes `500 → 15`. At the
shipped `rabbitmq_channel_pool_size` of 100 the soft packing threshold rises from
100 to 800 non-durable streams per pod, and — because prefetch is the dominant
broker-memory lever — the worst-case per-stream broker memory drops roughly 28×
(a saturated stream costs ~32 kB at prefetch 15 versus ~883 kB at 500). The
non-durable pool still carries **no per-pool `ChannelPoolConfig` override**: the
two settings remain process-wide globals that the pool late-binds on every read,
so the coupling contract in the consuming service's `stream_memory_cap.py`
remains intact and that service's computed memory-safe stream cap rises
automatically with the new defaults. Nothing public was added, removed, or
renamed — defaults only.

### Changed

- **`rabbitmq_channel_client_limit` default `1 → 8`** and
  **`rabbitmq_channel_prefetch_count` default `500 → 15`.** These govern the
  non-durable pool only; the durable / DLQ-browse twins
  (`rabbitmq_durable_channel_client_limit` 1, `rabbitmq_durable_channel_prefetch_count`
  500) and the pool sizes (`rabbitmq_channel_pool_size` 100,
  `rabbitmq_channel_pool_min_size` 75) are unchanged. `client_limit` is a packing
  *target* on an overflow-enabled pool, not a hard per-channel bound.

Cross-service note: these are process-wide globals, so every library consumer
inherits the new defaults **at its next pin bump**, not on publication — the same
inheritance model as the 6.6.0 overflow default. `event_aggregation_service`
pins `notification-common==6.3.2` and uses the pool publish-only (via
`notification_common.rabbitmq.publisher.Publisher`); publish channels host no
consumers, so the prefetch change is inert there, and `client_limit=8` only means
a pooled publish channel may carry up to 8 concurrent short-lived publishers
instead of 1 (fewer channels, no prefetch-memory hazard). Any consumer wanting
the previous behavior opts out per-setting:
`OMNI_EVENTS_RABBITMQ_CHANNEL_CLIENT_LIMIT=1` /
`OMNI_EVENTS_RABBITMQ_CHANNEL_PREFETCH_COUNT=500`.

### Version

- Bumped `6.6.0` -> `6.7.0`.

## [6.6.0]

Gives the non-durable `ChannelPool` a second admission tier. When overflow is
enabled, no open channel is under `client_limit`, and the pool has already
grown to `pool_size`, a new acquisition is admitted anyway onto the
least-loaded open channel instead of waiting out `wait_ms` and then failing.
`client_limit` therefore becomes a packing *target* on an overflow-enabled
pool rather than a hard per-channel bound, and the band it opens has no upper
bound of its own — bounding it is the consuming service's job, via a
memory-safe admission cap that refuses before the process runs out of memory.
The pool admits rather than refuses because a pinned gRPC stream cannot be
rescued by scaling out: a burst arriving on connections already bound to this
process has nowhere else to go. The durable / DLQ-browse pool is strict by
construction and is unaffected. Overflow is **on by default**, so every
consumer of the library inherits the new pool-exhaustion behavior at its next
pin bump, publishers included (see the cross-service note below). Nothing
public was removed or renamed.

### Added

- **`rabbitmq_channel_overflow_enabled` (default `True`, bool,
  `OMNI_EVENTS_RABBITMQ_CHANNEL_OVERFLOW_ENABLED`).** Switches the
  non-durable pool's overflow band on and off process-wide. It reaches the
  pool as a late-bound `ChannelPoolConfig.overflow_enabled` property that
  re-resolves against `common_settings` on every read, like the sibling pool
  limits, so a runtime or test-time settings override takes effect on an
  already-constructed pool. Unlike those siblings it is global-only by
  design: it takes **no constructor parameter** and has no per-pool backing
  field, because the consuming service's memory-safe stream cap couples to a
  non-durable pool that carries no per-pool overrides. A strict pool is
  therefore obtained either by setting the global to `False` or by using
  `DurableChannelPoolConfig`, which hard-overrides the property; there is
  deliberately no `rabbitmq_durable_channel_overflow_enabled` twin.
- **`pool_stats()` gains an `overflow_enabled` key** reporting the effective
  value for that pool, so a durable pool reports `False` while the global is
  `True`. The addition is purely additive and the autoscaler/readiness
  protocol that keys on these fields needs no change: no pre-existing key was
  removed, renamed, or re-typed, and every one keeps its existing
  computation — `max_client_capacity` is still `pool_size * client_limit`.
  What that number now *denotes* does shift; see `### Changed` below.

### Changed

- **`ChannelPool` acquisition is now two-tier.** A fourth tier was inserted
  into `_get_best_available_channel()` between the pool-growth tier and the
  blocking-wait tier, under the same acquisition lock: when the pool is
  overflow-enabled and no open channel sits under `client_limit`, it packs
  the checkout onto the least-loaded *open* channel regardless of that
  channel's client count. Placing it after the growth tier is what guarantees
  overflow never engages while the pool can still grow — reaching the tier is
  itself the proof that the pool is full, so no separate growth re-check is
  needed. The scan is factored out as the private
  `_least_loaded_channel(limit)` helper, which the under-limit tier reuses
  and which the overflow tier calls with `None` to ignore the limit.
- **`ChannelNotAvailableError` is no longer raised at pool saturation on an
  overflow-enabled pool.** A full pool whose channels are all at or past
  `client_limit` now admits instead of refusing. The error stays reachable
  for a closed pool, for the `wait_ms` timeout, and for the degenerate states
  where there is no open channel to pack onto at all (`pool_size` of `0`, or
  a pool whose tracked channels are all closed) — which is likewise all that
  the blocking-wait tier is still reached for while overflow is enabled.
- **`max_client_capacity` in `pool_stats()` now denotes the soft packing
  threshold, not a ceiling.** Its computation is unchanged (see `### Added`),
  but because `client_limit` became a packing target, `total_client_count`
  may legitimately exceed `max_client_capacity` on an overflow-enabled pool.
  Read it as the point at which the pool stops growing and starts packing,
  not as an absolute upper bound on concurrent clients.
- **`DurableChannelPoolConfig.overflow_enabled` hard-returns `False`,**
  ignoring the global. The durable / DLQ-browse pool keeps its strict
  one-tier behavior: `client_limit` of `1` keeps a durable checkout exclusive
  on its channel, `client_limit` remains a hard bound there, and the pool
  still raises `ChannelNotAvailableError` once all `pool_size` channels are
  checked out rather than silently sharing.
- **Cross-service: this release also changes publisher-side pool-exhaustion
  semantics in `event_aggregation_service`.** Because the global defaults to
  `True`, the overflow band applies to every plain `ChannelPoolConfig` pool
  library-wide — including the non-durable pool that
  `notification_common.rabbitmq.publisher` borrows from, since its
  `get_channel()` calls default to `durable_pool=False`. A saturated
  publisher pool will pack past `client_limit` instead of raising
  `ChannelNotAvailableError`. That was accepted deliberately: publisher
  checkouts are short-lived rather than held for the life of a stream,
  availability under a publish burst is strictly better than a hard refusal,
  and the publish path sets no consumer prefetch, so packing a publish
  channel carries no prefetch-memory hazard. The honest caveat is that a
  publisher-side pool has **no memory-safe cap upstream of it** — the cap
  that bounds the band lives in `event_consumer_service`, not in the
  library — so in a publisher the overflow band is genuinely unbounded, held
  in check only by how briefly each checkout is held. Set
  `OMNI_EVENTS_RABBITMQ_CHANNEL_OVERFLOW_ENABLED=false` to opt out and keep
  strict one-tier behavior everywhere. Timing is not immediate:
  `event_aggregation_service` pins `notification-common==6.3.2` as of
  2026-08-18 (re-verified at its `pyproject.toml:11`), so it inherits none of
  this on publication — it inherits it at its **next pin bump**.
- **Docstrings and library docs.** `ChannelPool`, `ChannelPoolConfig`,
  `DurableChannelPoolConfig`, the `client_limit` property, `get_channel()`,
  `_get_best_available_channel()`, and the `max_client_capacity` computation
  in `pool_stats()` all now document the two-tier contract and which side of
  it each pool is on. `docs/sdd.md` `## Key components` describes the
  acquisition tiers, the global-only wiring decision, and the durable
  hard-off, and `docs/engineering-best-practices.md` gains the caller-facing
  rule that a pooled channel is not exclusively yours on an overflow-enabled
  pool — do not close it, do not re-set QoS on it, and account for co-tenants
  on channel-scoped operations such as a queue delete — with the durable pool
  called out as exempt.
- No public class, function, or setting was removed or renamed. The new
  acquisition tier lives inside `ChannelPool`'s underscore-prefixed methods,
  and the new `_least_loaded_channel` helper is private; the only additions
  to the public surface are the settings field, the `overflow_enabled`
  config property, and the `pool_stats()` key.

### Version

- Bumped `6.5.0` -> `6.6.0`.

## [6.5.0]

Makes the scaler's scale-down path structurally conservative. Scale-down now
decides on the minimum available capacity observed over a trailing window
rather than one instantaneous reading, stays suppressed for a stabilization
period after any scale-up that actually executed, waits for starting pods to
settle, and can no longer be tricked into collapsing its deadband by a pod that
reports zero capacity. Scale-up is deliberately left un-smoothed, so an
under-capacity fleet still recovers on the instantaneous reading. Two defaults
also change, which moves a running deployment on upgrade: the replica floor and
the scale-down cooldown. Nothing public was removed or renamed.

### Added

- **`scaler_scale_down_window_seconds` (default `300`, seconds,
  `OMNI_EVENTS_SCALER_SCALE_DOWN_WINDOW_SECONDS`).** The trailing lookback over
  which the scale-down decision takes the *minimum* observed
  `total_available`, so only sustained excess capacity can give a pod back. A
  momentary trough in load no longer buys a pod deletion. The minimum is used
  rather than an average precisely because it is never above the instantaneous
  reading, so smoothing can only ever suppress a scale-down, never manufacture
  one. `0` disables smoothing and leaves the decision on the instantaneous
  reading, as an operator escape hatch.
- **`scaler_post_scale_up_stabilization_seconds` (default `600`, seconds,
  `OMNI_EVENTS_SCALER_POST_SCALE_UP_STABILIZATION_SECONDS`).** How long
  scale-down stays suppressed after a scale-up actually executed. This is a
  distinct guard from `scaler_scale_down_cooldown_seconds`, which keys off the
  last scale-*down* and therefore does not bind at all when a fleet enters a
  burst after a quiet period. It is also distinct from the trailing window: a
  scale-up steps `total_available` up by a full pod's capacity, and without
  this window that step would simply age in and permit the reversal of the very
  scale-up that produced it. The default is 2x the smoothing window for that
  reason. `0` disables it.
- Both new fields are validated non-negative by a `ScalerSettings` field
  validator. The "keep the stabilization window at or above the smoothing
  window" relationship is documented but deliberately *not* enforced by a
  cross-field validator, since a cross-field failure would raise at import time
  and crash the scaler rather than degrade it.

### Changed

- **`pod_capacity` is now the largest capacity reported by any healthy
  reporter** instead of whichever reporter happened to be first
  (`metrics[0].channels_total`). A pod that is terminating, restarting, or has
  not built its channel pool yet reports `channels_total=0`; as the first
  reporter it used to collapse the scale-down deadband
  (`total_available >= min_capacity + pod_capacity`) to no deadband at all.
  Taking the max also keeps the deadband at least as wide as the capacity a
  single deletion can actually cost during a rollout that changes per-pod
  capacity. If no reporter publishes a usable capacity, scale-down is refused
  outright with a warning rather than evaluated against an unknown deadband.
- **Scale-down is gated on `starting_pods == 0`**, symmetric with the existing
  scale-up gate. The moment a starting pod begins reporting, `total_available`
  jumps by a full pod-step, which could cross the deadband and immediately undo
  the scale-up that just completed.
- **Scale-down decides on the windowed minimum**, not the latest reading: the
  deadband comparison is made against the minimum `total_available` over
  `scaler_scale_down_window_seconds`, and scale-down is refused entirely while
  the window is not yet backed by a full window of observation. `pod_capacity`
  stays the current snapshot's, since what a deletion would cost is a property
  of the fleet as it is now. Iterations that produced no metrics record no
  sample, so a metrics outage cannot age into permission to scale down. When
  smoothing changes the outcome, the suppression is logged at INFO.
- **Scale-down is additionally suppressed for
  `scaler_post_scale_up_stabilization_seconds` after a scale-up executed.** The
  anchor is stamped only after the deployment scale actually succeeded, so a
  scale-up that was gated or cooled down does not hold off scale-down.
- **Scale-UP is deliberately left un-smoothed.** It continues to read the
  instantaneous available capacity, so under-capacity response stays fast —
  waiting a window to confirm an emergency would be waiting while clients are
  refused. (Kubernetes draws the same line: the HPA defaults its scale-up
  stabilization window to 0 and its scale-down window to 300 seconds.)
- **Scale-down-suppressing time state is anchored at process start** rather
  than at the epoch, so a scaler pod that restarts mid-incident (node drain,
  rollout, OOM, eviction) cannot read its guards as "long expired" and scale
  down on its first iteration. This covers `_last_scale_down_time`, the
  trailing capacity window, and the post-scale-up stabilization anchor.
  `_last_scale_up_time` is a deliberate exception and stays `0`: anchoring it
  would delay post-restart emergency scale-up by a full
  `scaler_scale_up_cooldown_seconds` and would make `_get_poll_interval` read a
  fresh process as having recently scaled up.
- **Default `scaler_min_replicas` moves `1` -> `3`.** On upgrade, a deployment
  that does not set `OMNI_EVENTS_SCALER_MIN_REPLICAS` sees its replica floor
  move from one pod to three. Sized for the 2026-08-11 sapi-bravo incident
  profile (violently bursty per-client fan-out across many concurrent clients,
  flapping in a 2-4 replica band): a fleet that has dug below 3 has to re-climb
  from cold when the next burst lands.
- **Default `scaler_scale_down_cooldown_seconds` moves `300` -> `1800`.** On
  upgrade, a deployment that does not set
  `OMNI_EVENTS_SCALER_SCALE_DOWN_COOLDOWN_SECONDS` sees its scale-down memory
  move from 5 minutes to 30 minutes. `scaler_scale_up_cooldown_seconds` is
  unchanged at 10 seconds — only the scale-down side is slowed.
- No public class, function, or setting was removed or renamed. The decision
  logic changed only inside `Autoscaler`'s underscore-prefixed methods; the
  signature changes to `_try_scale_down` and `_make_scaling_decision`, and the
  new `_record_available_capacity` helper, are all private.

### Version

- Bumped `6.4.0` -> `6.5.0`.

## [6.4.0]

Splits `RabbitMQConnector`'s single channel pool into two independently
configured pools — the existing non-durable pool plus a new durable /
DLQ-browse pool — so durable checkouts can no longer consume the channel
capacity that non-durable event streams share. Pool limits move from
process-global settings to per-pool configuration, and each pool now labels its
own log lines and reports its own statistics. Behavior at default settings is
unchanged for existing callers: `get_channel()` still serves the non-durable
pool, and `pool_stats()` keeps its exact shape.

### Added

- **`ChannelPoolConfig`.** The per-instance channel limits (`pool_size`,
  `min_size`, `client_limit`, `prefetch_count`, `wait_ms`) for a single
  `ChannelPool`, passed as the new optional `config` argument to
  `ChannelPool(...)` and readable back off the pool as `ChannelPool.config`.
  Any value left as `None` follows the process-global `rabbitmq_channel_*`
  setting, and that fallback is late-bound — resolved on each read rather than
  snapshotted at construction — so a pool built before the settings are
  adjusted still sees the adjustment. An explicit value always wins over the
  global, including an explicit `0`.
- **`DurableChannelPoolConfig`.** A `ChannelPoolConfig` whose unset values fall
  back to the durable `rabbitmq_durable_channel_*` settings instead of the
  non-durable `rabbitmq_channel_*` ones, with the same late-binding.
- **`ChannelPool(name=...)` and the read-only `ChannelPool.name` property.** An
  optional label the pool prefixes to every log line it emits, as
  `"[<name>] "`, so a process running more than one pool stays reconstructable
  from an incident timeline. An unnamed pool logs exactly the unprefixed
  messages it always has.
- **Dual-pool ownership in `RabbitMQConnector`.** The connector now creates and
  owns a durable `ChannelPool` alongside the non-durable one, both sharing its
  single connection. Acquisition from it is opt-in through the new
  `durable_pool=` keyword on `get_channel()` and `get_channel_direct()`, which
  defaults to `False` (the non-durable pool). `return_channel()` dispatches by
  ownership, so a caller never restates which pool it borrowed from.
- **`RabbitMQConnector.NON_DURABLE_POOL_NAME` and `DURABLE_POOL_NAME`.** The
  pool-name constants (`"non_durable"` and `"durable"`), used both as the
  log-line labels above and as the keys of `pool_stats_by_pool()`, so a stats
  key and an incident-timeline log line name the same pool the same way.
  Consumers should reference these constants rather than repeat the literals.
- **`RabbitMQConnector.pool_stats_by_pool()`.** Statistics for every pool the
  connector owns, keyed by pool name, each value that pool's own
  `ChannelPool.pool_stats()` dict — reported separately and never aggregated,
  omitting any pool that does not exist yet. `pool_stats()` is unchanged and is
  now explicitly the non-durable pool alone, so capacity surfaces that read it
  by key keep seeing non-durable capacity.
- **Five durable channel-pool settings on `CommonSettings`.**
  `rabbitmq_durable_channel_pool_size` (100),
  `rabbitmq_durable_channel_pool_min_size` (0, so the durable pool opens no
  channels until a durable checkout needs one),
  `rabbitmq_durable_channel_prefetch_count` (500),
  `rabbitmq_durable_channel_wait_ms` (500), and
  `rabbitmq_durable_channel_client_limit` (1, so durable checkouts stay
  exclusive).

### Version

- Bumped `6.3.2` -> `6.4.0`.

## [6.3.0]

Adds a source-available publishing pipeline that mirrors the library's source to
a public GitHub component repository, so external consumers can build
`notification-common` from source. The existing internal-PyPI publish is
retained unchanged, so the library is now dual-published.

### Added

- **`BUILDING.md`.** A build-from-source guide for the mirrored tree: `uv sync`
  against public PyPI, how the pre-generated `nvidia/` protobuf stubs ship, and
  the note that internal consumers keep depending on the internal-PyPI package.

### Changed

- **Relicensed `notification-common` to the Apache License 2.0.** The
  source-available library ships under Apache 2.0 so external consumers can
  build and use it under a permissive license.
  - Added the root [`LICENSE`](LICENSE) file carrying the full Apache License
    2.0 text.
  - Swapped every published NVIDIA-authored source file from the proprietary
    SPDX header to the 2-statement `SPDX-License-Identifier: Apache-2.0`
    header; `uv run nc-lint` now enforces the header so the Apache posture
    cannot silently regress.
  - Documented a closed-contributions posture in `CONTRIBUTING.md` (the
    project is not currently accepting contributions).
  - Swapped the `oss/OSS_LICENSE.txt` reference copy from the proprietary
    NVIDIA Software License Agreement to the Apache License 2.0 text (matching
    the root `LICENSE`), and swapped `oss/OSS_LICENSE_HEADER.txt`'s SPDX header
    template to the Apache form.
  - `generate_openapi_yaml` now stamps the generated `openapi.yaml` for
    consuming services with the Apache-2.0 SPDX header instead of the former
    proprietary header.

### Packaging

- **`github-publish` GitLab CI job.** A new trailing `github-publish` stage
  mirrors the source to a per-component GitHub repository as a squashed
  source-release commit, running after the internal publish and tag so the
  upstream-first ordering is observable.
- **`.publishignore` taxonomy.** A gitignore-syntax allowlist of what reaches
  the mirror: everything is published except internal CI/build infrastructure
  (`.gitlab-ci.yml`), the publish tooling under `/scripts/`, internal-infra
  artifacts (`uv.lock`, `buf.gen.yaml`), internal-only docs and agent config
  (`CLAUDE.md`, `docs/`, migration tracking), editor/IDE tooling, generated
  coverage/report output, and local overrides/secrets.
- **Prepare-script with a leakage guard.** `scripts/prepare-github-publish.sh`
  rewrites the published `pyproject.toml` to strip the two internal
  `[[tool.uv.index]]` registry entries (so the mirror resolves from public PyPI),
  then runs a fail-closed guard that scans the filtered snapshot twice for
  internal-registry and internal-API hostnames and for secret files, aborting the
  publish rather than shipping a leak.
- **Public-PyPI dry-run.** The `github:dry-run-resolve` job reconstructs the same
  filtered-and-transformed snapshot and runs `uv lock` against public PyPI with
  no internal index configured, proving every third-party dependency is
  reachable from public PyPI.
- **Mirror auto-tagging.** A published `tag-release.yml` GitHub Actions workflow
  runs on the mirror and, on push to `main`, reads `project.version` from
  `pyproject.toml` and idempotently creates and pushes the bare `X.Y.Z` git tag
  (via `GITHUB_TOKEN`) so consumers can git-dep the component by tag.
- The internal-PyPI `build_and_publish` job is unchanged; the library remains
  published to NVIDIA's internal PyPI for internal consumers (dual-publish).

### Version

- Bumped `6.2.0` -> `6.3.0`.

## [6.2.0]

Adds shared support for the v2 Discovery `CapabilitiesService`, so services
built on `NotificationServer` can advertise their gRPC and REST APIs to the
Discovery Kubernetes operator.

### Added

- **Discovery CapabilitiesService support.** Generated stubs for
  `nvidia.omniverse.discovery.capabilities.v2alpha` ship in the wheel under a
  new top-level `nvidia/` namespace-package tree, alongside a
  `CapabilitiesServicer` that serves a single static `ServiceEntry`.
- **`NotificationServer.capabilities_service_entry()` hook.** An overridable
  classmethod (default `None`) through which a service supplies its
  `ServiceEntry`. When it returns an entry and the new
  `grpc_capabilities_enabled` flag is on, `NotificationServer` registers the
  `ListServices` RPC, adds the service to reflection, and marks it SERVING in
  health.
- **`NotificationServer.build_capabilities_entry()` helper.** An optional
  classmethod a `capabilities_service_entry()` override may call to build its
  `ServiceEntry`, auto-deriving the `grpc_services` portion from the server's
  own exposed gRPC service name(s) so a service supplies only the discovery
  type key (`service_name`) and its REST resources/versions.
- **`grpc_capabilities_enabled` setting** (env
  `OMNI_EVENTS_GRPC_CAPABILITIES_ENABLED`, default `true`) on `CommonSettings`,
  alongside `grpc_health_checking_enabled` / `grpc_reflection_enabled`.
- **`update_proto` script** (`uv run update_proto`) that regenerates the
  Discovery stubs from the published buf module via `buf.gen.yaml`.

### Changed

- The `ListServices` RPC is exempted from authentication and metrics/tracing
  instrumentation (added to `EXCLUDE_METHODS` and the tracing exclude filter),
  matching how health and reflection are treated. The exemption is
  unauthenticated by design so the discovery scan can reach it.

### Packaging

- Added an explicit `[tool.hatch.build.targets.wheel]` `packages` list
  (`notification_common`, `nvidia`) now that a second top-level tree ships.
- Bumped the grpcio floors to `>=1.78.0` (and added a direct `grpcio` dependency)
  to match the generated Discovery stubs, which hardcode
  `GRPC_GENERATED_VERSION = '1.78.0'` and raise at import when an older grpcio is
  installed.

### Version

- Bumped `6.1.1` -> `6.2.0`.

## [6.1.1]

Follow-up to 6.1.0 that refines how the independent observability signals gate
log export.

### Changed

- Log export now follows "any signal on" rather than tracing specifically, so
  metrics-only deployments keep shipping logs to the OTLP collector.
  Trace/span-id correlation in those logs still requires tracing.
- Collapsed the duplicated `tracing_enabled` guard in `configure_observability`
  into a single block.

### Version

- Bumped `6.1.0` -> `6.1.1`.

## [6.1.0]

Splits the single `telemetry_enabled` master switch into independent metrics and
tracing signals.

### Added

- **`metrics_enabled` and `tracing_enabled` settings** so the two OTEL signals
  can be enabled independently instead of being gated together by
  `telemetry_enabled`. Both fall back to `telemetry_enabled` when unset, so
  deployments that only set `OMNI_EVENTS_TELEMETRY_ENABLED` behave identically.

### Changed

- `configure_observability`, the gRPC interceptors, the REST tracing
  middleware, the logging trace-id format, and the tracing decorators now gate
  on the appropriate per-signal flag. Log export and dependency instrumentation
  (aio_pika spans, log/trace-id correlation) follow tracing; there is no
  independent logs signal.

### Version

- Bumped `6.0.0` -> `6.1.0`.

## [6.0.0]

Major release. The authentication and permissions configuration model was
reworked. The `authorization_enabled` -> `permissions_enabled` rename (both the
Python field and its environment variable) is a **breaking change** for
downstream consumers, which is why this is a major version bump.

### Breaking

- **Renamed `authorization_enabled` -> `permissions_enabled`.** The
  `CommonSettings` field and its environment variable both changed:
  `OMNI_EVENTS_AUTHORIZATION_ENABLED` -> `OMNI_EVENTS_PERMISSIONS_ENABLED`.
  Downstream deployments that set the old environment variable must update it;
  the old name is no longer read. Default remains `false`.
- **Removed the permissions endpoint-presence short-circuit.** Permission
  checks are now gated solely on the explicit `permissions_enabled` flag.
  Previously the client could short-circuit based on whether a permissions
  endpoint value was set; that implicit behavior is gone, so a deployment that
  relied on "no endpoint configured" to skip permission checks must now set
  `permissions_enabled=false` explicitly.

### Added

- **`principal_source` setting** (env `OMNI_EVENTS_PRINCIPAL_SOURCE`) on
  `AuthSettings` — the single first-class gate for authentication behavior.
  Values: `disabled | jwtUnverified | jwt | opaque | userInfoService`. Default
  is `jwtUnverified`. `authentication_enabled()` is now defined as
  `principal_source != disabled`.
- **`user_info_service_uri` setting** (env `OMNI_EVENTS_USER_INFO_SERVICE_URI`)
  on `AuthSettings`, consulted only when `principal_source` is
  `userInfoService`.
- **No-verify JWT decoder** (`auth/auth_tokens/jwt_unverified_decoder.py`)
  backing the `jwtUnverified` mode. It decodes the JWT payload to extract the
  principal id but performs no signature, audience, or expiry verification —
  token validity is delegated to a trusted upstream component (e.g. Envoy).
- **REST user-info-service client**
  (`auth/auth_tokens/user_info_service_decoder.py`) backing the
  `userInfoService` mode. It resolves a bearer token to a principal id by
  calling `GET {user_info_service_uri}/api/v1alpha/users/me` with the forwarded
  token, with retries on transient faults (5xx/429/transport), a 64 KiB bounded
  response body, and scrubbed errors that never echo token material, URLs, or
  response bodies.

### Changed

- **Reworked `validate_token_settings`** on `AuthSettings` with per-mode
  validity rules:
  - `permissions_enabled=true` together with `principal_source=disabled` is
    rejected (permissions require authentication).
  - `jwt` requires both `openid_configuration_uri` and `jwt_audiences`.
  - `opaque` requires `openid_configuration_uri`.
  - `userInfoService` requires `user_info_service_uri`.
  - `jwtUnverified` and `disabled` impose no additional requirements.
- `openid_enabled` is retained as an infra-only field; it is no longer read by
  `authentication_enabled()` or the validator.
- `token_verification_type` is retained as the OIDC datum distinguishing the
  `jwt` vs `opaque` decoders; it no longer independently gates behavior.

### Packaging

- Added this root `CHANGELOG.md`. `scripts/build_archive.py` already bundles
  `CHANGELOG.md` into the documentation archive and warned "file not found"
  when it was absent; creating it both records this change and silences that
  warning.

### Version

- Bumped `5.4.4` -> `6.0.0`.
