# Changelog

All notable user-facing changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/), and this project adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Add new entries under the planned version read from the package manifest, in the matching `Added`, `Changed`,
`Removed`, `Fixed`, or `Security` section. Create the section if needed. Do not add release dates.

## [0.0.54]

### Added

- Add `serviceIdentity.accessTokenFile.path` to the conformance-test Helm chart.
- Add a references page to the API documentation.
- Add the `openId.requiredJwtClaims` Helm value (`JWT_REQUIRED_CLAIMS`) for requiring token claims to satisfy rules beyond `aud` and `iss`, with the `equals`, `notEquals`, `in`, `notIn`, `contains`, `containsAny`, `containsAll`, and `exists` operators — for example, to accept only tokens issued to approved client applications. Tokens that fail a rule are rejected with `401` and recorded in the invalid-token cache.
- Add the `openId.allowedJwtAudience` Helm value, the only source of accepted `aud` values (`JWT_AUDIENCE`). It defaults to `[]` and must be set when `openId.enabled` is true and `openId.tokenVerificationType` is `jwt` or `jwtuserinfo`; otherwise the service does not start.

### Changed

- Build from source and build all three images without NVIDIA-internal access: `axum-observability` and `otel-tools` are vendored under `vendor/` as path dependencies (now at 0.14.0, previously 0.12.0 from an internal registry), the Dockerfiles take this directory as the build context and no longer copy `.cargo/config.toml`, and the runtime base is `nvcr.io/nvidia/distroless/cc:v4.1.4`.
- Build the smoke-test and conformance-test images from the locked dependency set on distroless `cc:v4.1.4` and ship `ATTRIBUTIONS.md` in both images.
- Default the conformance-test chart's `conformanceTest.restUrl` and `conformanceTest.grpcUrl` to the in-cluster `permission-service` Service and set CPU and memory requests and limits for the conformance-test container.
- Mount the access-token exchange volume at the directory of `serviceIdentity.accessTokenFile.path` instead of a fixed path.
- Merge `smokeTest.podLabels` with the shared `podLabels` instead of replacing them.
- Regenerate `ATTRIBUTIONS.md` from the current production dependency set.
- Raise the minimum supported Rust version of the workspace crates, including the published `permission-core` and `permission-auth`, from 1.84 to 1.88.

### Removed

- Remove the top-level `tags` list from the OpenAPI document (`/openapi.json`). Its only entry was an internal tracking ID with a link to an internal page.
- Remove the unused `autoscaling.minReplicas`, `autoscaling.maxReplicas`, and `autoscaling.targetCPUUtilizationPercentage` Helm values.
- Remove the chart-created Postgres credentials Secret and the `database.postgres.password`, `secret.create`, and `secret.name` Helm values. Create the Secret before deploying, with the `POSTGRES_PASSWORD` and `POSTGRES_USER` keys, and reference it with the new `database.postgres.secretRef.name` value (default `permission-service-secret`).
- Remove the `openId.clientRegistrations` and `openId.additionalJwtAudience` Helm values, including the placeholder `some_client_id` audience. Move every accepted audience to `openId.allowedJwtAudience`.

### Fixed

- Fix the name of the chart-created Postgres credentials Secret so the Deployment mounts it when `secret.create` is enabled.
- Apply the conformance-test chart's `podAnnotations` to the pod instead of the Job.
- Correct the service documentation to match the service's actual behavior: policy evaluation order and caching, rejected-token caching and unauthenticated endpoints, policy scopes and validation, log levels, notification troubleshooting, and gRPC access-log encoding. Settings are now named by their Helm values.
- Start the service with `opaque` token verification (`AUTH_TYPE=opaque`) without `JWT_AUDIENCE`. Opaque tokens are verified through the identity provider's `userinfo` endpoint and have no `aud` check, so an audience is now required only for `jwt` and `jwtuserinfo`.
- Accept `jwtuserinfo` for `openId.tokenVerificationType` (`AUTH_TYPE`). The service rejected the documented value at startup and accepted only `jwt-with-user-info`, which remains supported as an alias.

### Security

- Run the conformance-test container as non-root with the `RuntimeDefault` seccomp profile, a read-only root filesystem, no privilege escalation, and all capabilities dropped.
- Reject JWTs that omit the `aud` claim or carry one that is neither a string nor a list of strings, and, when `openId.allowedJwtIssuers` is set, JWTs that omit `iss`. Also reject JWTs whose `nbf` (not-before) time is still in the future beyond `config.jwtLeeway`. Previously such tokens bypassed the audience and issuer allow-lists. Identity providers that issue tokens without `aud` must be configured to include it, otherwise callers receive `401`.
- Resolve `.` and `..` path segments, including percent-encoded forms such as `%2e%2e`, before applying `config.urlRemapper` rewrite rules. Previously an id such as `omniverse://host/NVIDIA/../Projects/scene.usd` was rewritten by the rule for `omniverse://host/NVIDIA/` and evaluated against another location's policies.
- Update dependencies with published security advisories: `h2` 0.4.19, `tokio-postgres` 0.7.18 (`postgres-protocol` 0.6.12), `time` 0.3.55, `ring` 0.17.14, `crossbeam-channel` 0.5.17, `crossbeam-epoch` 0.9.21, `rand` 0.9.5, `event-listener` 5.4.2, and `anyhow` 1.0.104.
- Replace the archived and unsound `serde_yml` YAML parser with `serde_norway` for reading policy, metadata, and URL remapper configuration files.

## [0.0.53]

### Added

- Add API access logs for REST and gRPC service access. One structured access record (or several when an endpoint unwinds a batch) is emitted per call with the operation, selected request parameters, and the response status and outcome. Requests rejected before a handler runs — authentication and authorization failures, unknown routes and methods, and malformed payloads — are also logged as call-level records (with the raw requested operation and no request parameters) so invalid access is auditable; infrastructure and discovery endpoints (health checks, schema/discovery document and UI, and gRPC health, reflection, and capability discovery) are never logged. Disabled by default; enable with `observability.logging.access.enabled` / `ACCESS_LOG_ENABLED` and configure `observability.logging.access.headers`.

### Changed

- Resolve meta-permission hierarchy via globally-unique resource types
- Reduce metric time-series cardinality: `authorization.request.allowed` / `authorization.request.denied` are now attributed only by the bounded `service` (plus `deny_reason` on denials) instead of per-request `principal`, `action`, and `resource`; the REST request metrics drop the unbounded `principal` attribute.

### Removed

- Remove the custom `http.server.request.duration` (ms) histogram; HTTP request latency is now provided solely by the `axum-observability` layer's `http.server.request.duration` (seconds), eliminating a duplicate metric with a conflicting unit.

## [0.0.52]

### Added

- Publish an `omni.permissions.meta.changed` notification whenever service metadata (a service, its actions, or its resource types) is changed or removed through the `/v1beta/services/` endpoints.
- Add headless Service for client-side gRPC load balancing.
- Add metrics for database connections, latency and errors.
- Add observability metrics for cache hit/miss counters, policy CRUD outcomes, categorized deny reasons, build info, in-flight HTTP and gRPC concurrency, policy evaluation depth and fetch size, and notification publish and consume outcomes.
- Allow to set ACCESS_TOKEN_FILE_PATH from Helm values.
- Add `observability.metrics.enabled` and `observability.tracing.enabled` Helm toggles.
- Add order filter to GET /policies API.
- Add *POST* `/v1beta/diagnostics/authorize/batch/` endpoint for batched policy listing diagnostics.
- Add Postgres token-file auth with connection pool rotation.
- Add resource hierarchy scopes for policies (`resource in <Type>::"<url>/"`).
- Reject deletion of system policies via API.
- Support cross-replica cache invalidation via change events.
- Register `event-consumer-service:inspect-and-replay-all-dlq` in the default metadata seed.
- Add PolicySet cache to avoid reparsing Cedar policy text
- Add `config.parallelEvaluationThreshold` to control when authorization requests evaluate resources across multiple CPU cores instead of sequentially.
- Add CapabilitiesService for discovery to find permission API dynamically

### Changed

- Split the `permissions:meta` meta-permission into `permissions:meta:read` (gating the metadata `GET` endpoints) and `permissions:meta:write` (gating the metadata `PUT`/`DELETE` endpoints). **Breaking:** seed policies granting `permissions:meta` must be updated to grant the two new actions.
- Support resource prefix queries with pattern-ops index range scan.
- Add first-class Cedar Group scopes for policies.
- Make meta-permission checks respect policy order and priority.
- Pass the changed policy as the resource for `permissions:edit` authorization checks.
- Precompute evaluation cache key fingerprint to avoid per-lookup hashing.
- Resolve request principal once per service in batch authorization.
- Move reusable gRPC interceptors into permission-auth crate.
- Split `JWT_AUDIENCE` and `JWT_ISSUERS` on comma delimiter when specifying multiple values.
- Rename HTTP metrics to use the `http.server` prefix. **Breaking:** existing dashboards and alerts referencing the old metric names must be updated.
- Wait for the Postgres credential file at startup before failing.
- Mount the Postgres secret volume only when password-file auth is used.
- Read `POSTGRES_USER` from a Helm value with secret fallback.
- Update the base distroless container to 3.2.10.
- Document resource hierarchy policies and the `parentType` metadata field.
- Log client token validation failures at debug instead of error.
- Move cache-size metrics off the request hot path to observable gauges; hit/miss counters remain in metrics.rs
- Skip resource hierarchy build for non-prefix policy evaluation
- Demote hot-path tracing spans to debug in permission-auth, permission-core, HTTP auth middleware, and authorization
- Index batch policies per resource to prune hierarchy evaluations
- Parallelize authorize_batch Cedar evaluation across resources

## [0.0.51]

### Added

- Implement RED metrics for gRPC.
- Add firewall-style order field to policies for controlling evaluation order.
- Support provisioner sidecar containers for getting service credentials.
- Add *POST* `/diagnostics/authorize` endpoint for debugging policy evaluation.
- Deduplicate Cedar policies added to the database.
- Add batched SQL insert for submitted policies
- Add optional comment field to policies for administrator metadata

### Changed

- Switch structured logging from Bunyan to `tracing-subscriber`'s built-in JSON formatter.

### Removed

- Remove `FMT_SPAN` configuration variable / `observability.logging.fmtSpan` Helm value.

### Fixed

- Restart service pods with Helm when managed secrets change
- Fixed `get_policies` and `get_policies_batched` returning scoped policies when scope is not specified
- Allow single quotes in resource ID validation

### Security

- Add TLS connector for Postgres.
- Resolve principal IDs in target service space before view check.

## [0.0.50]

### Fixed

- Remove commonName from CertManager's Certificate Request.
- Restart service pods with Helm when ConfigMap changes.

## [0.0.49]

### Added

- Added default Event Aggregation Service endpoint.
- Added new Helm values for specifying the service identity (client credentials flow).

### Changed

- Unified timeouts config for HTTPProxy in the Helm chart.

## [0.0.48]

### Fixed

- Fixed an issue where service accounts tokens that did not contain a custom user identifier could not authenticate.

## [0.0.47]

### Added

- Add `openId.principalIdClaim` field for configuring what claim is used as the principal identifier by default.
- Infer scopes for system policies automatically.
- Add `created_by` and `created_at` fields for policies.

### Changed

- Move logging-related Helm values into `observability.logging` section.
- Update *PUT* `/v1beta/policies/` and *PUT* `/v1beta/policies/batch/` APIs to use policy scope inference.

### Fixed

- Fixed an issue where traces were initialized incorrectly.

## [0.0.46]

### Added

- Add ingress support via httpProxy.

### Changed

- Update the base distroless container to 3.2.3.

## [0.0.45]

### Fixed

- Add serve annotations for AuthType values in permission-auth crate.

## [0.0.44]

### Changed

- Update default image repository to nvcr.io/nvidia/omniverse.

## [0.0.43]

### Added

- Support specifying OpenID configuration URL instead of separate settings for jwks_uri, userinfo_endpoint and
  token_endpoint.

## [0.0.42]

### Added

- Support passing multiple values for JWT audience

## [0.0.41]

### Changed

- Use canonical names for kubernetes services.

### Fixed

- Fixed incorrect invalidation of authorization results for policies with a specified principal.

## [0.0.40]

### Added

- Introduced explicit `ENABLE_NOTIFICATIONS` and `ENABLE_CLIENT_CREDENTIALS` environment variables to control event
  publishing.
- Added `extraVars` Helm value for specifying custom environment variables for the deployment.
- Added `secrets.create` Helm value to specify if Kubernetes secret needs to be created automatically during deployment.

### Changed

- Moved all Tonic-related Helm values into the `grpc` section.
- Moved all Helm values for event publishing into the `notifications` section.
- Update nSpect ID for OpenAPI specification.

### Fixed

- Fixed an issue where authentication has been required for health endpoints

## [0.0.39]

### Added

- Added API documentation.
- Added governance pipelines for gRPC and REST.
- Publish OpenAPI and gRPC specifications on NGC.

## [0.0.38]

### Fixed

- Add system attributes for principal and resources for evaluation.

### Security

- Update base distroless container to v3.1.13.

## [0.0.37]

### Changed

- Update gRPC spec version from v1 to v1beta.
- Update REST API version from v1 to v1beta.
- Update gRPC spec version for Notification Service to v1beta.

## [0.0.36]

### Fixed

- Fixed an issue where multiple HTTP requests could be sent to the IdP to retrieve JWK sets.
- Add additional logs for validating JWTs and retrieving public keys from the IdP.
- Add /v1/idp/keys endpoint to return the currently used public keys.

## [0.0.32]

### Added

- Support searching global policies.

### Changed

- Use clap for parsing environment variables.
- Use percent-encoding for resource IDs specified as URLs.

## [0.0.31]

### Added

- Publish events about policy updates.
- Add `service` and `action` fields into AuthorizationResponse in REST.
- Add POSTGRES_PORT env var for specifying the port for connecting to Postgres.

## [0.0.30]

### Fixed

- Wrap `auth.systemPoliciesJson` helm value into single quotes.

## [0.0.29]

### Added

- Add authorization for GET /v1/policies/ and GET /v1/policies/:id.

### Changed

- Restructure how system policies are configured and stored in the database.
- Allow to use wildcards for actions in the policy search.
- Use strict policy search for GET /v1/policies.

### Fixed

- Fixed incorrect system policy deletion after restart.

## [0.0.28]

### Added

- Add request attribute for authorization traces.

### Changed

- Update grafana dashboard.
- Update service dependencies.

### Fixed

- Fixed incorrect name for evaluation cache size metric.

## [0.0.27]

### Added

- Add caching for policy evaluation.

### Removed

- Remove page_count for /v1/policies endpoint.

## [0.0.26]

### Changed

- Update metadata API to allow registering actions and querying known services and resource types.

### Removed

- Remove policies_principal_idx index.

## [0.0.25]

### Changed

- Revise CI to use better rust image.

## [0.0.24]

### Added

- Include additional HTTP methods to CORS headers.

### Changed

- Include `page_count` for GET /v1/policies/.
- Include CORS headers for debug builds.
- Return Cache-Control headers from the Permission Service.
- Update DOCKER_REPO in .gitlab-ci.yml with the new GitLab URL.

### Removed

- Remove system policies from paginated responses.

### Fixed

- Fixed an issue where reason was not returned for ResourceActionDecision in gRPC.

## [0.0.23]

### Changed

- Update the rust-builder tag.

### Fixed

- Don't throw an error if the token doesn't have a custom ID claim.

## [0.0.22]

### Added

- Add query parameters for filtering results from GET /v1/policies/.

### Changed

- Use a dedicated runtime for Postgres transactions.
- Simplify Cedar entities parsing.
- Update grafana dashboard with new Requests per user visualization.

## [0.0.21]

### Added

- Add in-memory cache for metadata.
- grpc: adding opentelemetry tracing interceptor.
- Add more instrumentation for Cedar policy evaluation.
- Add principal for request metrics.

## [0.0.20]

### Fixed

- Fixed an issue where HTTP health endpoint required authentication.

## [0.0.19]

### Added

- Add CI job for publishing OpenAPI Specification to production.
- Publish OpenAPI Specification to readme.com.
- Generate openapi.json.
- Add buf lint and buf format to the pipeline.
- Upload proto files to the buf registry.

### Fixed

- Fixed a bug where /meta/principals raised HTTP500 if more than one resource meta was added for the same service.

## [0.0.18]

### Added

- Add distributed traces.
- Use TraceContextPropagator for distributed traces.

### Changed

- Enable auto token cache expiration.
- Include grafana dashboard to the helm chart.

### Removed

- Revert "Add axum-tracing-opentelemetry for distributed traces".

## [0.0.17]

### Fixed

- Fixed incorrect default value for ENABLE_DENY_REASON in the Helm deployment.yaml.

## [0.0.16]

### Added

- Add gRPC implementation.

### Changed

- Add TRACE_SAMPLING_RATIO for sampling traces.

### Removed

- Remove buf generate command from crate:publish:artifactory job.
- Removes auth plugin files and references.

## [0.0.15]

### Added

- Specify `service.instance.id` resource attribute for traces and metrics.

### Removed

- Remove hardcoded value for worker_threads.

### Fixed

- Cargo publish fix.

## [0.0.14]

### Added

- Add caching for invalid tokens.
- Add instrumentation for mutex lock in get_cached_principal.

### Changed

- Use a lock-free cache for auth tokens.
- Rename `text` field in `PolicyResponse` to `policy` to be consistent with POST requests.

## [0.0.13]

### Removed

- Remove tokens from permission-auth traces.

### Fixed

- Fix POSTGRES_CONNECTIONS env var not being utilized.

## [0.0.12]

### Changed

- Clean up docker and CI scripts.
- Make AuthPrincipal and token cache private for permission-auth crate.
- Isolates permission-auth crate and publishes to Artifactory.

### Fixed

- Fixed a bug where tokens were cached without expiration.
- Fixed the middleware order to capture http errors from the auth middleware.

## [0.0.11]

### Added

- Add USERINFO_URI to README.md.

### Changed

- Updates auth error codes to ensure 401 for standard error cases.

### Fixed

- Fix copying build artifacts to the final Docker layer.

## [0.0.10]

### Added

- Add AUTH_TYPE env var to the Helm chart.
- Use Buf to manage protobuf imports.

### Changed

- Allow running the pipeline on release branches.
- Introduce SERVICE_NAME and SERVICE_SCOPE environment variables for observability.
- Introduce ENABLE_DENY_REASON env var to toggle returning the "reason" field for denied requests.

### Fixed

- Fixed a bug where /v1/authorization/batch/ could not find any policies if resource is not specified.
- Fix CI pipeline to install correct version of Rust toolchain.

## [0.0.9]

### Added

- Implement ext_authz plugin for Envoy using permission-auth crate for verifying user tokens.

### Changed

- replaces local crate with tonic-observability dependency, cargo fmt.
- Change the private registry to omniverse/sevan-cesspool.

### Removed

- Remove hierarchical evaluation in favor of /authorization/batch with conditions.

### Fixed

- quick fix for envoy dockerfile.

## [0.0.8]

### Added

- Add principal meta for customizing what claim should be used as user ID.

### Changed

- Enforce app version during GitLab CI tag publishing.

## [0.0.7]

### Added

- Add versioning prefix for all API calls.

### Changed

- Restructure http request and initialize service spans.
- Return response body from authorization middleware.

### Fixed

- Fixed displaying secret config information on startup.
- Fixed missing traces for auth middleware.
- Add additional debug logging for JWT verification.
- Fixed an issue where empty env vars didn't return an error.

## [0.0.6]

### Added

- Add new environment variables to Dockerfile.

### Changed

- Add tests for hierarchical evaluation strategy.
- Document POSTGRES_DBNAME and POSTGRES_CONNECTIONS.
- Use separate endpoints for traces and metrics.

### Removed

- Remove PersistentVolumeClaim from the helm chart.

### Fixed

- Address clippy issues in authorization.rs.

## [0.0.5]

### Added

- Add deadpool-postgres for Postgres connection pool.
- Add EvaluationPriority to opt out "forbid over permit" rule.
- Add FMT_SPAN env var for logging span events.
- Retrieve all policies for resource hierarchy in one query.
- Implement hierarchical policy evaluation.
- Add endpoints for managing resource metadata.
- Move endpoints to separate modules.
- Add meta permissions for authorization actions.

### Changed

- Replace ADMIN_GROUP env var with DEFAULT_POLICY.
- Use PolicyStorage trait to abstract database operations.
- Fix incorrect volume name in values.yaml.
- Add metadata permissions for actions that update policies.
- Publish containers and Helm charts to omniverse/cesspool.

### Removed

- Remove opentelemetry-stdout.
- Remove POSTGRES_PASSWORD_FILE env from Dockerfile.

### Fixed

- Fixed default env values in Dockerfile.
- fixed example run command.
- Fixing run command example.

## [0.0.4]

### Added

- Support patterns in `resource.id` field.

### Changed

- Fixed traces ignoring EnvFilter.
- Integrate RED metrics.
- Add "summary" field for batch authorization queries.
- Add "Multiple resources for multiple actions" example to Postman collection.
- Include "id" field as an available resource attribute.

### Fixed

- Fixed unit tests checking that saved policies are trimmed.

## [0.0.3]

### Added

- Add tracing.
- Implement OpenID Connect.
- Add Helm chart.
- Add policy validation for PUT /policies/batch/.

### Changed

- Change /authorization/ method to POST.
- Add /policies/batch/ API for sending multiple policies at once.
- Add POLICY_VALIDATION env var to toggle Cedar policy validation.

## [0.0.2]

### Added

- Add docker:publish job.
- Add docker:build job.
- Add Dockerfile.
- Add GitLab pipeline.
- Add tests for permission-http crate.
- Add GET /authorization/batch/ API.
- Add GET /policies/ API.

### Changed

- Add Swagger UI and OpenAPI spec generation.

### Fixed

- Raise 404 error when policy cannot be found by its ID.

## [0.0.1]

### Added

- Add REST API for policies and authorization.
- Move all crates inside the crates folder.
- Return policy ID from the db.
- Add policy validation.
- Initial commit.

