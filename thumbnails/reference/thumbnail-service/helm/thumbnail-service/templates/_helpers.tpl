{{/*
Expand the name of the chart.
*/}}
{{- define "service.name" -}}
{{- default .Chart.Name .Values.thumbnailService.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create a default fully qualified app name.
*/}}
{{- define "service.fullname" -}}
{{- if .Values.thumbnailService.fullnameOverride }}
{{- .Values.thumbnailService.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- $name := default .Chart.Name .Values.thumbnailService.nameOverride }}
{{- if contains $name .Release.Name }}
{{- .Release.Name | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}
{{- end }}

{{/*
Create chart name and version as used by the chart label.
*/}}
{{- define "service.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Common labels
*/}}
{{- define "service.labels" -}}
helm.sh/chart: {{ include "service.chart" . }}
{{ include "service.selectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{/*
Selector labels
*/}}
{{- define "service.selectorLabels" -}}
app.kubernetes.io/name: {{ include "service.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/*
Create a string name to be used in service and headless service
*/}}
{{- define "service.servicename" -}}
{{- $name := default .Chart.Name .Values.service.nameOverride }}
{{- printf "%s" $name | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Default TLS secret name
*/}}
{{- define "service.tlsSecretName" -}}
{{- $defaultname := print "tls-cert-" (include "service.fullname" .) -}}
{{- $name := default $defaultname .Values.httpProxy.tls.secretName }}
{{- printf "%s" $name | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create the name of the service account to use.

Resolution order is component name, then the shared root name, then a generated
one. The root fallback is the part worth stating: `serviceAccount` holds the
create/automount/annotations that both components share, so a values file that
also sets a name there reasonably expects it to apply. Skipping that level meant
a deployment setting only `serviceAccount.name` silently landed on `default` —
and with `create: false` no ServiceAccount is rendered either, so the pod runs as
the namespace default with none of the Workload Identity annotations.
*/}}
{{- define "service.serviceAccountName" -}}
{{- $name := default .Values.serviceAccount.name .Values.thumbnailService.serviceAccount.name }}
{{- if .Values.serviceAccount.create }}
{{- default (include "service.fullname" .) $name }}
{{- else }}
{{- default "default" $name }}
{{- end }}
{{- end }}

{{/*
Existing Kubernetes Secret that holds the OAuth client secret.
This chart does not create Secrets; client-credentials mode mounts this name.
*/}}
{{- define "chart.clientSecretName" -}}
{{- required "serviceIdentity.clientCredentials.clientSecretRef.name is required when clientCredentials.enabled is true: the chart does not create Secrets" .Values.serviceIdentity.clientCredentials.clientSecretRef.name }}
{{- end }}

{{/*
Resolve the secret name mounted by the orchestrator.
*/}}
{{- define "service.secretName" -}}
{{- include "chart.clientSecretName" . }}
{{- end }}

{{/*
Validate the serviceIdentity block at template-time. Asserts that exactly
one of `clientCredentials.enabled` or `accessTokenFile.enabled` is true,
and that access-token-file mode is paired with a provisioner sidecar
container. The thumbnail-service binary also enforces these invariants
at startup (clap `conflicts_with` and OutboundClients::build), but
catching them here surfaces misconfigurations during `helm template` /
`helm install` instead of via CrashLoopBackOff after pod boot. Templates
that render any service-identity helper (env, volume mounts, sidecar,
secret) should `include` this helper near the top.
*/}}
{{- define "service.validateServiceIdentity" -}}
{{- $cc := .Values.serviceIdentity.clientCredentials.enabled -}}
{{- $atf := .Values.serviceIdentity.accessTokenFile.enabled -}}
{{- if and $cc $atf -}}
{{- fail "serviceIdentity: both `clientCredentials.enabled` and `accessTokenFile.enabled` are true; exactly one outbound auth mode must be set." -}}
{{- end -}}
{{- if and (not $cc) (not $atf) -}}
{{- fail "serviceIdentity: neither `clientCredentials.enabled` nor `accessTokenFile.enabled` is true; exactly one outbound auth mode must be set." -}}
{{- end -}}
{{- if and $atf (not .Values.serviceIdentity.accessTokenFile.provisionerSidecar.container) -}}
{{- fail "serviceIdentity: `accessTokenFile.enabled` is true but `accessTokenFile.provisionerSidecar.container` is empty; the sidecar that provisions the token file is required in this mode." -}}
{{- end -}}
{{- if and $cc (not .Values.serviceIdentity.clientCredentials.clientSecretRef.key) -}}
{{- fail "serviceIdentity: `clientCredentials.enabled` is true but `clientCredentials.clientSecretRef.key` is empty; the Secret key (and CLIENT_SECRET_FILE basename) is required." -}}
{{- end -}}
{{- if and $cc (not .Values.serviceIdentity.clientCredentials.clientSecretRef.name) -}}
{{- fail "serviceIdentity: `clientCredentials.enabled` is true but `clientCredentials.clientSecretRef.name` is empty; the chart does not create Secrets." -}}
{{- end -}}
{{- /* Kubernetes Secret data keys: alphanumeric, '-', '_', or '.' (also used as the mounted file basename). */ -}}
{{- if and $cc (not (regexMatch "^[A-Za-z0-9._-]+$" .Values.serviceIdentity.clientCredentials.clientSecretRef.key)) -}}
{{- fail (printf "serviceIdentity: `clientCredentials.clientSecretRef.key` %q is not a valid Kubernetes Secret key (must match ^[A-Za-z0-9._-]+$)" .Values.serviceIdentity.clientCredentials.clientSecretRef.key) -}}
{{- end -}}
{{- end }}

{{/*
notifications.eventConsumer.resourcePrefixes as the comma-separated string both
binaries parse out of NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES, with blank entries dropped.

The values key is documented as a list, but a bare string is accepted: an operator with one
prefix will write it either way, and the two Deployments must agree on the answer. Every site
that reads the list goes through here, so a shape this accepts cannot render on one Deployment
and fail on the other.
*/}}
{{- define "chart.resourcePrefixes" -}}
{{- $raw := .Values.notifications.eventConsumer.resourcePrefixes -}}
{{- $prefixes := list -}}
{{- if kindIs "slice" $raw -}}
{{- range $raw -}}
{{- if trim (toString .) -}}{{- $prefixes = append $prefixes (trim (toString .)) -}}{{- end -}}
{{- end -}}
{{- else if and $raw (trim (toString $raw)) -}}
{{- $prefixes = append $prefixes (trim (toString $raw)) -}}
{{- end -}}
{{- $prefixes = sortAlpha $prefixes -}}
{{- join "," $prefixes -}}
{{- end }}

{{/*
True when notifications.eventConsumer.resourcePrefixes has no non-blank entry.
*/}}
{{- define "chart.resourcePrefixesEmpty" -}}
{{- if not (include "chart.resourcePrefixes" .) }}true{{- end -}}
{{- end }}

{{/*
Orchestrator-side required-field checks, including root-level storage.endpoint.
StorageEventQueue's id is derived at runtime from resource prefixes.
*/}}
{{- define "service.validate" -}}
{{- if not .Values.storage.endpoint }}
{{- fail "storage.endpoint is required: both the orchestrator and worker talk to storage over this endpoint" }}
{{- end }}
{{- if and .Values.notifications.eventConsumer.storageEventQueue.enabled (include "chart.resourcePrefixesEmpty" .) }}
{{- fail "notifications.eventConsumer.resourcePrefixes is required when notifications.eventConsumer.storageEventQueue.enabled=true: an empty resource-prefix filter binds a queue that receives nothing while every call still reports success" }}
{{- end }}
{{- end }}

{{/*
Service-identity env block. Emits the canonical CLIENT_* env vars when the
in-process client-credentials flow is enabled, OR the
ACCESS_TOKEN_FILE_* env vars when the sidecar-driven flow is enabled.
The two modes are mutually exclusive; `service.validateServiceIdentity`
asserts exactly one of the two `.enabled` flags is set. Mirrors
permission-service's deployment template lines 211-234.
Shared by both the orchestrator and the worker (same external principal).
*/}}
{{- define "service.serviceIdentityEnv" -}}
{{- if .Values.serviceIdentity.clientCredentials.enabled }}
- name: ENABLE_CLIENT_CREDENTIALS
  value: "true"
- name: CLIENT_CREDENTIALS_OPENID_CONFIGURATION
  value: {{ .Values.serviceIdentity.clientCredentials.openIdConfigurationUri | quote }}
- name: CLIENT_ID
  value: {{ .Values.serviceIdentity.clientCredentials.clientId | quote }}
{{- if .Values.serviceIdentity.clientCredentials.clientScope }}
- name: CLIENT_SCOPE
  value: {{ .Values.serviceIdentity.clientCredentials.clientScope | quote }}
{{- end }}
- name: CLIENT_SECRET_FILE
  value: "/etc/secrets/{{ .Values.serviceIdentity.clientCredentials.clientSecretRef.key }}"
{{- if .Values.serviceIdentity.clientCredentials.extraHeaders }}
- name: CLIENT_CREDENTIALS_EXTRA_HEADERS
  value: {{ .Values.serviceIdentity.clientCredentials.extraHeaders | toJson | quote }}
{{- end }}
{{- if .Values.serviceIdentity.clientCredentials.extraBodyParams }}
- name: CLIENT_CREDENTIALS_EXTRA_BODY_PARAMS
  value: {{ .Values.serviceIdentity.clientCredentials.extraBodyParams | toJson | quote }}
{{- end }}
{{- end }}
{{- if .Values.serviceIdentity.accessTokenFile.enabled }}
- name: ENABLE_ACCESS_TOKEN_FILE
  value: "true"
- name: ACCESS_TOKEN_FILE_PATH
  value: {{ .Values.serviceIdentity.accessTokenFile.path | quote }}
- name: ACCESS_TOKEN_FILE_POLL_INTERVAL
  value: {{ .Values.serviceIdentity.accessTokenFile.pollInterval | quote }}
{{- end }}
{{- end }}

{{/*
Service-identity volume mounts for the in-process client-credentials flow
(the secret containing CLIENT_SECRET) and/or the sidecar access-token file.
Shared mount paths for both components; volume *names* stay per-pod.
*/}}
{{- define "service.serviceIdentityVolumeMounts" -}}
{{- if .Values.serviceIdentity.clientCredentials.enabled }}
- name: service-identity-secret
  mountPath: /etc/secrets
  readOnly: true
{{- end }}
{{- if .Values.serviceIdentity.accessTokenFile.enabled }}
- name: token-exchange
  mountPath: /etc/secrets/access-token
  readOnly: true
{{- end }}
{{- end }}

{{/*
Shared env vars used by the orchestrator HTTP/gRPC process.
*/}}
{{- define "service.sharedRuntimeEnv" -}}
# Storage Service Endpoint (root-level; shared with the worker)
- name: STORAGE_GRPC_ENDPOINT
  value: {{ required "storage.endpoint is required (e.g. storage-service:8011)" .Values.storage.endpoint | quote }}
# Thumbnail dimensions to generate per source asset (chart default exists; overrides must stay non-empty)
- name: THUMBNAIL_SIZES
  value: {{ required "thumbnailSizes must resolve to a non-empty WxH comma-separated list (chart default: \"256x256,512x512\"), e.g. \"256x256\" or \"64x64,128x128,256x256\"" .Values.thumbnailSizes | quote }}
# Operator-owned destination policy. No fixed-address fallback exists after cutover.
- name: THUMBNAIL_PATH_TEMPLATE
  value: {{ required "thumbnailPathTemplate is required and must resolve to an absolute storage address" .Values.thumbnailPathTemplate | quote }}
# Required thumbnail processing config used when fields are omitted from request/event contracts.
- name: THUMBNAIL_ENCODING_FORMAT
  value: {{ required "thumbnailConfig.encodingFormat is required (e.g. PNG/JPEG/GIF/WEBP)" .Values.thumbnailConfig.encodingFormat | quote }}
- name: THUMBNAIL_TRANSFORM_TYPE
  value: {{ required "thumbnailConfig.transformType is required (e.g. SQUARE/CROP/FIT/...)" .Values.thumbnailConfig.transformType | quote }}
- name: THUMBNAIL_CROP_STRATEGY
  value: {{ required "thumbnailConfig.cropStrategy is required (e.g. SMART/CENTER/TOP/...)" .Values.thumbnailConfig.cropStrategy | quote }}
- name: THUMBNAIL_QUALITY
  value: {{ required "thumbnailConfig.quality is required (1..100)" .Values.thumbnailConfig.quality | quote }}
- name: THUMBNAIL_PRESERVE_ASPECT_RATIO
  value: {{ required "thumbnailConfig.preserveAspectRatio is required (true/false)" .Values.thumbnailConfig.preserveAspectRatio | quote }}
{{- include "service.serviceIdentityEnv" . }}
- name: RUST_LOG
  value: {{ .Values.observability.logging.level | quote }}
- name: OTEL_LOGS_EXPORTER
  value: "console"
{{- include "service.otelEnv" . }}
{{- end }}

{{/*
Reject the removed observability.otel.* block.

Helm ignores value keys it does not recognize, so an environment values file still
carrying the old block would render OTEL_TRACES_EXPORTER=none and
OTEL_METRICS_EXPORTER=none and silently stop exporting everything, with no error
anywhere. Failing at render turns that into a migration message.
*/}}
{{- define "service.validateObservability" -}}
{{- if hasKey .Values.observability "otel" -}}
{{- fail (printf "%s\n%s\n%s\n%s\n%s\n%s"
  "observability.otel.* has been replaced by observability.tracing.* and observability.metrics.*, which are independent signals."
  "Helm ignores unknown value keys, so leaving observability.otel set disables all OTLP export silently. Migrate it:"
  "  observability.otel.endpoint         -> observability.tracing.endpoint.grpc and/or observability.metrics.endpoint.grpc"
  "  observability.otel.tracesSamplerArg -> observability.tracing.samplingRatio"
  "  observability.otel.resourceAttributes -> observability.resourceAttributes"
  "  observability.otel.tracesExporter / .metricsExporter / .tracesSampler have no replacement: a signal exports when its endpoint is set and its `enabled` flag is true (both default true), and the sampler is always parentbased_traceidratio.") -}}
{{- end -}}
{{- end }}

{{/*
OTLP observability env vars.

Tracing and metrics are independent signals, each gated on its own
enabled flag AND a non-empty endpoint (observability.tracing.enabled +
observability.tracing.endpoint.grpc; observability.metrics.enabled +
observability.metrics.endpoint.grpc). Each signal that is on injects its
own OTEL_EXPORTER_OTLP_{TRACES,METRICS}_ENDPOINT per the OTel spec's
per-signal endpoint override, so tracing and metrics can point at
different collectors. A signal that is off injects
OTEL_{TRACES,METRICS}_EXPORTER=none so that otel-tools' MetricsConfig/
TracesConfig see "disabled" and do NOT fall back to the
opentelemetry-otlp default of http://localhost:4317 -- without this,
every pod would build a live exporter aimed at localhost and log export
errors on every push cycle.

POD_NAME (downward API) and OTEL_RESOURCE_ATTRIBUTES (always includes
service.instance.id from the pod name; operator resourceAttributes are
appended after a comma) are injected whenever either signal is on.

Note: OTEL_SERVICE_NAME is intentionally absent. ResourceBuilder::new(name)
gives code-supplied attributes priority over env detectors (OTel spec §Resource),
so the env var would be silently ignored — the binary name is always the one
passed to init_tracing(), not the chart value.
*/}}
{{- define "service.otelEnv" -}}
{{- $tracingOn := and .Values.observability.tracing.enabled .Values.observability.tracing.endpoint.grpc -}}
{{- $metricsOn := and .Values.observability.metrics.enabled .Values.observability.metrics.endpoint.grpc -}}
{{- if or $tracingOn $metricsOn }}
- name: POD_NAME
  valueFrom:
    fieldRef:
      fieldPath: metadata.name
- name: OTEL_RESOURCE_ATTRIBUTES
  value: "service.instance.id=$(POD_NAME){{- if .Values.observability.resourceAttributes }},{{ .Values.observability.resourceAttributes }}{{- end }}"
{{- end }}
{{- if $tracingOn }}
- name: OTEL_EXPORTER_OTLP_TRACES_ENDPOINT
  value: {{ .Values.observability.tracing.endpoint.grpc | quote }}
- name: OTEL_TRACES_EXPORTER
  value: "otlp"
- name: OTEL_TRACES_SAMPLER
  value: "parentbased_traceidratio"
- name: OTEL_TRACES_SAMPLER_ARG
  value: {{ .Values.observability.tracing.samplingRatio | quote }}
{{- else }}
- name: OTEL_TRACES_EXPORTER
  value: "none"
{{- end }}
{{- if $metricsOn }}
- name: OTEL_EXPORTER_OTLP_METRICS_ENDPOINT
  value: {{ .Values.observability.metrics.endpoint.grpc | quote }}
- name: OTEL_METRICS_EXPORTER
  value: "otlp"
{{- else }}
- name: OTEL_METRICS_EXPORTER
  value: "none"
{{- end }}
{{- end }}

{{/*
Event-consumer env wiring. There is a single `service_server` container that
runs both HTTP and gRPC, so this is no longer gated on a per-container "which
server handles events" role — only on whether event consumption is enabled.
Streaming knobs come from the root-level `streaming` key (shared with the worker).
Usage: include "service.eventConsumerEnvForServer" (dict "Values" .Values)
*/}}
{{- define "service.eventConsumerEnvForServer" -}}
{{- $values := .Values -}}
{{- if $values.notifications.eventConsumer.storageEventQueue.enabled }}
- name: NOTIFICATIONS_EVENT_CONSUMER_ENDPOINT
  value: {{ required "notifications.eventConsumer.endpoint must be set when the orchestrator's event consumer is enabled" $values.notifications.eventConsumer.endpoint | quote }}
- name: NOTIFICATIONS_QUEUE_RESOURCE_PREFIXES
  value: {{ required "notifications.eventConsumer.resourcePrefixes must contain at least one non-empty prefix when the orchestrator's event consumer is enabled" (include "chart.resourcePrefixes" (dict "Values" $values)) | quote }}
- name: EVENT_PROCESSING_MAX_CONCURRENT
  value: {{ $values.streaming.maxConcurrent | quote }}
- name: STREAM_TIMEOUT_SECS
  value: {{ $values.streaming.streamTimeoutSecs | quote }}
- name: MAX_RECONNECT_BACKOFF_SECONDS
  value: {{ $values.streaming.maxReconnectBackoffSecs | quote }}
{{- else }}
- name: DISABLE_EVENT_CONSUMPTION
  value: "true"
{{- end }}
{{- end }}

{{/*
Return the proper image pull secrets.
{{ include "service.imagePullSecrets" ( dict "image" .Values.thumbnailService.image ) }}
*/}}
{{- define "service.imagePullSecrets" -}}
  {{- $pullSecrets := list }}

  {{- range .image.pullSecrets -}}
    {{- $pullSecrets = append $pullSecrets (get . "name") -}}
  {{- end -}}

  {{- if (not (empty $pullSecrets)) -}}
imagePullSecrets:
    {{- range $pullSecrets | uniq }}
  - name: {{ . }}
    {{- end -}}
  {{- end -}}
{{- end -}}

{{/*
Worker (task-worker) helpers. Distinct `worker.*` prefixes keep them from
colliding with the `service.*` definitions above. Value paths read
`.Values.thumbnailWorker.*` (renamed from the post-fusion `worker:` key).
*/}}

{{/*
Expand the name of the worker component.

The standalone thumbnail-task-worker chart defaulted this to `.Chart.Name`
("thumbnail-task-worker"). Now that both components share one chart,
`.Chart.Name` is "thumbnail-service" — reusing it bare here would collide
with `service.name` and produce identical Deployment/ServiceAccount names
for both components. The literal default below keeps the worker's resource
names byte-for-byte identical to the pre-fusion chart's defaults (see
openspec/changes/fold-task-worker-into-service-chart/design.md's migration
notes on ServiceAccount-name continuity for Workload Identity bindings).
*/}}
{{- define "worker.name" -}}
{{- default "thumbnail-task-worker" .Values.thumbnailWorker.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Fully qualified worker app name.
*/}}
{{- define "worker.fullname" -}}
{{- if .Values.thumbnailWorker.fullnameOverride }}
{{- .Values.thumbnailWorker.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- $name := include "worker.name" . }}
{{- if contains $name .Release.Name }}
{{- .Release.Name | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}
{{- end }}

{{- define "worker.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{- define "worker.labels" -}}
helm.sh/chart: {{ include "worker.chart" . }}
{{ include "worker.selectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/component: task-worker
{{- end }}

{{- define "worker.selectorLabels" -}}
app.kubernetes.io/name: {{ include "worker.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/*
Name of the worker's service account. Same resolution order as the orchestrator's:
component name, then the shared root name, then a generated one.
*/}}
{{- define "worker.serviceAccountName" -}}
{{- $name := default .Values.serviceAccount.name .Values.thumbnailWorker.serviceAccount.name }}
{{- if .Values.serviceAccount.create }}
{{- default (include "worker.fullname" .) $name }}
{{- else }}
{{- default "default" $name }}
{{- end }}
{{- end }}

{{/*
Worker Secret name: the same operator-provided Secret the orchestrator mounts.
*/}}
{{- define "worker.secretName" -}}
{{- include "chart.clientSecretName" . }}
{{- end }}

{{/*
Refuse to render a worker that cannot do its job.

Each of these produces a pod that starts, looks healthy, and converts nothing,
which is worse than a failed install. `resourcePrefixes` in particular is also
the input to WorkerEventQueue's runtime-derived `queue_id`
(`worker_event_queue_id`, distinct from the orchestrator's
`storage_event_queue_id` by construction, so the two consumers can never
collide on one durable session), so an empty list fails both the id
derivation's intent and the filter groups in one motion.

Outbound auth is validated once via `service.validateServiceIdentity` (root-level
`serviceIdentity`); the worker no longer carries a separate identity block.
*/}}
{{- define "worker.validate" -}}
{{- if not .Values.notifications.eventConsumer.endpoint }}
{{- fail "notifications.eventConsumer.endpoint is required: the worker consumes generate work from Notification" }}
{{- end }}
{{- if include "chart.resourcePrefixesEmpty" . }}
{{- fail "notifications.eventConsumer.resourcePrefixes is required: an empty resource-prefix filter binds a queue that receives nothing while every call still reports success" }}
{{- end }}
{{- if not .Values.storage.endpoint }}
{{- fail "storage.endpoint is required: the worker reads sources and writes thumbnails over storage" }}
{{- end }}
{{- include "worker.validateMemoryLimit" . }}
{{- include "service.validateServiceIdentity" . }}
{{- end }}

{{/*
Parse a Kubernetes binary memory quantity (Ki/Mi/Gi) to bytes. Integer
coefficients only — "2.5Gi" is refused so the comparison below cannot silently
truncate.
*/}}
{{- define "chart.k8sMemoryBytes" -}}
{{- $raw := . | toString | trim }}
{{- if hasSuffix "Gi" $raw }}
{{- $n := trimSuffix "Gi" $raw }}
{{- if regexMatch "^[0-9]+$" $n }}
{{- mul ($n | int) 1073741824 }}
{{- else }}
{{- fail (printf "cannot parse memory quantity %q (use an integer Gi/Mi/Ki value)" $raw) }}
{{- end }}
{{- else if hasSuffix "Mi" $raw }}
{{- $n := trimSuffix "Mi" $raw }}
{{- if regexMatch "^[0-9]+$" $n }}
{{- mul ($n | int) 1048576 }}
{{- else }}
{{- fail (printf "cannot parse memory quantity %q (use an integer Gi/Mi/Ki value)" $raw) }}
{{- end }}
{{- else if hasSuffix "Ki" $raw }}
{{- $n := trimSuffix "Ki" $raw }}
{{- if regexMatch "^[0-9]+$" $n }}
{{- mul ($n | int) 1024 }}
{{- else }}
{{- fail (printf "cannot parse memory quantity %q (use an integer Gi/Mi/Ki value)" $raw) }}
{{- end }}
{{- else }}
{{- fail (printf "cannot parse memory quantity %q (expected a binary suffix: Ki, Mi, Gi)" $raw) }}
{{- end }}
{{- end }}

{{/*
Refuse a worker memory limit that cannot hold the documented worst case.

The binding figure is maxTransformPeakBytes, not the decode bound. A resample
allocates a working buffer of source_width × new_height at 16 B/px on top of the
source and the result, so the peak is set by the source and the request
together — the earlier `maxDecodedPixels × 8` formula counted one buffer per
conversion and understated it. maxDecodedPixels is still validated here because
it bounds the decode that precedes the resample.

Raising either knob without the limit is how 1536Mi went stale.
*/}}
{{- define "worker.validateMemoryLimit" -}}
{{- $limit := dig "resources" "limits" "memory" "" .Values.thumbnailWorker }}
{{- if not $limit }}
{{- fail "thumbnailWorker.resources.limits.memory is required: the worker's worst-case flatten peak is sized in the chart and must fit under the limit" }}
{{- end }}
{{- $limitBytes := include "chart.k8sMemoryBytes" $limit | int64 }}
{{- $conc := int64 .Values.workerRuntime.maxConcurrentConversions }}
{{- $pixels := int64 .Values.workerRuntime.maxDecodedPixels }}
{{- if le $conc 0 }}
{{- fail (printf "workerRuntime.maxConcurrentConversions must be > 0 (got %d)" $conc) }}
{{- end }}
{{- if le $pixels 0 }}
{{- fail (printf "workerRuntime.maxDecodedPixels must be > 0 (got %d)" $pixels) }}
{{- end }}
{{- $transformPeak := int64 .Values.workerRuntime.maxTransformPeakBytes }}
{{- if le $transformPeak 0 }}
{{- fail (printf "workerRuntime.maxTransformPeakBytes must be > 0 (got %d)" $transformPeak) }}
{{- end }}
{{- $peak := mul $conc $transformPeak }}
{{- $idle := 209715200 }}
{{- $need := add $peak $idle }}
{{- if gt $need $limitBytes }}
{{- fail (printf "thumbnailWorker.resources.limits.memory %s (%d bytes) is below the worst-case conversion peak: maxConcurrentConversions (%d) × maxTransformPeakBytes (%d) + 200Mi idle = %d bytes. Raise the limit, or lower concurrency or the transform budget." $limit $limitBytes $conc $transformPeak $need) }}
{{- end }}
{{- end }}

{{/*
In-cluster AssociateThumbnail target. The worker and orchestrator ship in one
release, so this is always this chart's Service — not an operator-supplied host.
*/}}
{{- define "worker.thumbnailServiceGrpcEndpoint" -}}
{{- printf "%s:%v" (include "service.servicename" .) .Values.service.grpcPort -}}
{{- end }}

{{/*
Worker identity env — same root-level serviceIdentity as the orchestrator.
Volume mount names stay worker-specific so they don't collide with any other
volume in the worker pod, but the mount *paths* and CLIENT_SECRET_FILE layout
match the orchestrator (clientSecretRef.key under /etc/secrets).
*/}}
{{- define "worker.identityEnv" -}}
{{- include "service.serviceIdentityEnv" . }}
{{- end }}

{{- define "worker.identityVolumeMounts" -}}
{{- if .Values.serviceIdentity.clientCredentials.enabled }}
- name: worker-identity-secret
  mountPath: /etc/secrets
  readOnly: true
{{- end }}
{{- if .Values.serviceIdentity.accessTokenFile.enabled }}
- name: worker-token-exchange
  mountPath: /etc/secrets/access-token
  readOnly: true
{{- end }}
{{- end }}
