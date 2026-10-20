# Changelog

Change history for the **envoy-auth-ext** project

This file is maintained by hand. Add entries for the planned release under the heading that matches
`workspace.package.version` in `Cargo.toml`, in the matching section. Do not regenerate it from the
commit history: commit messages carry internal references, and the curated entries would be lost.

## [2.4.1]

### 🚀 Features

- Add the `openId.requiredJwtClaims` Helm value to reject tokens whose claims do not satisfy the configured rules (`equals`, `notEquals`, `in`, `notIn`, `contains`, `containsAny`, `containsAll`, `exists`)
- Add the `config.jwtLeeway` Helm value to set the clock skew, in seconds, tolerated when validating JWT `exp` and `nbf` claims (0-300)

### 📚 Documentation

- Document required JWT claims and the JWT leeway setting in the README
- Document Contour local rate limiting as the way to bound authorization checks
- Update the README for building and running from the published source

### ⚙️ Miscellaneous Tasks

- Update permission-auth and permission-core to 0.0.54
- Update the runtime base image to `nvcr.io/nvidia/distroless/cc:v4.1.6`
- Vendor the `tonic-observability` and `otel-tools` crates so the project builds without a private Cargo registry
- Relicense the project under the Apache License 2.0

### 🛡️ Security

- Require the `aud` claim, require the `iss` claim when `openId.allowedJwtIssuers` is set, and enforce the `nbf` claim when validating JWTs
- Stop recording the `Check` request, including bearer tokens and cookies, in logs and traces, and change the default log level to `info`
- Remove the unverified principal hint from failed-authentication logs and reject tokens longer than `openId.maxTokenLength` before verification
- Add the `config.idpConnectTimeout`, `config.idpRequestTimeout`, and `config.idpPoolIdleTimeout` Helm values to bound identity provider requests (5, 15, and 90 seconds by default)

### 💼 Other

- Replace the `openId.clientRegistrations` and `openId.additionalJwtAudience` Helm values with `openId.allowedJwtAudience`, now the only source of accepted token audiences; deployments using the old values must migrate
- Rename the `openId.jwtIssuers` Helm value to `openId.allowedJwtIssuers`
- Remove the unused `openId.enabled` and `openId.principalGroupsClaim` Helm values
- Switch the liveness and readiness probes from TCP to gRPC health checks on the `service.ports.envoyAuthExt` port, limit the `livenessProbe` and `readinessProbe` Helm values to probe timings, and require Kubernetes 1.27 or later

## [2.4.0]

### 🚀 Features

- Add experimental prelaunch security scans for container images
- Add helm and repository scans, NSPECT Scans trigger job
- *(helm)* Add liveness and readiness probes to deployment
- *(helm)* Add security context to satisfy Checkov checks
- Expose the new authentication settings (JWT issuers, principal id and groups claims, maximum token length, negative-token cache, and JWKS failure backoff) in the Helm chart

### 🐛 Bug Fixes

- Correct the cache-size key so `token_cache_size` is applied instead of the ignored `jwt_cache_size`
- Run prelaunch scans in build stage after docker:build
- Export container image URI for prelaunch scans via dotenv
- Use tags-only runner template for prelaunch scans
- Correct helm_chart_paths for prelaunch:helm:checkov scan
- Fixes resource settings in helm values
- *(ci)* Add feat/ branch to workflow rules
- *(ci)* Expect clean Checkov scan (exit code 0)
- *(docker)* Update distroless base image to v4.0.3 to fix OpenSSL CVEs
- *(docker)* Update distroless base image to nvcr.io/nvidia/distroless/cc:v4.0.4
- *(docker)* Try nvidian registry path for distroless v4.0.4

### 💼 Other

- Add migration readiness report and metadata
- Add Checkov skip for CKV_K8S_21 false positive
- Consolidate the failed-authentication log hint to reuse `principal_id_claim`, removing the redundant `log_user_claim` setting

CKV_K8S_21 "Default namespace should not be used" is a false positive for
Helm charts since templates render with 'default' but users deploy to
actual namespaces via `helm install --namespace=<ns>`.
- Add inline Checkov skip for CKV_K8S_21 in Helm templates

Add checkov:skip=CKV_K8S_21 comment to all Kubernetes resource templates.
This is required because the .checkov.yaml config file is not picked up
when Checkov scans the packaged Helm chart.
- Move Checkov skip comments to file start

Move checkov:skip=CKV_K8S_21 comments to the beginning of each template
file (before apiVersion) as the inline format after metadata: is not
being recognized by Checkov's Helm framework scanner.
- Add explicit namespace to Helm templates for CKV_K8S_21

Add namespace: {{ .Release.Namespace }} to all Kubernetes resource
metadata sections. This properly fixes Checkov CKV_K8S_21 which was
flagging the use of default namespace.

Also remove .checkov.yaml as inline skip comments don't work for
rendered Helm templates.
- Adds default deployment settings, cliff config, updates version

### ⚙️ Miscellaneous Tasks

- *(ci)* Update prelaunch components to v0.28.1
- Bump prelaunch component to v0.32.0
- Set prelaunch jobs to run automatically on MR pipelines
- Remove NSPECT Scans workaround job
- Bump prelaunch_container and prelaunch_helm to v0.32.0

## [2.3.3] - 2026-03-11

### 🐛 Bug Fixes

- Create dummy helmfile.d to satisfy generate-sbom workspace check
- Resolve dotenv variable at runtime for build:sbom

### 💼 Other

- Bumps version
- Update file .gitkeep
- Fix inconsistency between .gitlab-ci.yml publishing the image as...

### ⚙️ Miscellaneous Tasks

- Integrate SBOM generation into CI pipeline
- Align SBOM generation with SRD requirements
- Temporarily enable build:sbom on MR pipelines for verification
- Switch to build-sbom CI component for SRD-compliant SBOM
- Restrict build:sbom to tag-only pipelines
- Pin build-sbom component to ci-components@0.2.0

## [2.3.2] - 2026-02-20

### 💼 Other

- Remove legacy publishing jobs
- Updates distroless to patch libssl CVE

## [2.3.1] - 2026-02-12

### 💼 Other

- Use permission-auth:0.0.45 crate (fixes AuthType serde deserialization)

## [2.3.0] - 2026-02-11

### 💼 Other

- Update default image repository to nvcr.io/nvidia/omniverse

Update the default container image repository in values.yaml from
the internal staging registry to
nvcr.io/nvidia/omniverse/envoy-auth-extension to align with the standard
Omniverse container registry location.
- Refactor JWT claim extraction and enhance logging configuration

- Renamed `extract_sub_from_token` to `extract_claim_from_token` to allow extraction of any claim from the JWT.
- Updated tests to reflect the new claim extraction method and added a test for custom claims.
- Introduced `log_user_claim` setting in `ServerSettings` to specify which claim to log, defaulting to "sub".
- Updated configuration files to include the new `log_user_claim` setting.
- Adds default log claim to values.yaml
- Update OpenID configuration URL in envoy-ext-config-test.toml to use example.com
- Configuration standardization and cleanup

## [2.2.3] - 2026-02-03

### 💼 Other

- Enhance logs

## [2.2.2] - 2026-01-29

### 💼 Other

- Adds more error details

## [2.2.1] - 2026-01-29

### 💼 Other

- Ensures openid config loaded at startup

## [2.2.0] - 2026-01-22

### 🐛 Bug Fixes

- Fixes clippy warnings for generated protos

### 💼 Other

- Switches to new openid configuration argument

## [2.1.2] - 2026-01-21

### 💼 Other

- Updates perm-auth to 0.0.42
- Bumps project version

## [2.1.1] - 2026-01-20

### 💼 Other

- Updates auth-perm settings and fixes reversion

## [2.1.0] - 2026-01-20

### 💼 Other

- Bump project version

## [2.0.3] - 2025-12-09

### 💼 Other

- Fix image reference

## [2.0.2] - 2025-11-10

### 🐛 Bug Fixes

- Fixes missing env var error; version bump

### 💼 Other

- Initial OSRB updates
- Adds NGC login

## [2.0.1] - 2025-10-23

### 🐛 Bug Fixes

- Fixes service name

## [2.0.0] - 2025-10-22

### 💼 Other

- Migrate to new repo, remove unneeded envoy proxy settings

## [1.1.0] - 2025-09-25

### 💼 Other

- Updates permissions to latest -> 0.36, fixes version number
- Updates app and chart versions

## [0.12.1004] - 2025-09-18

### 💼 Other

- [OMPE-62744] Add the internal staging registry to discovery-service
- [OMPE-62744] chore(ci): Remove latest tag and force push from staging publish
- [OMPE-62744] Add manual deployment gate for tag-based releases
- [OMPE-62744] Modernize CI pipeline with CDS v0.16.8 and CI Components
- [OMPE-63671] Add storage-api-dev publishing for NGC dev environment
- [OMPE-64083] Add NSPECT security scanning support

## [1.0.1] - 2025-06-18

### 💼 Other

- Bumps permission-auth version

## [1.0.0] - 2025-06-17

### 💼 Other

- Removes deprecated envoy responses

## [0.12.1] - 2025-06-10

### 🐛 Bug Fixes

- Fixes CI repo, bumps version

## [0.12.0] - 2025-05-13

### 💼 Other

- Adds support for additional auth-config data

## [0.11.0] - 2025-05-08

### 💼 Other

- Support configuring the auth type

## [0.10.0] - 2025-04-21

### 💼 Other

- Updates permission service crate

## [0.9.0] - 2025-04-21

### 💼 Other

- Improved testing
- Adds sequence diagram
- Updates API signature, bumps version

## [0.8.0] - 2025-03-18

### 💼 Other

- Initial work
- Adds kcl yaml generation, static content, Justfile
- Adds direct Helm support
- Removes kcl, updates helm files to match dev deployment
- Extracts services-200.json out of configmap.yaml, adds helmfile for testing
- Cleans up values files
- Adds OTEL parameter, version bump
- Updates chart to v0.5.0
- Import auth extension
- Switches back to vendored proto files
- Updates to permission-auth 0.0.12
- Bumps version to 0.8.0
