# Changelog

Notable change history for the discovery-chart project.


## [2.4.1-rc.3] - 2026-08-13

### 💼 Other

- Merge branch 'ci/github-publish-from-main' into 'main'

ci: use github-publish component from ci-components main

See merge request omniverse/usd-storage/api-discovery/discovery-chart!61
- Updates helpers to ensure proper hostnames when not using httpproxy

### ⚙️ Miscellaneous Tasks

- Pin github-publish component to ci-components main

## [2.4.1-rc.2] - 2026-08-10

### 🐛 Bug Fixes

- Fixes nspect ci

### 💼 Other

- Merge branch 'rschanafelt/fix-nspect-ci' into 'main'

fixes nspect ci error

See merge request omniverse/usd-storage/api-discovery/discovery-chart!60

## [2.4.1-rc.1] - 2026-08-07

### 💼 Other

- Merge branch 'chore/relicense-apache-2.0' into 'main'

chore: relicense to Apache-2.0

See merge request omniverse/usd-storage/api-discovery/discovery-chart!58
- Bumps version to 2.4.1-rc.1
- Merge branch 'rschanafelt/bump-to-v2.4.1-rc.1' into 'main'

bumps version to 2.4.1-rc.1

See merge request omniverse/usd-storage/api-discovery/discovery-chart!59

### ⚙️ Miscellaneous Tasks

- Relicense to Apache-2.0

## [2.4.0-rc.2] - 2026-08-04

### 💼 Other

- Adds GitHub publishing to CI pipeline
- Bumps version, updates changelog
- Merge branch 'rschanafelt/github-publishing' into 'main'

adds GitHub publishing to CI pipeline

See merge request omniverse/usd-storage/api-discovery/discovery-chart!56
- Ignore tar files
- Merge branch 'rschanafelt/ci-fix' into 'main'

ignore tar files

See merge request omniverse/usd-storage/api-discovery/discovery-chart!57

## [2.4.0-rc.1] - 2026-08-03

### 💼 Other

- Adds helm testing setup
- Changes default image to nginx:1.31
- Bumps version
- Adds third-party notice
- Merge branch 'rschanafelt/try-nginx-base' into 'main'

Move to official nginx container image

See merge request omniverse/usd-storage/api-discovery/discovery-chart!55

## [2.3.16-rc.3] - 2026-07-21

### 💼 Other

- Updates simple-nginx to 1.1.6 to address CVEs
- Merge branch 'rschanafelt/simple-nginx-1.1.6' into 'main'

updates simple-nginx to 1.1.6 to address CVEs

See merge request omniverse/usd-storage/api-discovery/discovery-chart!54

## [2.3.16-rc.2] - 2026-07-08

### 💼 Other

- Merge branch 'fix/osec-2793-bump-prp-scan-components' into 'main'

chore: bump Pulse security-scan component pins to PRP-ready versions (OSEC-2793)

See merge request omniverse/usd-storage/api-discovery/discovery-chart!52
- Updates simple-nginx to 1.1.5
- Merge branch 'rschanafelt/simple-nginx-1.1.5' into 'main'

Updates simple-nginx to 1.1.5

See merge request omniverse/usd-storage/api-discovery/discovery-chart!53

### ⚙️ Miscellaneous Tasks

- Bump Pulse security-scan component pins to PRP-ready versions (OSEC-2793)

## [2.3.16-rc.1] - 2026-06-12

### 💼 Other

- Updates simple-nginx to 1.1.4
- Merge branch 'rschanafelt/updates-container-1.1.4' into 'main'

updates simple-nginx to 1.1.4

See merge request omniverse/usd-storage/api-discovery/discovery-chart!51

## [2.3.15-rc.1] - 2026-05-20

### 💼 Other

- Merge branch 'rschanafelt/version-2.3.15-rc.1' into 'main'

chore: updates version to RC

See merge request omniverse/usd-storage/api-discovery/discovery-chart!50

### ⚙️ Miscellaneous Tasks

- Updates version to RC

## [2.3.14-rc.1] - 2026-05-20

### 🚀 Features

-   feat(values): register live-edit-service in discovery catalog

  Adds a `live-edit-service-01` entry with `type: "live-service"` to the
  default `discovery.services` list and a matching defaults block in the
  `mkurl` helper (serviceName `live-edit-service`, ingressHost `live-edit`,
  gRPC port 50001). live-edit-service is gRPC-only, so the values entry
  omits the `rest` endpoint and the defaults dict carries no `rest` port —
  `mkServiceJson` skips REST emission when the endpoint key is absent.

  The ingressHost is `live-edit` (not `grpc.live-edit`) because the
  discovery helper auto-prepends `grpc.` for gRPC endpoints, yielding
  `grpc.live-edit.<domain>` — exactly matching the live-edit chart's own
  `httpProxy.fqdn.host` default.

### 🏗️ Build & CI

- Bumps version, updates changelog
- Bump version for new rc candidate

### 💼 Other

- Merge branch 'jesseg/feature/add-live-edit' into 'main'

feat(values): register live-edit-service in discovery catalog

See merge request omniverse/usd-storage/api-discovery/discovery-chart!48
- Merge branch 'rschanafelt/bump-2.3.14' into 'main'

Bump version for new rc candidate

See merge request omniverse/usd-storage/api-discovery/discovery-chart!49

## [2.3.12] - 2026-05-19

### 💼 Other

- Added defaults for Permisisons and Userinfo svcs
- Merge branch 'fidot/add-permissions-service' into 'main'

Added defaults for Permisisons and Userinfo svcs

See merge request omniverse/usd-storage/api-discovery/discovery-chart!47

## [2.3.11] - 2026-05-05

### 🐛 Bug Fixes

- *(container)* Updates to simple-nginx image with latest security fixes (1.1.0)

### 💼 Other

- Merge branch 'rschanafelt/updates-simple-nginx' into 'main'

Updates to simple-nginx image with latest security fixes (1.1.0)

See merge request omniverse/usd-storage/api-discovery/discovery-chart!46
- Updates repo version

## [2.3.10-rc.2] - 2026-04-23

### 💼 Other

- Merge branch 'fix/bump-prelaunch-v0.32.0' into 'main'

ci: bump prelaunch component to v0.32.0

See merge request omniverse/usd-storage/api-discovery/discovery-chart!43

### ⚙️ Miscellaneous Tasks

- Bump prelaunch component to v0.32.0
- Remove compliance include (osec:checkmarx, osec:pulse-trufflehog)
- Set prelaunch jobs to run automatically on MR pipelines

## [2.3.10-rc.1] - 2026-04-23

### 💼 Other

- Bumps version for new version scheme
- Merge branch 'rschanafelt/new-versioning' into 'main'

bumps version for new version scheme

See merge request omniverse/usd-storage/api-discovery/discovery-chart!45

## [2.3.9] - 2026-04-22

### 🚀 Features

- Add conformance tests for discovery-service [OMPE-89073]
- *(ci)* Add prelaunch security scans

### 🐛 Bug Fixes

- Fix artifact directory
- *(ci)* Use packaged tgz for helm prelaunch scans
- *(ci)* Proper prelaunch scan configuration
- Add alternate variable name for secretscan exclusion
- Use TRUFFLEHOG_EXTRA_ARGS for detector exclusion

### 💼 Other

- Merge branch 'feat/conformance-tests' into 'main'

[OMPE-89073] Add conformance tests for discovery-service

See merge request omniverse/usd-storage/api-discovery/discovery-chart!36
- Merge branch 'rschanafelt/fix-ci' into 'main'

fix artifact directory

See merge request omniverse/usd-storage/api-discovery/discovery-chart!37
- Adds helm binary to CI
- Merge branch 'rschanafelt/fix-ci-helm' into 'main'

adds helm binary to CI

See merge request omniverse/usd-storage/api-discovery/discovery-chart!38
- Adds package repo for helm
- Merge branch 'rschanafelt/fix-ci-helm-2' into 'main'

adds package repo for helm

See merge request omniverse/usd-storage/api-discovery/discovery-chart!39
- Switches to components for helm publish
- Merge branch 'rschanafelt/fix-helm-3' into 'main'

switches to components for helm publish

See merge request omniverse/usd-storage/api-discovery/discovery-chart!40
- Add migration readiness report and metadata
- Merge branch 'feat/add-prelaunch-security-scans' into 'main'

feat(ci): add prelaunch security scans

See merge request omniverse/usd-storage/api-discovery/discovery-chart!41
- Deleting comment
- Merge branch 'updating-smoke-test-base' into 'main'

chore: update smoke-test-base image tag from 0.0.5 to 1.0.1

See merge request omniverse/usd-storage/api-discovery/discovery-chart!42
- Bumps version to 2.3.9
- Merge branch 'rschanafelt/bump-version' into 'main'

Bumps version to 2.3.9

See merge request omniverse/usd-storage/api-discovery/discovery-chart!44

### ⚙️ Miscellaneous Tasks

- Trigger MR creation
- Update smoke-test-base image tag from 0.0.5 to 1.0.1
- Normalize smokeTest values.yaml structure
- Add consistent comments to smokeTest values section

## [2.3.8] - 2026-03-31

### 🐛 Bug Fixes

- Updates simple-nginx version to 1.0.2

### 💼 Other

- Merge branch 'rschanafelt/update-nginx' into 'main'

Updates simple-nginx version to 1.0.2

See merge request omniverse/usd-storage/api-discovery/discovery-chart!35

## [2.3.7] - 2026-03-24

### 💼 Other

- Update nginx base image to 1.0.1, version to 2.3.7
- Merge branch 'nginx-base-image' into 'main'

Update nginx base image to 1.0.1, version to 2.3.7

See merge request omniverse/usd-storage/api-discovery/discovery-chart!34

## [2.3.6] - 2026-03-24

### 💼 Other

- Update simple-nginx base image, bump version to 2.3.6
- Merge branch 'nginx-base-image' into 'main'

Update simple-nginx base image, bump version to 2.3.6

See merge request omniverse/usd-storage/api-discovery/discovery-chart!33

## [2.3.5] - 2026-03-10

### 💼 Other

- Remove commonName from CertManager's Certificate Request
- Merge branch 'fidot/remove-cert-common-name' into 'main'

Remove commonName from CertManager's Certificate Request

See merge request omniverse/usd-storage/api-discovery/discovery-chart!31
- Bump version
- Merge branch 'rschanafelt/bump-version' into 'main'

Bump version

See merge request omniverse/usd-storage/api-discovery/discovery-chart!32

## [2.3.4] - 2026-02-25

### 🐛 Bug Fixes

- *(ci)* Removes legacy CI build

### 💼 Other

- Merge branch 'rschanafelt/fix-ci' into 'main'

fix(ci): removes legacy CI build

See merge request omniverse/usd-storage/api-discovery/discovery-chart!29

## [2.3.3] - 2026-02-25

### 🐛 Bug Fixes

- *(container)* Updates simple-nginx, adds changelog

### 💼 Other

- Merge branch 'rschanafelt/update-container' into 'main'

fix(container): updates simple-nginx, adds changelog

See merge request omniverse/usd-storage/api-discovery/discovery-chart!28

## [2.3.2] - 2026-02-17

### 💼 Other

- Fixed a typo in the helper that constructs REST service URLs in Helm

## [2.3.1] - 2026-02-12

### 💼 Other

- Bumping smoke-test-base to 0.0.5
- Merge branch 'agevorgyan/smoke_test_base' into 'main'

Bumping smoke-test-base to 0.0.5

See merge request omniverse/usd-storage/api-discovery/discovery-chart!25
- Add default timeouts
- Merge branch 'fidot/add-timeouts' into 'main'

Add default timeouts

See merge request omniverse/usd-storage/api-discovery/discovery-chart!24
- 2.3.1

## [2.3.0] - 2026-02-10

### 💼 Other

- Add auth hookups and auth-config generation to Disco
- Merge branch 'fidot/add-auth' into 'main'

Add auth hookups and auth-config generation to Disco

See merge request omniverse/usd-storage/api-discovery/discovery-chart!15
- Update VERSION.toml

## [2.2.0] - 2026-02-06

### 💼 Other

- Adds health check to nginx config
- Bumps version
- Merge branch 'rschanafelt/add-health-check' into 'main'

adds health check to nginx config

See merge request omniverse/usd-storage/api-discovery/discovery-chart!23

## [2.1.0] - 2026-02-06

### 💼 Other

- Add readiness/liveness probes
- Merge branch 'update-simple-nginx-health-probes' into 'main'

OMPE-79489: add readiness/liveness probes

See merge request omniverse/usd-storage/api-discovery/discovery-chart!21
- Removes tag suffix creation
- Merge branch 'rschanafelt/fix-tags' into 'main'

removes tag suffix creation

See merge request omniverse/usd-storage/api-discovery/discovery-chart!22

## [2.0.12-dev] - 2026-02-06

### 💼 Other

- Disabling smokeTest for main.
- Merge branch 'agevorgyan/smoke_tests' into 'main'

Disabling smokeTest for main.

See merge request omniverse/usd-storage/api-discovery/discovery-chart!20

## [2.0.11-dev] - 2026-02-05

### 💼 Other

- Add httpProxy.corsPolicy.exposeHeaders and httpProxy.annotations
- Merge branch 'http-proxy/annotations' into 'main'

Add httpProxy.corsPolicy.exposeHeaders and httpProxy.annotations

See merge request omniverse/usd-storage/api-discovery/discovery-chart!19
- 2.0.11

## [2.0.10-dev] - 2026-02-04

### 💼 Other

- Adds override to CI nSpect stage
- Merge branch 'rschanafelt/fix-nspect-ci' into 'main'

adds override to CI nSpect stage

See merge request omniverse/usd-storage/api-discovery/discovery-chart!18

## [2.0.9-dev] - 2026-02-04

### 💼 Other

- Update default image repository to nvcr.io/nvidia/omniverse
- Merge branch 'update-image-registry' into 'main'

Update default image repository to nvcr.io/nvidia/omniverse

See merge request omniverse/usd-storage/api-discovery/discovery-chart!17

## [2.0.8-dev] - 2026-01-30

### 💼 Other

- Adding smoke test
- Merge branch 'agevorgyan/smoke_tests' into 'main'

Adding smoke test

See merge request omniverse/usd-storage/api-discovery/discovery-chart!12
- Bump version to 2.0.8.
- Merge branch 'agevorgyan/smoke_tests' into 'main'

Bump version to 2.0.8.

See merge request omniverse/usd-storage/api-discovery/discovery-chart!16

## [2.0.7-dev] - 2026-01-20

### 💼 Other

- Update values.yaml to set corsPolicy.allowCredentials to false by default since it's not compatible with corsPolicy.allowOrigin set to '*'.
- Update VERSION.toml
- Update nSpect jobs to register proper container and the helm chart
- Merge branch 'nspect' into 'main'

Update nSpect jobs to register proper container and the helm chart

See merge request omniverse/usd-storage/api-discovery/discovery-chart!14
- Fix incorrect paths to helm/values.yaml file in nspect:register:container
- Add GIT_STRATEGY and GIT_CHECKOUT to nspect:register:version
- Extract nspect:container:vars job to prepare the container name for nspect:register:container

## [2.0.6-dev] - 2026-01-15

### 💼 Other

- Adding Ingress (httpProxy) to helm chart
- Merge branch 'fidot/add-ingress' into 'main'

Adding Ingress (httpProxy) to helm chart

See merge request omniverse/usd-storage/api-discovery/discovery-chart!11
- Bumps version
- Merge branch 'rschanafelt/bump-version' into 'main'

bumps version

See merge request omniverse/usd-storage/api-discovery/discovery-chart!13

## [2.0.5-dev] - 2025-12-10

### 🐛 Bug Fixes

- Fix nspect service name
- Fixes schema version key

### 💼 Other

- Deleting test suit
- Merge branch 'deleting-test-suite' into 'main'

Deleting test suit

See merge request omniverse/usd-storage/api-discovery/discovery-chart!7
- Merge branch 'rschanafelt/fix-nspect-release' into 'main'

fix nspect service name

See merge request omniverse/usd-storage/api-discovery/discovery-chart!8
- Merge branch 'rschanafelt/fix-schema' into 'main'

fixes schema version key

See merge request omniverse/usd-storage/api-discovery/discovery-chart!9

## [2.0.4-dev] - 2025-12-10

### 💼 Other

- Add nSpect support
- Merge branch 'rschanafelt/adds-nspect' into 'main'

Adds nSpect support

See merge request omniverse/usd-storage/api-discovery/discovery-chart!6

## [2.0.3-rc.1-dev] - 2025-12-09

### 💼 Other

- Fixing bug where namespaces weren't processed correctly, remove URL options from values, and make endpoints optional
- Merge branch 'fidot/remove-url-add-ports-bugfixes-bump-version' into 'main'

Fixing bug where namespaces weren't processed correctly, remove URL options from values, and make endpoints optional

See merge request omniverse/usd-storage/api-discovery/discovery-chart!5

## [2.0.2-dev] - 2025-12-09

### 💼 Other

- Added test suite
- Changing default values
- Merge branch 'adding-test-suite' into 'main'

Added test suite

See merge request omniverse/usd-storage/api-discovery/discovery-chart!1
- Fix automatic endpoint generation; remove permissions' endpoints; add notifications' endpoints
- Merge branch 'fidot/fix-endpoints-autogeneration' into 'main'

Fix automatic endpoint generation; remove permissions' endpoints; add notifications' endpoints

See merge request omniverse/usd-storage/api-discovery/discovery-chart!4

## [2.0.1-dev] - 2025-12-09

### 🐛 Bug Fixes

- Fixes values file, improves docs

### 💼 Other

- Merge branch 'rschanafelt/fixes-values-file' into 'main'

fixes values file, improves docs

See merge request omniverse/usd-storage/api-discovery/discovery-chart!3

## [2.0.0-dev] - 2025-12-09

### 💼 Other

- Initial commit
- Adds initial files
- Base structure  and values structure for Disco service chart
- Renamed discoveryConfig -> discovery
- Simplifies values file
- Adds more default value settings
- Latest tweaks and fixes to Disco helm charts: secrets, services, naming, and config
- Adds gitlab CI and documentation
- Merge branch 'rschanafelt/adds-ci' into 'main'

Adds gitlab CI

See merge request omniverse/usd-storage/api-discovery/discovery-chart!2

