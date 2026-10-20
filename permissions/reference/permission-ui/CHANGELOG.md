# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Add new entries under the current (topmost) release, in the matching `Added`, `Changed`, `Removed`, `Fixed`, or
`Security` section. Create the section if it does not exist yet. Do not add release dates.

## [0.0.22]

### Changed

- Container image builds on the public nginxinc/nginx-unprivileged image by default; NVIDIA builds still use simple-nginx through the SERVE_IMAGE build argument

### Security

- React Router upgraded to 7.18.4 (GHSA advisory on React Server error-type validation)
- simple-nginx base image updated to 1.1.7 (NVIDIA distroless v4.1.4; OpenSSL 3.5.7, glibc 2.41-12+deb13u4)

## [0.0.22] - 2026-08-03

### Added

- Group-scoped policy support in the UI
- Order filter on the policy list search
- Configurable, reorderable, persisted columns on the policy list
- Multi-select policy deletion with progress and retry dialog
- Empty-state placeholder when policy search returns no results
- `created_by` resolved to display name on the policy list
- Identifier search on user and group lists
- Resource selector in the policy form operator field
- Principal column on the diagnostic results table
- Parent type support for diagnostics hierarchy expansion
- Principal group membership passed to diagnostics
- Batch diagnostics API with multi-resource testing
- Helm `userService` section with enabled flag and default endpoint
- Lock logo favicon with transparent background

### Changed

- React upgraded to 19.2 and Mantine to 9
- Update simple-nginx to 1.1.6

## [0.0.21]

### Added

- Diagnostics UI for debugging policy evaluation
- Optional comment field on policies for administrator metadata
- Firewall-style order field on policies to control evaluation order
- Migration readiness report and metadata
- Display name resolution for the policy form's initial principal value
- "UNSCOPED" badge shown as the text input value instead of "NULL"

### Changed

- Reduced font size across UI components
- Helm chart restarts pods when the ConfigMap changes

### Fixed

- Double-encoding of resource IDs in the policy form
- Policies deleted incorrectly during editing due to deduplication

## [0.0.18]

### Added

- Users and Groups pages
- Resolution of user identifiers to human-readable display names (and vice versa) in the policy form

## [0.0.17]

### Changed

- Helm chart certificate request no longer sets a common name (CertManager compatibility)

## [0.0.15]

### Added

- `created_at` and `created_by` fields on the policy list

### Changed

- Default UI host and Permission Service backend endpoint in Helm values

## [0.0.14]

### Added

- Ingress support in the Helm chart

## [0.0.13]

### Changed

- Default container image repository to `nvcr.io/nvidia/omniverse`

## [0.0.12]

### Changed

- Default Helm values aligned with the simple-nginx base image

## [0.0.11]

### Added

- Third-party attributions document (`ATTRIBUTIONS.md`)

## [0.0.10]

### Changed

- Permission Service REST API version updated from `v1` to `v1beta`

## [0.0.7]

### Changed

- Invalid email principals now show a warning instead of an error

## [0.0.6]

### Fixed

- Text selection on the policy list no longer navigates to policy details
- Policy list not refreshing after submitting the policy form or deleting a policy
- Actions removed from the service form when clicking a row
- Unsaved-changes prompt appearing after a successful policy form submission

## [0.0.5]

### Added

- Global policy search
- Wildcard support for resources in policy search
- URL percent-encoding warning in search

### Changed

- Narrower navigation sidebar

## [0.0.4]

### Added

- Breadcrumbs on the policy form
- Autocomplete on the policy search form
- Policy editing (replace existing policy via put and delete)

### Changed

- Search and policy form values are trimmed on submit

## [0.0.3]

### Added

- Wildcard support for actions in policy search

## [0.0.2]

### Changed

- Pagination limited to previous and next page controls

### Fixed

- Incorrect tooltips about underscores in the policy form

## [0.0.1]

### Added

- Policy list, search, details, creation, and deletion
- Service registry UI with service metadata deletion
- Autocomplete for services, actions, and resource types in forms
- Current-user display button
- OpenID Connect authentication support
