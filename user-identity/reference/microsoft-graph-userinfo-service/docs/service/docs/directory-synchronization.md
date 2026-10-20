(userinfo-service-directory-synchronization-directory-synchronization-and-data-freshness)=
# Directory Synchronization and Data Freshness

User Info Service does not own the directory it serves. It keeps a local replica of the
organization's identity provider (currently Microsoft Entra ID) and answers every request from
that replica. This page describes how the replica is kept aligned with the identity provider,
how the replica reaches the instances that serve requests, and what data freshness callers can
expect as a result.

Synchronization applies to the recommended deployment, in which a shared store carries directory
data between instances. It does **not** apply to the reader-only live mode, in which every
request is answered directly from the identity provider and nothing is synchronized. See
{ref}`User Info Service Configuration <userinfo-service-deployment-configuration>` for how to choose between the two
deployments, and {ref}`Live Mode <userinfo-service-directory-synchronization-live-mode>` below
for how the live mode differs.

(userinfo-service-directory-synchronization-writer-and-reader-roles)=
## Writer and Reader Roles

The service runs in one of two roles, deployed as separate workloads that share a common
database:

- The **writer** keeps the replica aligned with the identity provider. It reads users, groups,
  and group memberships from the provider and saves them to the shared store. The writer does
  not serve directory requests, and only one writer runs at a time.
- Each **reader** serves the REST and gRPC APIs. Readers load the replica the writer has saved,
  refresh it on a fixed interval, and never contact the identity provider for synchronization.
  Read capacity scales out by adding more reader replicas.

Separating the two roles keeps the synchronization work off the request path. A reader becomes
ready and starts serving within seconds by loading what the writer has already saved, instead of
waiting for a directory synchronization to finish. For how the shared store is configured and
what it holds, see {ref}`Database Configuration <userinfo-service-database-database-configuration>`.

(userinfo-service-directory-synchronization-initial-and-incremental)=
## Initial and Incremental Synchronization

The writer keeps the replica current using the identity provider's incremental-change mechanism,
which lets it ask the provider only for what has changed since the previous request rather than
re-reading the whole directory each time.

- **Initial full synchronization.** On the first cycle, when there is no saved progress, the
  writer reads the entire directory. It requests users and groups page by page, saving each page
  to the shared store as it arrives. When the last page has been read, the provider hands back a
  marker that identifies the point in time the read reached.
- **Incremental synchronization.** On every later cycle, the writer presents the marker from the
  previous cycle and receives only the users, groups, and memberships that were added, changed,
  or removed since then. Removed entities are deleted from the store; added and changed entities
  are inserted or updated. Each cycle produces a fresh marker for the next one.

Full synchronization can take a long time for a large directory — potentially hours when
membership synchronization is enabled — while incremental cycles are typically much shorter
because they carry only the changes. See
{ref}`What Gets Synchronized <userinfo-service-directory-synchronization-what-gets-synchronized>`
for the effect of membership synchronization on synchronization time and memory use.

(userinfo-service-directory-synchronization-reading-from-graph)=
## Reading Changes from Microsoft Graph

The writer reads directory data through the [Microsoft Graph API][graph], the programming
interface to Microsoft Entra ID, using its [delta query][delta-overview] feature — also called
change tracking. Delta query lets an application retrieve only what has changed since its
previous request instead of re-reading a whole collection each time, which is what makes
incremental synchronization efficient and keeps the service within the provider's request
limits.

Two collections are tracked, each in its own change-tracking session:

- **Users** — [`GET /users/delta`][user-delta]
- **Groups** — [`GET /groups/delta`][group-delta]

Each request returns a page of results together with exactly one of two links, which the writer
saves and reuses rather than constructing URLs itself:

- A **next-page link** (`@odata.nextLink`) means more pages remain in the current round. The
  writer requests the next page using this link until no more remain.
- A **delta link** (`@odata.deltaLink`) means the round is complete. The writer saves this link
  and presents it at the start of the next cycle to receive only the changes made since. A
  single response never carries both links.

On the first cycle the writer starts a round with no saved link and walks every page to capture
the full current state of the directory. On later cycles it starts the round from the saved
delta link, so the provider returns only the users, groups, and memberships that changed since
the previous cycle. The links are opaque state tokens; the writer stores and replays them
without interpreting their contents.

**Selecting properties.** Each initial request names only the properties the service needs, so
responses stay small. For users these are the identifier, display name, given and family name,
email, username, account-enabled status, and job title; for groups, the identifier, display
name, and description. The identifier is always returned, and the chosen properties are encoded
into the returned links, so the writer does not repeat them on the following pages.

**Tracking group memberships.** When membership synchronization is enabled, the group request
additionally selects the `members` relationship. The provider then returns, on each group, a
`members@delta` list describing the members added or removed. A group with many members can span
several pages, each carrying a different portion of that group's membership, so the writer
accumulates the membership changes across all of a group's pages before applying them.

**Detecting deletions.** Deleted users and groups, and removed memberships, are reported as
entries carrying an `@removed` marker in place of the full record. The writer deletes the
corresponding records from the shared store. Because the same change can occasionally be
reported more than once, the writer applies changes idempotently — reapplying a change that has
already been made has no additional effect.

The page size of each response is chosen by the provider; the service does not request a
specific size, because the delta endpoints do not support that option.

**Scoped reading.** When the served directory is narrowed to a single enterprise application,
the writer does not use the global groups delta above. Instead it reads the application's
assignments through [`GET /servicePrincipals/{id}/appRoleAssignedTo`][app-role-assigned-to] and
rebuilds the assigned groups and their members from that list each cycle. See
{ref}`Directory Scoping <userinfo-service-directory-scoping-directory-scoping>` for details.

For how the writer authenticates to the provider as an application, the directory permissions it
requires, and how it honors the provider's request limits, see
{ref}`Microsoft Entra ID Integration <userinfo-service-identity-provider-microsoft-entra-id-integration>`.

(userinfo-service-directory-synchronization-interval-and-jitter)=
## Synchronization Interval and Startup Jitter

After each cycle completes, the writer waits a fixed interval before starting the next one. An
optional startup delay spreads out the first cycle when several instances start at the same time,
so they do not all begin reading from the identity provider at once.

| Setting | Environment variable | Helm value (`providers.cache.*`) | Default | Description |
|---|---|---|---|---|
| Synchronization interval | `SYNC_INTERVAL` | `syncInterval` | `5m` | How long the writer waits after finishing one cycle before starting the next. |
| Startup jitter | `SYNC_JITTER_MAX` | `syncJitterMax` | `0s` | Upper bound on a random delay applied before the first cycle only. `0s` disables the delay. |

The interval is measured from the end of one cycle to the start of the next, so a slow cycle does
not overlap the following one. The startup jitter is applied a single time, before the writer's
first cycle; every cycle after that follows the fixed interval.

(userinfo-service-directory-synchronization-how-readers-refresh)=
## How Readers Load and Refresh the Replica

When a reader starts, it loads whatever the shared store currently holds into memory and becomes
ready to serve immediately, even if the writer has not yet finished its first full
synchronization. A reader that starts against an empty store serves an empty directory until the
writer has saved data and the reader has picked it up.

After startup, each reader reloads the replica from the shared store on a fixed interval. Readers
never read from the identity provider and never take part in synchronization; they only re-read
what the writer has saved.

| Setting | Environment variable | Helm value (`reader.*`) | Default | Description |
|---|---|---|---|---|
| Reader reload interval | `READER_RELOAD_INTERVAL` | `reloadInterval` | `60s` | How often a reader reloads the replica from the shared store. |

Two rules keep a reader from serving worse data than it already has:

- If a reload fails — for example, because the store is briefly unreachable — the reader keeps
  serving the copy it already holds in memory rather than dropping to an empty directory.
- While the writer is performing a full synchronization, the store holds a partially built
  directory. A reader adopts that partial data only until it has loaded a complete directory;
  once it holds a complete copy, a later partial state never replaces it.

(userinfo-service-directory-synchronization-single-full-sync)=
## Coordinating a Single Full Synchronization

A full synchronization rebuilds the stored directory from scratch, so only one may run at a time.
The writer coordinates this through a lock in the shared database: before it begins a full
synchronization it must acquire the lock, and it releases the lock when the synchronization
finishes. This is why the writer runs as a single instance; the lock guards against a second
writer being started by accident.

If another writer already holds the lock, the waiting writer does not read from the identity
provider. It waits until the lock is free or until a complete directory appears in the store,
then loads that directory instead of building its own. If the writer process stops
unexpectedly, the database releases the lock automatically so the next writer can proceed.

(userinfo-service-directory-synchronization-what-gets-synchronized)=
## What Gets Synchronized

Three categories of directory data can be synchronized. Users and groups are synchronized by
default; group memberships are disabled by default because they are the most expensive to
synchronize and to hold in memory.

| Category | Environment variable | Helm value (`providers.cache.*`) | Default | Description |
|---|---|---|---|---|
| Users | `CACHE_USERS_ENABLED` | `users.enabled` | `true` | Synchronize user records. |
| Groups | `CACHE_GROUPS_ENABLED` | `groups.enabled` | `true` | Synchronize group records. |
| Memberships | `CACHE_MEMBERSHIPS_ENABLED` | `memberships.enabled` | `false` | Synchronize which users belong directly to which groups. |

Enabling membership synchronization can substantially increase both the initial synchronization
time and the memory each instance uses, because the writer must read every group's members and
each instance holds the full set of memberships in memory. For a large directory this can add
hours to the initial synchronization and require significantly more memory; size the memory
limits accordingly when enabling it. When a category is turned off, requests
for that kind of data are answered directly from the identity provider instead of from the
replica.

Changing which categories are enabled invalidates the stored directory: the next cycle discards
the saved progress and performs a fresh full synchronization so the stored data matches the new
configuration. The served directory can also be narrowed to a single enterprise application
rather than the whole tenant; see {ref}`Directory Scoping <userinfo-service-directory-scoping-directory-scoping>`.

(userinfo-service-directory-synchronization-data-freshness)=
## Data Freshness and Eventual Consistency

The service is **eventually consistent**. A change made in the identity provider becomes visible
to callers after two independent delays: the time until the writer's next cycle picks up the
change and saves it, and the time until each reader's next reload loads the updated directory.
In the worst case a change can therefore take up to one synchronization interval plus one reader
reload interval to become visible, on top of the time the synchronization cycle itself takes.

Because each reader holds its own copy of the directory and reloads on its own schedule, readers
are not perfectly aligned with one another. Callers may briefly observe minor differences between
readers — for example, a newly added user visible on one reader before another, or a removed user
still visible on a reader that has not yet reloaded. This is expected for the service's
eventually-consistent, read-only model and resolves on its own once every reader has reloaded.

Callers that require a strongly consistent, authoritative view of a specific record at a specific
moment should read it from the identity provider directly. User Info Service is designed for
fast, centralized lookups of a continuously refreshed replica, not as a system of record.

(userinfo-service-directory-synchronization-recovering-from-interruptions)=
## Recovering from Interruptions

The writer saves its progress to the shared store as it goes, so an interrupted synchronization
does not have to start over. Each page of users or groups is saved together with a marker of how
far the read reached. If the writer is restarted mid-synchronization, it resumes from the last
saved position rather than re-reading the directory from the beginning.

The delta links the writer stores can also expire on the provider's side. For directory objects
such as users and groups these links are valid for up to seven days, and the provider may also
retire a link during internal maintenance. In either case the provider responds with a
`410 Gone` status, or an error indicating that the synchronization state was not found or that a
resync is required. When this happens the writer discards the saved link and performs a fresh
full synchronization on the next cycle. Readers are unaffected and continue serving the last
complete directory they loaded.

(userinfo-service-directory-synchronization-rate-limits)=
## Rate Limits and Transient Errors

The identity provider limits how quickly it will answer requests and may occasionally return
temporary errors. During synchronization the writer honors the wait time the provider asks for
after a rate-limit response and backs off before retrying transient failures, so the service
coexists with other consumers of the same tenant. See
{ref}`Microsoft Entra ID Integration <userinfo-service-identity-provider-microsoft-entra-id-integration>` for how the service authenticates to
the provider and the retry behavior in more detail.

If a whole synchronization cycle fails, the writer does not retry immediately; it waits for the
next scheduled interval and tries again. Meanwhile it keeps the last directory it built available
in the store, so readers continue serving the most recent good data rather than losing the
directory because of a transient provider problem.

(userinfo-service-directory-synchronization-helm-configuration)=
## Configuring Synchronization in the Helm Chart

The Helm chart deploys the writer and the readers as two separate workloads and sets each one's
role automatically — there is no role setting to choose by hand. Turn each workload on or off
with a single toggle:

| Setting | Helm value | Default | Description |
|---|---|---|---|
| Deploy the writer | `writer.enabled` | `true` | Runs the workload that synchronizes the directory into the shared store. |
| Deploy the readers | `reader.enabled` | `true` | Runs the workload that serves the REST and gRPC APIs. |
| Reader replicas | `reader.replicaCount` | `1` | Number of reader replicas when `autoscaling.enabled` is false. |

The recommended synchronized-directory deployment enables both. Disabling one role supports
split installations — for example, running readers only when another installation already owns
the shared store, or running a writer only to keep the store in sync. The writer always runs as
a single replica and its replica count is not configurable, because a full synchronization is a
single sequential stream serialized by the database lock described above; extra writers cannot
speed it up and would only add load. Scale read capacity instead with `reader.replicaCount`, or set
`autoscaling.enabled` to hand the reader count to an externally managed autoscaler.

Synchronization itself is turned on and shaped under `providers.cache`. Enabling the cache is
what makes the deployment a synchronized-directory deployment; the same block selects which
categories are synchronized and how often, using the settings already described in
{ref}`What Gets Synchronized <userinfo-service-directory-synchronization-what-gets-synchronized>`,
{ref}`Synchronization Interval and Startup Jitter <userinfo-service-directory-synchronization-interval-and-jitter>`,
and {ref}`How Readers Load and Refresh the Replica <userinfo-service-directory-synchronization-how-readers-refresh>`:

```yaml
writer:
  enabled: true

reader:
  enabled: true
  replicaCount: 1
  reloadInterval: "60s"

providers:
  cache:
    enabled: true
    users:
      enabled: true
    groups:
      enabled: true
    memberships:
      enabled: false
    syncInterval: "5m"
    syncJitterMax: "0s"
```

Two related topics affect synchronization even though they are not part of the `cache` block:

- **Readiness during the first synchronization.** The writer reports ready only after it holds
  a complete directory, which on a long initial full synchronization — as membership
  synchronization can make it — may take hours. The probes need no change for this: a failing
  readiness probe never restarts a container and only keeps a pod out of Service endpoints, the
  writer is not part of the Service, and the writer's liveness probe (`livenessProbe`, on
  `/healthz`) keeps passing while it synchronizes. Readers become ready without waiting for the
  writer and serve an empty or partial directory until the complete directory has been saved and
  reloaded; see
  {ref}`Readiness and Health Signals <userinfo-service-observability-readiness>` for how to
  confirm that data has reached the readers.
- **Scoped synchronization.** To narrow synchronization to the users and groups assigned to a
  single enterprise application, enable it under `appRoleAssignments`; the writer then reads the
  application's assignments instead of the whole tenant. See
  {ref}`Directory Scoping <userinfo-service-directory-scoping-directory-scoping>`.

For the complete chart values, including the identity-provider and shared-store settings that a
synchronized deployment also requires, see
{ref}`User Info Service Configuration <userinfo-service-deployment-configuration>` and {ref}`Database Configuration <userinfo-service-database-database-configuration>`.

(userinfo-service-directory-synchronization-live-mode)=
## Live Mode

For deployments that do not use a shared store, a reader can run in a live mode that sends every
request straight to the identity provider. In this mode nothing is synchronized: there is no
writer, no shared store, and no replica to refresh. Every request reflects the provider's current
data, but availability and latency are tied directly to the provider, and the provider's rate
limits apply to live request traffic rather than to background synchronization. This mode is
intended for initial testing rather than production use.

In the Helm chart, live mode is a matter of turning synchronization off: set
`providers.cache.enabled` to `false`, disable the writer with `writer.enabled: false`, and drop
the shared store with `providers.cache.postgres.enabled: false`. Only the readers remain, each
answering requests directly from the identity provider. See
{ref}`User Info Service Configuration <userinfo-service-deployment-configuration>` for the full live-mode values.

(userinfo-service-directory-synchronization-references)=
## References

- [Microsoft Graph REST API][graph] — the interface the service reads directory data through.
- [Use delta query to track changes in Microsoft Graph data][delta-overview] — the
  change-tracking model, next-page and delta links, state tokens, removed-entity markers, replays,
  and delta-link expiry.
- [`user: delta`][user-delta] — tracking changes to users.
- [`group: delta`][group-delta] — tracking changes to groups, including membership changes with
  `$select=members`.
- [List app role assignments granted for a service principal][app-role-assigned-to] — the call
  used when the directory is scoped to an enterprise application.
- [Liveness, Readiness, and Startup Probes][k8s-probes] — a failed readiness probe keeps the
  container running and only removes the pod from Service endpoints; restarts come from liveness
  and startup probe failures.

[graph]: https://learn.microsoft.com/en-us/graph/use-the-api
[delta-overview]: https://learn.microsoft.com/en-us/graph/delta-query-overview
[user-delta]: https://learn.microsoft.com/en-us/graph/api/user-delta
[group-delta]: https://learn.microsoft.com/en-us/graph/api/group-delta
[app-role-assigned-to]: https://learn.microsoft.com/en-us/graph/api/serviceprincipal-list-approleassignedto
[k8s-probes]: https://kubernetes.io/docs/concepts/configuration/liveness-readiness-startup-probes/

(userinfo-service-directory-synchronization-see-next)=
## See next

- {ref}`Microsoft Entra ID Integration <userinfo-service-identity-provider-microsoft-entra-id-integration>`
