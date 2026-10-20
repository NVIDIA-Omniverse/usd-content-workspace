(userinfo-api-directory-synchronization-title)=
# Directory Synchronization and Data Freshness

User Info Service serves directory data that is eventually consistent with the organization's identity provider. How an implementation backs that data — for example, a synchronized copy of the directory or queries to the identity provider — is outside the API contract. This section describes the data freshness and readiness contract that consuming services MUST design against.

(userinfo-api-directory-synchronization-consistency-model)=
## Consistency model

Directory data served by the API is **eventually consistent** with the identity provider. Changes made in the identity provider — new users, updated profiles, disabled accounts, modified group memberships, deleted entries — are reflected in API responses only after the service next refreshes its directory data from the identity provider.

When an implementation refreshes directory data on an interval, that interval MUST be deployment-configurable. Consuming services MUST NOT assume that directory changes are reflected immediately. For example, a user added to a group in the identity provider MAY NOT appear in the group's member list until the next refresh completes.

(userinfo-api-directory-synchronization-readiness)=
## Readiness

The service MUST expose a readiness probe compatible with the deployment orchestrator (e.g. Kubernetes). The probe MUST report ready (for an HTTP probe, a `2xx` status) only when the service is able to serve directory data, and MUST report not ready until then. The orchestrator MUST use this probe to avoid routing traffic to an instance that has no directory data.

Once the service becomes ready, it MUST remain ready even if subsequent refreshes fail — it continues serving the most recently retrieved data.

(userinfo-api-directory-synchronization-identity-provider-unavailability)=
## Identity provider unavailability

If refreshing directory data from the identity provider fails, the service MUST behave as follows:

- **Service previously ready** (it has served directory data before): the service MUST continue serving the last successfully retrieved data. API availability MUST NOT be affected. Data staleness grows until the next successful refresh.
- **Service not yet ready** (it has not yet been able to serve directory data): the service MUST remain not ready. API requests that depend on directory data MUST fail until a refresh succeeds.

The service MUST NOT expose partially refreshed data to callers. Updated directory data MUST become visible atomically — either all changes from a refresh are reflected, or none are.

(userinfo-api-directory-synchronization-enterprise-application-scoping)=
## Enterprise-application scoping (optional)

A deployment MAY restrict the served directory to the groups assigned to a specific enterprise application, so that the groups exposed here match those the application emits into user tokens (the set usable by downstream services such as the Permission Service). When this scoping is enabled:

- **Groups** are limited to those assigned to the enterprise application. Groups that are not assigned are absent from all group and membership responses, even if they exist in the identity provider.
- **Memberships** are limited to the assigned groups; a user's group listing reflects only their memberships within that assigned set.
- **Users** are not restricted — the full set of directory users remains available. Additionally, service principals assigned to the application MAY be surfaced as users so that they can be resolved like any other caller.
- The assigned set is re-evaluated on every refresh of directory data, so assignment additions, removals, and renames converge under the same eventual-consistency contract as any other directory change.

When scoping is disabled (the default), the full tenant directory is served. This scoping affects only which entities are visible; it does not change response shapes, error codes, or any other part of the API contract.

(userinfo-api-directory-synchronization-see-next)=
## See next

* {ref}`Authentication and authorization <userinfo-api-authentication-and-authorization-title>`
