{{/*
Expand the name of the chart.
*/}}
{{- define "userinfoservice.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create a default fully qualified app name.
We truncate at 63 chars because some Kubernetes name fields are limited to this (by the DNS naming spec).
If release name contains chart name it will be used as a full name.
*/}}
{{- define "userinfoservice.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- $name := default .Chart.Name .Values.nameOverride }}
{{- if contains $name .Release.Name }}
{{- .Release.Name | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}
{{- end }}

{{/*
Create a string name to be used in the Service K8s object
*/}}
{{- define "userinfoservice.servicename" -}}
{{- $name := default .Chart.Name .Values.service.nameOverride }}
{{- printf "%s" $name | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create chart name and version as used by the chart label.
*/}}
{{- define "userinfoservice.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Common labels
*/}}
{{- define "userinfoservice.labels" -}}
helm.sh/chart: {{ include "userinfoservice.chart" . }}
{{ include "userinfoservice.selectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{/*
Selector labels
*/}}
{{- define "userinfoservice.selectorLabels" -}}
app.kubernetes.io/name: {{ include "userinfoservice.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/*
Writer / reader component labels and selectors.
The component label lets the Service target reader pods only.
*/}}
{{- define "userinfoservice.writerLabels" -}}
{{ include "userinfoservice.labels" . }}
app.kubernetes.io/component: writer
{{- end }}

{{- define "userinfoservice.readerLabels" -}}
{{ include "userinfoservice.labels" . }}
app.kubernetes.io/component: reader
{{- end }}

{{- define "userinfoservice.writerSelectorLabels" -}}
{{ include "userinfoservice.selectorLabels" . }}
app.kubernetes.io/component: writer
{{- end }}

{{- define "userinfoservice.readerSelectorLabels" -}}
{{ include "userinfoservice.selectorLabels" . }}
app.kubernetes.io/component: reader
{{- end }}

{{/*
Shared environment variable blocks, reused by the writer and reader deployments.
Each renders a list of env entries; include with `nindent` at the env indentation.
*/}}
{{- define "userinfoservice.loggingEnv" -}}
- name: RUST_LOG
  value: {{ .Values.observability.logging.level | quote }}
- name: STRUCTURED_LOGGING
  value: {{ .Values.observability.logging.structured.enabled | quote }}
- name: ACCESS_LOG_ENABLED
  value: {{ .Values.observability.logging.access.enabled | quote }}
{{- if .Values.observability.logging.access.enabled }}
- name: ACCESS_LOG_HEADERS
  value: {{ .Values.observability.logging.access.headers | toJson | quote }}
{{- end }}
{{- end }}

{{- define "userinfoservice.forwardHeadersEnv" -}}
- name: FORWARD_HEADERS
  value: {{ join "," .Values.observability.headers.forward | quote }}
{{- end }}

{{- define "userinfoservice.entraEnv" -}}
{{- if .Values.providers.entra.tenantId }}
- name: AZURE_TENANT_ID
  value: {{ .Values.providers.entra.tenantId | quote }}
{{- end }}
{{- if .Values.providers.entra.clientId }}
- name: AZURE_CLIENT_ID
  value: {{ .Values.providers.entra.clientId | quote }}
{{- end }}
- name: AZURE_AUTH_MODE
  value: {{ .Values.providers.entra.authMode | quote }}
{{- if eq .Values.providers.entra.authMode "client-secret" }}
- name: AZURE_CLIENT_SECRET
  valueFrom:
    secretKeyRef:
      name: {{ .Values.providers.entra.clientSecret.secretRef.name }}
      key: {{ .Values.providers.entra.clientSecret.secretRef.key }}
{{- end }}
- name: GRAPH_MAX_RETRIES
  value: {{ .Values.providers.entra.maxRetries | quote }}
{{- end }}

{{- define "userinfoservice.cacheKindsEnv" -}}
- name: CACHE_USERS_ENABLED
  value: {{ .Values.providers.cache.users.enabled | quote }}
- name: CACHE_GROUPS_ENABLED
  value: {{ .Values.providers.cache.groups.enabled | quote }}
- name: CACHE_MEMBERSHIPS_ENABLED
  value: {{ .Values.providers.cache.memberships.enabled | quote }}
{{- end }}

{{- define "userinfoservice.appRoleAssignmentsEnv" -}}
- name: APP_ROLE_ASSIGNMENTS_ENABLED
  value: {{ .Values.appRoleAssignments.enabled | quote }}
{{- if .Values.appRoleAssignments.enabled }}
{{- if .Values.appRoleAssignments.servicePrincipalObjectId }}
- name: APP_ROLE_ASSIGNMENTS_SERVICE_PRINCIPAL_OBJECT_ID
  value: {{ .Values.appRoleAssignments.servicePrincipalObjectId | quote }}
{{- end }}
- name: APP_ROLE_ASSIGNMENTS_INCLUDE_SERVICE_PRINCIPALS
  value: {{ .Values.appRoleAssignments.includeServicePrincipals | quote }}
{{- end }}
{{- end }}

{{/*
Fail install/upgrade/template early when app-role assignment scoping is enabled
against an incompatible cache configuration. The service enforces the same
invariants at startup; this surfaces the error before a pod is created.
*/}}
{{- define "userinfoservice.validateAppRoleAssignments" -}}
{{- if .Values.appRoleAssignments.enabled }}
{{- if not .Values.providers.cache.enabled }}
{{- fail "appRoleAssignments.enabled requires providers.cache.enabled=true" }}
{{- end }}
{{- if not .Values.providers.cache.groups.enabled }}
{{- fail "appRoleAssignments.enabled requires providers.cache.groups.enabled=true" }}
{{- end }}
{{- if not .Values.providers.cache.memberships.enabled }}
{{- fail "appRoleAssignments.enabled requires providers.cache.memberships.enabled=true" }}
{{- end }}
{{- end }}
{{- end }}

{{- define "userinfoservice.postgresEnv" -}}
- name: POSTGRES_ENABLED
  value: {{ .Values.providers.cache.postgres.enabled | quote }}
{{- if .Values.providers.cache.postgres.enabled }}
{{- $_ := required "POSTGRES_HOST must be set when postgres.enabled is true" .Values.providers.cache.postgres.host }}
- name: POSTGRES_HOST
  value: {{ .Values.providers.cache.postgres.host | quote }}
- name: POSTGRES_PORT
  value: {{ .Values.providers.cache.postgres.port | quote }}
- name: POSTGRES_DBNAME
  value: {{ .Values.providers.cache.postgres.dbname | quote }}
- name: POSTGRES_USER
  value: {{ .Values.providers.cache.postgres.user | quote }}
{{- if .Values.providers.cache.postgres.tokenFile.enabled }}
- name: POSTGRES_TOKEN_FILE
  value: {{ .Values.providers.cache.postgres.tokenFile.path | quote }}
- name: POSTGRES_TOKEN_FILE_POLL_INTERVAL
  value: {{ .Values.providers.cache.postgres.tokenFile.pollInterval | quote }}
- name: POSTGRES_TOKEN_FILE_WAIT_TIMEOUT
  value: {{ .Values.providers.cache.postgres.tokenFile.waitTimeout | quote }}
- name: POSTGRES_TOKEN_FILE_WAIT_INTERVAL
  value: {{ .Values.providers.cache.postgres.tokenFile.waitInterval | quote }}
{{- else }}
- name: POSTGRES_PASSWORD
  valueFrom:
    secretKeyRef:
      name: {{ .Values.providers.cache.postgres.password.secretRef.name }}
      key: {{ .Values.providers.cache.postgres.password.secretRef.key }}
{{- end }}
- name: POSTGRES_CONNECTIONS
  value: {{ .Values.providers.cache.postgres.connections | quote }}
- name: POSTGRES_SSL_MODE
  value: {{ .Values.providers.cache.postgres.sslMode | quote }}
{{- end }}
{{- end }}

{{/*
PostgreSQL rotating-token provisioner sidecar wiring.

Rendered only when the snapshot store is enabled, tokenFile.enabled is true, and
a provisionerSidecar.container is supplied. The chart adds a shared in-memory
`postgres-token` volume mounted at the token file's directory: read-only on the
main container, writable on the sidecar. When no sidecar is supplied the operator
provides the token file via the chart-level `volumes`/`volumeMounts`.
*/}}
{{- define "userinfoservice.postgresTokenSidecarEnabled" -}}
{{- if and .Values.providers.cache.postgres.enabled .Values.providers.cache.postgres.tokenFile.enabled .Values.providers.cache.postgres.tokenFile.provisionerSidecar.container -}}
true
{{- end -}}
{{- end }}

{{- define "userinfoservice.postgresTokenVolumeMount" -}}
- name: postgres-token
  readOnly: true
  mountPath: {{ .Values.providers.cache.postgres.tokenFile.path | dir | quote }}
{{- end }}

{{- define "userinfoservice.postgresTokenSidecar" -}}
- name: {{ .Chart.Name }}-postgres-token-provisioner
  {{- (omit .Values.providers.cache.postgres.tokenFile.provisionerSidecar.container "volumeMounts") | toYaml | nindent 2 }}
  volumeMounts:
    {{- with .Values.providers.cache.postgres.tokenFile.provisionerSidecar.container.volumeMounts }}
      {{- toYaml . | nindent 4 }}
    {{- end }}
    - name: postgres-token
      mountPath: {{ .Values.providers.cache.postgres.tokenFile.path | dir | quote }}
{{- end }}

{{- define "userinfoservice.postgresTokenVolume" -}}
- name: postgres-token
  emptyDir:
    sizeLimit: 4Mi
    medium: Memory
{{- end }}

{{- define "userinfoservice.accessTokenSidecarEnabled" -}}
{{- if and .Values.serviceIdentity.accessTokenFile.enabled .Values.serviceIdentity.accessTokenFile.provisionerSidecar.container -}}
true
{{- end -}}
{{- end }}

{{- define "userinfoservice.accessTokenVolumeMount" -}}
- name: token-exchange
  readOnly: true
  mountPath: {{ .Values.serviceIdentity.accessTokenFile.path | dir | quote }}
{{- end }}

{{- define "userinfoservice.accessTokenSidecar" -}}
- name: {{ .Chart.Name }}-token-provisioner
  {{- (omit .Values.serviceIdentity.accessTokenFile.provisionerSidecar.container "volumeMounts") | toYaml | nindent 2 }}
  volumeMounts:
    {{- with .Values.serviceIdentity.accessTokenFile.provisionerSidecar.container.volumeMounts }}
      {{- toYaml . | nindent 4 }}
    {{- end }}
    - name: token-exchange
      mountPath: {{ .Values.serviceIdentity.accessTokenFile.path | dir | quote }}
{{- end }}

{{- define "userinfoservice.accessTokenVolume" -}}
- name: token-exchange
  emptyDir:
    sizeLimit: 4Mi
    medium: Memory
{{- end }}

{{- define "userinfoservice.observabilityEnv" -}}
- name: OTEL_TRACES_ENABLED
  value: {{ .Values.observability.tracing.enabled | quote }}
{{- if .Values.observability.tracing.enabled }}
- name: OTEL_EXPORTER_OTLP_TRACES_ENDPOINT
  value: {{ .Values.observability.tracing.endpoint.grpc | quote }}
{{- end }}
- name: OTEL_METRICS_ENABLED
  value: {{ .Values.observability.metrics.enabled | quote }}
{{- if .Values.observability.metrics.enabled }}
- name: OTEL_EXPORTER_OTLP_METRICS_ENDPOINT
  value: {{ .Values.observability.metrics.endpoint.grpc | quote }}
{{- end }}
{{- if or .Values.observability.tracing.enabled .Values.observability.metrics.enabled }}
- name: OTEL_SERVICE_NAME
  value: {{ .Values.observability.serviceName | quote }}
{{- end }}
{{- end }}

{{/*
Create the name of the service account to use
*/}}
{{- define "userinfoservice.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}
{{- default (include "userinfoservice.fullname" .) .Values.serviceAccount.name }}
{{- else }}
{{- default "default" .Values.serviceAccount.name }}
{{- end }}
{{- end }}

{{- define "userinfoservice.imagePullSecrets" -}}
  {{- $pullSecrets := list }}

  {{- range ((.global).imagePullSecrets) -}}
    {{- $pullSecrets = append $pullSecrets (get . "name") -}}
  {{- end -}}

  {{- range .images -}}
    {{- range .pullSecrets -}}
      {{- $pullSecrets = append $pullSecrets (get . "name") -}}
    {{- end -}}
  {{- end -}}

  {{- if (not (empty $pullSecrets)) -}}
imagePullSecrets:
    {{- range $pullSecrets | uniq }}
  - name: {{ . }}
    {{- end -}}
  {{- end -}}
{{- end -}}

{{/*
Create the TLS secret name with an SSL certificate
*/}}
{{- define "userinfoservice.tlsSecretName" -}}
{{- $defaultname := print "tls-cert-" (include "userinfoservice.fullname" .) -}}
{{- $name := default $defaultname .Values.httpProxy.tls.secretName }}
{{- printf "%s" $name | trunc 63 | trimSuffix "-" }}
{{- end }}
