# Changelog

All notable user-facing changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/), and this project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Add new entries under the planned version read from the package manifest, in the matching `Added`, `Changed`,
`Removed`, `Fixed`, or `Security` section. Create the section if needed. Do not add release dates.

## [0.0.9]

### Added

- Add optional API access logs for REST and gRPC (`ACCESS_LOG_ENABLED`, `ACCESS_LOG_HEADERS`), recording all service access — including requests rejected before a handler (authentication failures, unknown routes and methods, malformed requests) — while excluding infrastructure and discovery endpoints.
- Forward configured incoming request headers to the Permission Service (`observability.headers.forward`).

### Changed

- Rename the `LOG_STRUCTURED` environment variable to `STRUCTURED_LOGGING`.
- Pull the distroless runtime base image from `nvcr.io/nvidia/distroless/cc:v4.1.4`, which NGC serves anonymously, in the service, smoke-test, and conformance-test images alike, so the published source builds on the same image that is built and scanned here. The smoke-test and conformance-test images move from `v4.1.2` to `v4.1.4` with it.
- Default the conformance-test chart's `conformanceTest.restUrl` and `conformanceTest.grpcUrl` to the in-cluster `userinfo-service` REST and gRPC addresses.
- Set default resource requests and limits for the conformance-test Job.
- Mount the smoke-test and conformance-test access-token volume in memory at the directory of `serviceIdentity.accessTokenFile.path` (now configurable in the conformance-test chart).
- Regenerate `ATTRIBUTIONS.md` from the Linux production dependency set of the shipped service, smoke-test, and conformance-test binaries.

### Fixed

- Include the rendered API reference in the published `userinfo-api` NGC resource.
- Apply the conformance-test chart's `podAnnotations` to the test pod instead of the Job object.
- Correct the operator documentation of reader readiness, readiness probe behavior, and Helm value defaults.
- Remove implementation-specific requirements (local replica, cache design, notification-service integration) from the API specification and add the required `username` field to its User examples.

### Security

- Run the conformance-test Job with a hardened security context by default (non-root, read-only root filesystem, no privilege escalation, all capabilities dropped, runtime-default seccomp profile).

## [0.0.8]

### Fixed

- Register the v1alpha gRPC reflection service so v1alpha-only clients can discover the CapabilitiesService.

## [0.0.7]

### Added

- Add a `helm test` smoke-test hook that verifies a deployed release end-to-end over REST and gRPC.
- Add a User Info Service conformance test crate and Helm chart for validating REST and gRPC contract compliance.
- Split the service into separate writer and reader deployment modes: writers synchronize the directory to PostgreSQL; readers serve the read APIs and scale horizontally.
- Add a static directory overlay (`STATIC_DIRECTORY_FILE`) for injecting supplemental users, groups, and memberships at read time.
- Restrict the cached directory to an enterprise application's app-role assignments (`APP_ROLE_ASSIGNMENTS_*`).
- Add rotating PostgreSQL password support from a token file with connection pool swap.
- Add Helm support for provisioner sidecar containers that inject service credentials and database tokens.
- Implement structured JSON logging (`LOG_STRUCTURED`).
- Add PostgreSQL TLS connection support (`POSTGRES_SSL_MODE`, default `prefer`).
- Default the Permission Service URL in Helm values.
- Add CapabilitiesService ListServices for dynamic API discovery

### Changed

- Update distroless base images to 3.2.10.
- Forward the caller's bearer token to the Permission Service for authorization checks instead of using a dedicated service identity.
- Switch OTLP trace and metric export from HTTP to gRPC.
- Nest the OTLP endpoint under `observability.http` in Helm values.
- Update default Kubernetes resource requests and limits.
- Set default Helm `nameOverride` and `service.nameOverride`.
- Rename reader and writer container images in Helm.

### Removed

- Drop the dedicated service identity token used for Permission Service authorization checks.
- Remove the unused `resources` section from Helm `values.yaml`.

### Fixed

- Wait for the PostgreSQL token file to appear at startup before failing when a provisioner sidecar is used.

### Security

- Escape OData `$filter` identifiers and `$search` display-name terms to prevent Microsoft Graph query injection.
- Percent-encode dynamic path segments in Microsoft Graph URLs.
- Sanitize client-visible API error messages so upstream diagnostics are not leaked in responses.
- Harden pod and container security context defaults in Helm (`runAsNonRoot`, `readOnlyRootFilesystem`, drop all capabilities, seccomp).
- Use SHA-256 fingerprints for permission-cache keys instead of a truncated hash.
- Build container images with `cargo build --locked` for reproducible dependency pins.

## [0.0.6]

- Redirect requests with trailing slashes.
- Update gRPC response type names to address "buf lint" warnings.

## [0.0.5]

- Update default features used from OTLP.

## [0.0.4]

- Rename the service to "microsoft-graph-userinfo-service"
- Add an init container to run an initial async

## [0.0.3]

- Use cursor-based pagination
- Add locking mechanism for multi-replica full syncs
- Add OpenTelemetry metrics
- Add distributed tracing

## [0.0.2]

- Use normalized Postgres tables instead of a binary snapshot
- Cache group membership in Postgres

## [0.0.1]

- Initial release