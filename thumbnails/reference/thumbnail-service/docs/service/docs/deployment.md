(thumbnail-service-deployment-thumbnail-service-configuration)=
# Thumbnail Service Configuration

**Goal**: Deploy the Thumbnail Service in Kubernetes. One Helm chart renders the **orchestrator**, which serves
the REST and gRPC APIs and consumes storage events, and — by default — a second **task-worker** Deployment,
which converts images. Set `thumbnailWorker.enabled: false` to install the orchestrator only. This guide covers
the prerequisites the Notification Service needs before the first rollout, the values every environment must
set, installation, the rollout ordering the two components require, and rollback.

(thumbnail-service-deployment-service-ports)=
## Service Ports

| Port  | Protocol  | Component    | Description                                                            |
|-------|-----------|--------------|------------------------------------------------------------------------|
| 3000  | HTTP/REST | orchestrator | REST API, `GET /health` (readiness), `GET /health/live` (liveness)     |
| 50051 | gRPC      | orchestrator | gRPC API, `grpc.health.v1.Health`, reflection, discovery `ListServices` |
| 8080  | HTTP      | task-worker  | `/healthz`, `/readyz`, and `/metrics` only — the worker serves no API   |

Only the orchestrator is exposed through a Kubernetes `Service` (ports named `rest` and `grpc`). The worker is
reached by nothing: it pulls work from its queue and calls out to storage and to the orchestrator's in-cluster
Service.

(thumbnail-service-deployment-pulling-the-thumbnail-service-helm-chart)=
## Pulling the Thumbnail Service Helm Chart

Pull and unpack the Thumbnail Service Helm chart from the NGC catalog. Replace `{NGC_API_KEY}` with your NGC API
key and `{VERSION}` with the chart version you want to install.

```bash
# pull the chart from NGC
helm fetch https://helm.ngc.nvidia.com/nvidia/omniverse/charts/thumbnail-service-{VERSION}.tgz --username='$oauthtoken' --password=${NGC_API_KEY}

# unpack the chart and cd into the directory
tar -xvf thumbnail-service-{VERSION}.tgz
cd thumbnail-service
```

(thumbnail-service-deployment-prerequisites-before-the-first-rollout)=
## Prerequisites before the first rollout

Provision these in order. The ordering is not a nicety: publishing an event to the Notification Service
succeeds whether or not any queue is bound to its event type, so an orchestrator that dispatches generate
events into an environment without a worker queue acknowledges every storage event for work that is silently
dropped.

1. **Grants for the service identity.** The principal both components authenticate as needs:
   - `event-aggregation-service:publish-event` on `EventType:omni.thumbnail.generate`
   - `event-consumer-service:create-durable-queues`
   - `event-consumer-service:consume-durable-queues`

   plus read access to the watched containers, write access to every destination the path template can render,
   and delete access to those destinations for the storage-deleted sweep. See
   {ref}`Permissions the service identity needs <thumbnail-service-authentication-permissions-the-service-identity-needs>`.
2. **WorkerEventQueue.** The worker self-provisions its queue on startup, so deploying the release is normally
   enough — but confirm it exists and is consuming **before** rolling out an orchestrator image that publishes
   generate events. See {ref}`Rollout ordering and upgrades <thumbnail-service-deployment-rollout-ordering-and-upgrades>`.
3. **Recursion prevention.** Generated thumbnails land back in watched storage and their `omni.storage.created`
   events return to the orchestrator. The service skips any address whose shape matches the rendered path
   template, so verify in the target environment that a thumbnail written by the worker does not enqueue another
   generation. See {ref}`Generated-output recursion prevention <thumbnail-service-destination-path-template-generated-output-recursion-prevention>`.

(thumbnail-service-deployment-configuring-the-chart)=
## Configuring the chart

Values are split into three groups: **root** keys hold runtime configuration shared by both Deployments;
`thumbnailService:` and `thumbnailWorker:` hold only workload settings (image repository, replicas, resources,
autoscaling, probes, extra environment variables). There is no `common:` wrapper.

By default the chart renders both Deployments. Set `thumbnailWorker.enabled: false` to render the orchestrator
only — for example, an orchestrator-only values file with no other `thumbnailWorker.*` overrides.
`storage.endpoint` and `notifications.eventConsumer.endpoint` default to their canonical in-cluster Service
names (`storage-service:8011` and `event-consumer-service:50051`); override them only if your cluster names
those Services differently, or to reach an external endpoint. `notifications.eventConsumer.resourcePrefixes`
has no usable default: when the worker is enabled (the default), the chart fails at render time until it
names at least one storage-container prefix to watch.

Create a `thumbnail-values.yaml` file with the image-pull secret, the dependency endpoints, the storage
containers to watch, and the thumbnail policy:

```bash
# create and edit the local values file, we're using VS Code for this example
code thumbnail-values.yaml
```

```{code-block} yaml
:caption: thumbnail-values.yaml

image:
  pullSecrets:
    - name: ngcpull-secret

storage:
  # gRPC endpoint of the Storage service, shared by both components
  endpoint: "storage-service:8011"

notifications:
  eventConsumer:
    # Event Consumer API, shared by StorageEventQueue and WorkerEventQueue
    endpoint: "event-consumer-service:50051"
    # Storage-container URL prefixes to watch. Both queue ids are derived from this list.
    resourcePrefixes:
      - "https://<account>.blob.core.windows.net/<container>/"
    storageEventQueue:
      enabled: true
  eventAggregation:
    enabled: true
    endpoint: "event-aggregation-service:50051"

# Where every generated thumbnail is written. Required; no default.
thumbnailPathTemplate: "https://<account>.blob.core.windows.net/<container>/thumbnails/{resource_address_md5}/{resource_identity_md5}/{width}x{height}/q{quality}/{resource_filename}.auto.{output_encoding}"

# One thumbnail per entry for every eligible source
thumbnailSizes: "256x256,512x512"

# Defaults injected when a request or event omits a processing field
thumbnailConfig:
  encodingFormat: "PNG"
  transformType: "FIT"
  cropStrategy: "SMART"
  quality: "85"
  preserveAspectRatio: "true"

thumbnailService:
  image:
    repository: "nvcr.io/nvidia/omniverse/thumbnail-service"

thumbnailWorker:
  image:
    repository: "nvcr.io/nvidia/omniverse/thumbnail-task-worker"
```

Notes on the values:

- **`notifications.eventConsumer.resourcePrefixes`** takes storage **container roots**, not the thumbnail
  output path. Each entry becomes one greedy-prefix filter on both durable queues, so one deployment can watch
  several containers. The chart trims and drops blank entries, sorts the list, and joins it into
  `NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES`; the binaries derive both queue ids from it. See
  {ref}`Durable queue self-provisioning <thumbnail-service-event-consumption-durable-queue-self-provisioning>`.
- **`thumbnailPathTemplate`** must identify its source with `{resource_address_md5}` alone or with both
  `{resource_path}` and `{resource_filename}`, and must render an absolute storage address. Helm only checks
  that it is non-empty; the orchestrator fails fast at startup if the template is malformed. See
  {ref}`Thumbnail destination path template <thumbnail-service-destination-path-template-thumbnail-destination-path-template>`.
- **`thumbnailSizes`** entries are `WxH` pairs, each dimension `1..=4096`, at most ten entries, no
  duplicates. Every eligible storage event pays for every size.
- **`thumbnailConfig.transformType: FIT`** scales the whole source into the requested box and pads the
  remainder — transparent for PNG output, opaque white for JPEG — so nothing is cut off. `cropStrategy` is
  inert while the transform is `FIT`; it applies only to `CROP` and `SCALE_AND_CROP`.

(thumbnail-service-deployment-service-identity)=
### Service identity

Both Deployments call other Omniverse services on their own behalf — the orchestrator publishes to the
Notification Service and provisions its durable queue, the worker reads and writes storage blobs and calls
back into the orchestrator — so both need a bearer token of their own, obtained through **exactly one** of two
mutually exclusive modes, configured once at the chart root under `serviceIdentity`.

Prefer `serviceIdentity.accessTokenFile`: a sidecar provisions the token from your cluster's own workload
identity, Vault agent, or cloud metadata client, so no secret material lives in this chart's values or in the
service containers. Reserve `serviceIdentity.clientCredentials` — a long-lived client secret stored in a
Kubernetes `Secret` — for local development and testing.

```{code-block} yaml
:caption: thumbnail-values.yaml

# access-token-file mode: swap the sidecar image/config for whatever token-provisioning
# container your cluster uses (Azure Workload Identity shown here)
serviceIdentity:
  accessTokenFile:
    enabled: true
    provisionerSidecar:
      container:
        image: "<registry>/client-credentials-sidecar:<tag>"
        env:
          - name: TOKEN_OUTPUT_PATH
            value: "/etc/secrets/access-token/token"
        resources:
          requests:
            cpu: 10m
            memory: 32Mi
```

The chart **does not create Secrets**: for client-credentials mode, `clientSecretRef.name` must name an
existing Kubernetes `Secret`, which both pods mount read-only at `/etc/secrets`. For the full values reference
of both modes, see
{ref}`Outbound service identity (serviceIdentity) <thumbnail-service-authentication-outbound-service-identity-serviceidentity>`.

(thumbnail-service-deployment-render-time-validation)=
### Render-time validation

`helm template` and `helm install` fail before anything reaches the cluster when:

- `storage.endpoint`, `notifications.eventConsumer.endpoint`, `thumbnailPathTemplate`, `thumbnailSizes`, or
  any of the five `thumbnailConfig.*` values is empty (the two endpoints default to their canonical
  in-cluster Service names, so this only triggers on an explicit blank override);
- the storage-event consumer or the worker is enabled and `resourcePrefixes` has no non-blank entry;
- neither or both `serviceIdentity` modes are enabled, client-credentials mode has no `clientSecretRef.name`,
  or access-token-file mode has no `provisionerSidecar.container`;
- the worker's `resources.limits.memory` is missing, not an integer `Ki`/`Mi`/`Gi` quantity, or below
  `workerRuntime.maxConcurrentConversions × workerRuntime.maxTransformPeakBytes + 200Mi` (see
  {ref}`Scaling <thumbnail-service-deployment-scaling>`);
- `workerRuntime.maxConcurrentConversions`, `maxDecodedPixels`, or `maxTransformPeakBytes` is zero.

The binaries repeat the identity and template checks at startup as a second line of defense.

(thumbnail-service-deployment-install-the-thumbnail-service)=
## Install the Thumbnail Service

Validate and install the chart:

```bash
# validate the chart
helm template . -f thumbnail-values.yaml

# dry-run the install
helm upgrade --install thumbnail-service . -f thumbnail-values.yaml --namespace storage-apis --dry-run --debug
```

If everything looks good, install the Thumbnail Service:

```bash
# install the thumbnail service
helm upgrade --install thumbnail-service . -f thumbnail-values.yaml --namespace storage-apis

# validate both pods are running; the first start waits up to 60s for a service-identity token
kubectl get pods -n storage-apis
```

Verify the orchestrator is responding and the worker is consuming:

```bash
# readiness: 200 only once both the HTTP and gRPC listeners are serving
curl http://thumbnail-service.storage-apis.svc.cluster.local:3000/health

# the worker is Ready only while its WorkerEventQueue consume stream is established
kubectl get deployment -n storage-apis -l app.kubernetes.io/component=task-worker
```

```{note}
Both Deployments ship a startup probe with a ~125 s budget because each process waits up to 60 s for its first
service-identity bearer token before serving. A slow identity provider or a late-starting token sidecar shows
up as a long `ContainerCreating`/`Running` without `Ready`, not as a crash; a bearer that never arrives ends in
`CrashLoopBackOff` with the token-wait error in the logs.
```

(thumbnail-service-deployment-rollout-ordering-and-upgrades)=
## Rollout ordering and upgrades

Complete the **worker** rollout before exposing new **orchestrator** behavior. A single `helm upgrade` of the
fused chart starts both Deployments together and does not establish this ordering by itself.

1. Upgrade with the worker's new image and confirm WorkerEventQueue exists and is being consumed — the worker
   pods report `Ready` only while their consume stream is established, and
   `thumbnail_worker_unacknowledged_deliveries` on `/metrics` moves when work arrives.
2. Roll the orchestrator. It publishes generate events only once it starts; every event it publishes before a
   consumer is bound is acknowledged upstream and lost.
3. Upload a small image into a watched container and confirm a thumbnail is generated and readable through
   `GET /v1alpha/thumbnails`.

When `notifications.eventConsumer.resourcePrefixes` changes, **both** queue ids change on the next startup: each
component provisions a fresh queue carrying the new filters, and work held on the superseded queue is not
consumed. Treat a prefix change as a deliberate one-time cutover.

(thumbnail-service-deployment-rollback)=
## Rollback

Roll the orchestrator back first, then drain the worker:

```bash
# stop new generate work by rolling the orchestrator back
helm rollback thumbnail-service <REVISION> -n storage-apis

# wait until the worker has settled every delivery it already holds
kubectl port-forward -n storage-apis deploy/thumbnail-service-thumbnail-task-worker 8080:8080
curl -s localhost:8080/metrics | grep thumbnail_worker_unacknowledged_deliveries

# only then scale or roll the worker
kubectl scale deployment -n storage-apis thumbnail-service-thumbnail-task-worker --replicas=0
```

Never delete WorkerEventQueue during a rollback; unacknowledged work is redelivered to the next consumer, and the
worker re-provisions a queue that went missing without a restart.

(thumbnail-service-deployment-ingress-access)=
## Ingress Access

To expose the Thumbnail Service outside the cluster, enable the root `httpProxy` section in your values file.
The chart creates [Contour HTTPProxy](https://projectcontour.io/docs/) resources for both the REST and gRPC
endpoints.

```{code-block} yaml
:caption: thumbnail-values.yaml

httpProxy:
  enabled: true
  fqdn:
    host: "thumbnails"
    domain: "{DNS_URL}"
  tls:
    enabled: true
```

Redeploy the Thumbnail Service with the updated values:

```bash
helm upgrade --install thumbnail-service . -f thumbnail-values.yaml --namespace storage-apis
```

Verify the HTTPProxy resources were created:

```bash
kubectl get httpproxy -n storage-apis
```

This creates two endpoints: REST at `thumbnails.{DNS_URL}` and gRPC at `grpc.thumbnails.{DNS_URL}` (the gRPC
route upstreams over `h2c` to the in-cluster port). TLS terminates at Contour — the components themselves
speak plaintext HTTP/2 inside the cluster and refuse `https://` dependency endpoints at startup. For full
details on setting up Contour as a load balancer, configuring DNS records, and enabling TLS, see the
[Ingress Configuration](https://docs.omniverse.nvidia.com/ovstorage/ovstorage-guide/latest/dev-journey/full-deployment/ingress.html)
guide.

(thumbnail-service-deployment-scaling)=
## Scaling

The two Deployments scale on different signals and are sized independently.

| | orchestrator (`thumbnailService`) | task-worker (`thumbnailWorker`) |
|---|---|---|
| Default replicas | `1` | `2` |
| Autoscaling | off (`minReplicas: 1`, `maxReplicas: 10`, 80% CPU when enabled) | **on** (`minReplicas: 2`, `maxReplicas: 20`, 70% CPU) |
| Default requests / limits | `250m` / `64Mi` → `1` / `256Mi` | `1300m` / `750Mi` → `2500m` / `2304Mi` |
| Scaling signal | request and event volume | conversion CPU |

`thumbnailWorker.replicaCount` (or the HPA) is the "number of workers" knob: the worker is single-process and
scales horizontally, one blast radius per pod for image decode. `workerRuntime.maxConcurrentConversions`
(default `2`) controls how much each replica does at once, and the memory limit is derived from it:

```text
limits.memory  ≥  maxConcurrentConversions × maxTransformPeakBytes + ~200 MiB idle
                  2 × 1 GiB + 200 MiB ≈ 2.2 GiB  →  2304Mi
```

The chart refuses to render a combination the limit cannot cover, so raising concurrency or
`maxTransformPeakBytes` without raising the limit is an install error rather than a silent OOM. Anything over the
budget at runtime is a permanent conversion failure with a clear reason, not a pod restart. For the reasoning
behind the budget and the CPU figures, see {ref}`Sizing <thumbnail-service-generation-pipeline-sizing>`.

```{note}
In access-token-file mode the provisioner sidecar carries no default resources. Both HPAs use the
`ContainerResource` metric type, which targets the orchestrator or worker container by name rather than
aggregating across the whole pod, so a sidecar with no CPU request does not affect autoscaling.
```

(thumbnail-service-deployment-clean-up)=
## Clean Up

To clean up: uninstall the Helm release, delete the secrets you created, and remove the namespace.

```bash
# uninstall the thumbnail service (both Deployments)
helm uninstall thumbnail-service -n storage-apis

# delete the client-credentials secret if you created one
kubectl delete secret thumbnail-service-client-credentials -n storage-apis

# delete the namespace
kubectl delete namespace storage-apis

# validate the namespace was deleted
kubectl get namespace storage-apis
```

Uninstalling does not delete the two durable queues on the Notification Service; unacknowledged work stays
queued for the next consumer with the same id.

(thumbnail-service-deployment-references)=
## References

- [Contour HTTPProxy](https://projectcontour.io/docs/) — ingress resource used by the Helm chart
- [Ingress Configuration](https://docs.omniverse.nvidia.com/ovstorage/ovstorage-guide/latest/dev-journey/full-deployment/ingress.html) — full guide for Contour, DNS, and TLS setup
- [Azure Workload Identity](https://azure.github.io/azure-workload-identity/docs/) — the platform the access-token-file mode is designed around
- [OpenTelemetry Collector](https://opentelemetry.io/docs/collector/) — receives the metrics and traces configured under `observability.tracing` / `observability.metrics`

(thumbnail-service-deployment-what-s-next)=
## What's Next?

Continue your deployment by configuring the
[Discovery Service](https://docs.omniverse.nvidia.com/ovstorage/ovstorage-guide/latest/dev-journey/full-deployment/discovery-service.html)
so clients can find and connect to all services via a single endpoint; the chart annotates its Service for the
discovery operator by default.

Then review {ref}`Environment configuration reference <thumbnail-service-configuration-environment-configuration-reference>` for every variable the chart injects,
and {ref}`Observability <thumbnail-service-observability-observability>` to point metrics and traces at your collector.
