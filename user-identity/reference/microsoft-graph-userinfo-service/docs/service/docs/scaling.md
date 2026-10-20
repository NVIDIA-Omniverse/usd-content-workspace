(userinfo-service-scaling-scaling-and-high-availability)=
# Scaling and High Availability

User Info Service separates the work of keeping the directory current from the work of serving
requests, and this shapes how it scales and how available it is. The read path scales out simply
by running more readers; the write path runs as a single instance and is deliberately not scaled.
This page describes the scaling model for each role, how the two roles coordinate through the
shared store, how the deployment behaves when a component is lost, how to size memory and CPU,
and how to hand the reader replica count to an external autoscaler.

For the roles themselves and how data reaches the readers, see
{ref}`Directory Synchronization and Data Freshness <userinfo-service-directory-synchronization-directory-synchronization-and-data-freshness>`; for the shared
store the two roles use, see {ref}`Database Configuration <userinfo-service-database-database-configuration>`.

(userinfo-service-scaling-scaling-model)=
## Reader and Writer Scaling Model

The service runs as two separate workloads with opposite scaling characteristics:

- **Readers** serve the REST and gRPC APIs and scale **horizontally**. Each reader holds its own
  in-memory copy of the directory, so adding replicas adds read capacity without any coordination
  between them. This is the dimension operators scale.
- The **writer** keeps the shared store aligned with the identity provider and runs as a **single
  instance** that is not scaled. It is off the request path, so it does not contribute to read
  capacity.

Because the two roles are independent workloads, they are sized and scaled independently: the
reader count follows request volume, while the single writer is sized only for the cost of
synchronizing the directory.

(userinfo-service-scaling-read-capacity)=
## Scaling Read Capacity

Every reader loads the stored directory into memory on startup and reloads it on its refresh
interval, then answers every request from that in-memory copy. Readers never contact each other
and never contact the identity provider for synchronization, so a request served by one reader
takes no coordination with any other. Read capacity therefore grows roughly linearly with the
number of readers.

The Kubernetes Service created by the chart selects **reader pods only**, and routes both the REST
and gRPC ports to them. Traffic is balanced across the ready readers, so a request may land on any
reader; because each reader refreshes on its own schedule, two readers can briefly return slightly
different data for the same request, as described in
{ref}`Data Freshness and Eventual Consistency <userinfo-service-directory-synchronization-data-freshness>`.

Set the number of readers with `reader.replicaCount`, or hand the count to an external autoscaler
as described in {ref}`Horizontal Autoscaling <userinfo-service-scaling-autoscaling>`. Each
additional reader opens its own connections to the shared store, so scaling readers out also
raises the store's connection load; size the database's connection limit for the whole fleet as
described in
{ref}`Connection Budget and Scaling <userinfo-service-database-connection-budget-and-scaling>`.

(userinfo-service-scaling-single-writer)=
## Why the Writer Runs as a Single Instance

A full synchronization rebuilds the stored directory from a single sequential stream of changes
read from the identity provider, and only one may run at a time. The writer coordinates this
through an advisory lock in the shared database, so even if a second writer were started it could
not run a full synchronization concurrently — it would only duplicate identity-provider and
database load while waiting for the lock. Running more than one writer therefore cannot speed
synchronization up. See
{ref}`Coordinating a Single Full Synchronization <userinfo-service-directory-synchronization-single-full-sync>`
for the locking model.

For this reason the writer's replica count is fixed at one by the chart and is not configurable.
The writer Deployment also uses the `Recreate` update strategy, which stops the existing writer
pod before starting the replacement, so a rollout never briefly runs two writers at once.

(userinfo-service-scaling-coordination)=
## How Replicas Coordinate Through the Shared Store

The only thing the writer and the readers share is the PostgreSQL store. The writer persists the
synchronized directory into it; each reader independently reloads from it. There is no direct
communication between the writer and the readers, and none between one reader and another — no
leader election, no request-time coordination, and no cross-replica cache invalidation. The store
is the single point through which a change made by the writer eventually reaches every reader.

This indirection is what makes the read path easy to scale and keeps it available: readers do not
depend on the writer being reachable, only on the store holding a directory they can load. The
cost is that the service is eventually consistent — a change is visible on a given reader only
after the writer has saved it and that reader has reloaded. The freshness bounds this implies are
described in
{ref}`Data Freshness and Eventual Consistency <userinfo-service-directory-synchronization-data-freshness>`.

(userinfo-service-scaling-high-availability)=
## High Availability

Availability of each part of the service follows from the scaling model above.

- **Read path.** Run more than one reader for a highly available API. The Service routes only to
  ready readers, so losing a single reader pod — to a node failure, an eviction, or a rolling
  update — does not interrupt the API as long as another reader stays ready. Spreading readers
  across nodes with `affinity` improves resilience to node loss; the chart does not set a default
  anti-affinity rule or a PodDisruptionBudget, so an operator that needs those guarantees supplies
  them through the chart's `affinity` value and their own PodDisruptionBudget.
- **Writer outage.** Because the writer is off the request path, a writer that is down, restarting,
  or rolling out does not affect read availability: readers keep serving the last complete
  directory they hold in memory. Only data freshness pauses — the directory stops advancing until
  the writer returns — after which synchronization resumes from its saved progress, as described in
  {ref}`Recovering from Interruptions <userinfo-service-directory-synchronization-recovering-from-interruptions>`.
- **Store outage.** A reader that is already serving keeps serving its in-memory copy when the
  store is briefly unreachable, rather than dropping to an empty directory. A reader that *starts*
  while the store is unreachable exits and is restarted by Kubernetes until it can connect, so it
  never becomes ready. A reader that starts against a reachable store that holds no directory yet
  becomes ready and serves an empty directory until the writer has saved data and a later reload
  picks it up. For a highly available deployment, make the PostgreSQL store highly available as
  well; see {ref}`Database Configuration <userinfo-service-database-database-configuration>`.
- **Live mode.** The reader-only live mode has no writer and no store, so every request depends on
  the identity provider directly and its availability and latency are tied to the provider. It is
  intended for initial testing rather than for a highly available production deployment; see
  {ref}`Live Mode <userinfo-service-directory-synchronization-live-mode>`.

(userinfo-service-scaling-sizing)=
## Memory and CPU Sizing

Each instance — the writer and every reader — holds the full directory it serves in memory, so
memory is the resource most sensitive to directory size. The largest factor is whether group
memberships are synchronized: memberships are the most expensive category to hold, and enabling
them raises the memory each instance needs substantially. For a large directory — on the order of
100,000 users and 170,000 groups — the in-memory membership index alone can approach 2.5 GB, on
top of the memory used for users and groups. Directories without memberships need considerably
less.

The chart's default reader and writer requests and limits (`reader.resources`, `writer.resources`)
suit a small to moderate directory. Raise them when serving a large directory, and raise them
further when membership synchronization is enabled, so the limit comfortably exceeds the resident
directory size. Because every reader holds an independent copy, memory use across the fleet is the
per-instance figure multiplied by the reader count; horizontal scaling trades total memory for
read capacity and availability.

Startup time is also sensitive to directory size. When the store does not yet hold a complete
directory, the writer reports ready only after its first full synchronization completes, which can
take hours for a large directory with memberships. A long first synchronization does not need any
probe change: a failing readiness probe only keeps a pod out of Service endpoints and never
restarts the container, the writer is not part of the Service, and the writer's liveness probe
keeps passing while it synchronizes. Readers become ready in seconds by loading whatever the store
already holds; until the writer has saved a complete directory, they serve an empty or partial
one. See
{ref}`What Gets Synchronized <userinfo-service-directory-synchronization-what-gets-synchronized>`
for the effect of memberships on synchronization time and memory, and
{ref}`Readiness and Health Signals <userinfo-service-observability-readiness>` for how to confirm
that data has reached the readers.

(userinfo-service-scaling-autoscaling)=
## Horizontal Autoscaling

Reader replicas can be driven by an external autoscaler instead of a fixed count. The
`autoscaling.enabled` value controls only whether the reader Deployment carries a fixed replica
count:

- When `autoscaling.enabled` is `false` (the default), the reader Deployment sets
  `replicas: reader.replicaCount`.
- When `autoscaling.enabled` is `true`, the reader Deployment omits `replicas` entirely, so an
  external controller can own the replica count without the chart fighting it on every upgrade.

The chart does **not** create a HorizontalPodAutoscaler. Enabling `autoscaling` only frees the
replica field; the operator must supply their own HorizontalPodAutoscaler (or equivalent
controller) targeting the reader Deployment. A CPU-based HorizontalPodAutoscaler works with this
chart because the reader container sets a CPU resource request, which is what the autoscaler
divides observed usage against; the writer is never a scaling target.

(userinfo-service-scaling-configuration)=
## Configuration

The settings that shape scaling and sizing are all in the Helm chart.

| Setting | Helm value | Default | Description |
|---|---|---|---|
| Reader replicas | `reader.replicaCount` | `1` | Number of readers when `autoscaling.enabled` is `false`. |
| Externalize the replica count | `autoscaling.enabled` | `false` | Omit `replicas` from the reader Deployment so an external autoscaler owns the count. Does not create a HorizontalPodAutoscaler. |
| Reader resources | `reader.resources` | `requests: cpu 500m, memory 1Gi` / `limits: cpu 1000m, memory 2Gi` | CPU and memory requests and limits for each reader. Raise for large directories and when memberships are enabled. |
| Writer resources | `writer.resources` | `requests: cpu 500m, memory 1Gi` / `limits: cpu 1000m, memory 2Gi` | CPU and memory requests and limits for the single writer. |
| Reader reload interval | `reader.reloadInterval` | `60s` | How often a reader reloads the directory from the store. See {ref}`Directory Synchronization and Data Freshness <userinfo-service-directory-synchronization-directory-synchronization-and-data-freshness>`. |
| Store connection budget | `providers.cache.postgres.connections` | `4` | Connections each instance opens to the store. See {ref}`Connection Budget and Scaling <userinfo-service-database-connection-budget-and-scaling>`. |
| Reader scheduling | `affinity` | `{}` | Pod affinity/anti-affinity for the reader, e.g. to spread readers across nodes for high availability. |

Set the replica bounds and target metric on the HorizontalPodAutoscaler you create yourself; the
chart carries no autoscaling tuning of its own.

(userinfo-service-scaling-helm)=
### Configuring Scaling in the Helm Chart

For a fixed number of readers, set `reader.replicaCount` and size the reader and writer resources
to the directory. The example below runs three readers and raises memory for a directory with
membership synchronization enabled:

```yaml
reader:
  enabled: true
  replicaCount: 3
  resources:
    requests:
      cpu: 500m
      memory: 2Gi
    limits:
      cpu: 1000m
      memory: 4Gi

writer:
  enabled: true
  resources:
    requests:
      cpu: 500m
      memory: 2Gi
    limits:
      cpu: 1000m
      memory: 4Gi
```

To hand the reader count to an external autoscaler instead of pinning it, enable `autoscaling` so
the Deployment omits its replica field, and create your own HorizontalPodAutoscaler targeting the
reader Deployment:

```yaml
autoscaling:
  enabled: true
```

Install or upgrade the release with the same commands used for the synchronized directory
deployment in {ref}`User Info Service Configuration <userinfo-service-deployment-configuration>`.

(userinfo-service-scaling-references)=
## References

- [Horizontal Pod Autoscaling][hpa] — the Kubernetes controller and API object an operator
  creates to drive the reader replica count, and its requirement that the target's containers set
  the resource requests it scales against.
- [Recreate Deployment strategy][recreate] — the writer's update strategy, which stops the
  existing pod before starting its replacement.
- [Liveness, Readiness, and Startup Probes][k8s-probes] — a failed readiness probe keeps the
  container running and only removes the pod from Service endpoints; restarts come from liveness
  and startup probe failures.

[hpa]: https://kubernetes.io/docs/tasks/run-application/horizontal-pod-autoscale/
[recreate]: https://kubernetes.io/docs/concepts/workloads/controllers/deployment/#recreate-deployment
[k8s-probes]: https://kubernetes.io/docs/concepts/configuration/liveness-readiness-startup-probes/

(userinfo-service-scaling-see-next)=
## See next

- {ref}`Observability <userinfo-service-observability-observability>`
