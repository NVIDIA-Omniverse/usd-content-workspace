{{/*
Expand the name of the chart.
*/}}
{{- define "conformance-test.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create a default fully qualified app name.
*/}}
{{- define "conformance-test.fullname" -}}
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
Create chart name and version as used by the chart label.
*/}}
{{- define "conformance-test.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Common labels
*/}}
{{- define "conformance-test.labels" -}}
helm.sh/chart: {{ include "conformance-test.chart" . }}
{{ include "conformance-test.selectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{/*
Selector labels
*/}}
{{- define "conformance-test.selectorLabels" -}}
app.kubernetes.io/name: {{ include "conformance-test.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/*
Create the name of the ServiceAccount to use.
*/}}
{{- define "conformance-test.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}
{{- default (include "conformance-test.fullname" .) .Values.serviceAccount.name }}
{{- else }}
{{- default "default" .Values.serviceAccount.name }}
{{- end }}
{{- end }}

{{/*
Existing Kubernetes Secret that holds the OAuth2 client secret. This chart
does not create Secrets, matching helm/thumbnail-service's convention.
*/}}
{{- define "conformance-test.clientSecretName" -}}
{{- required "serviceIdentity.clientCredentials.clientSecretRef.name is required when clientCredentials.enabled is true: this chart does not create Secrets" .Values.serviceIdentity.clientCredentials.clientSecretRef.name }}
{{- end }}

{{/*
Existing Kubernetes Secret that holds the direct bearer token. This chart
does not create Secrets, matching helm/thumbnail-service's convention.
*/}}
{{- define "conformance-test.bearerTokenSecretName" -}}
{{- required "serviceIdentity.bearerToken.secretRef.name is required when bearerToken.enabled is true: this chart does not create Secrets" .Values.serviceIdentity.bearerToken.secretRef.name }}
{{- end }}

{{/*
Validate the serviceIdentity block at template-time. Unlike
helm/thumbnail-service, at most one mode may be enabled here — an
unauthenticated run against a deployment with auth disabled is a valid,
supported configuration for this chart, so none of the flags being set is not
an error the way it is for the long-running service.
*/}}
{{- define "conformance-test.validateServiceIdentity" -}}
{{- $cc := .Values.serviceIdentity.clientCredentials.enabled -}}
{{- $atf := .Values.serviceIdentity.accessTokenFile.enabled -}}
{{- $bt := .Values.serviceIdentity.bearerToken.enabled -}}
{{- $enabledCount := 0 -}}
{{- if $cc }}{{- $enabledCount = add $enabledCount 1 -}}{{- end -}}
{{- if $atf }}{{- $enabledCount = add $enabledCount 1 -}}{{- end -}}
{{- if $bt }}{{- $enabledCount = add $enabledCount 1 -}}{{- end -}}
{{- if gt $enabledCount 1 -}}
{{- fail "serviceIdentity: more than one of `clientCredentials.enabled`, `accessTokenFile.enabled`, and `bearerToken.enabled` is true; at most one outbound auth mode may be set." -}}
{{- end -}}
{{- if and $atf (not .Values.serviceIdentity.accessTokenFile.provisionerSidecar.container) -}}
{{- fail "serviceIdentity: `accessTokenFile.enabled` is true but `accessTokenFile.provisionerSidecar.container` is empty; the sidecar that provisions the token file is required in this mode." -}}
{{- end -}}
{{- if and $cc (not .Values.serviceIdentity.clientCredentials.openIdConfigurationUri) -}}
{{- fail "serviceIdentity: `clientCredentials.enabled` is true but `clientCredentials.openIdConfigurationUri` is empty; the discovery endpoint is required." -}}
{{- end -}}
{{- if and $cc (not .Values.serviceIdentity.clientCredentials.clientId) -}}
{{- fail "serviceIdentity: `clientCredentials.enabled` is true but `clientCredentials.clientId` is empty; the OAuth2 client id is required." -}}
{{- end -}}
{{- if and $cc (not .Values.serviceIdentity.clientCredentials.clientSecretRef.key) -}}
{{- fail "serviceIdentity: `clientCredentials.enabled` is true but `clientCredentials.clientSecretRef.key` is empty; the Secret key is required." -}}
{{- end -}}
{{- if and $cc (not .Values.serviceIdentity.clientCredentials.clientSecretRef.name) -}}
{{- fail "serviceIdentity: `clientCredentials.enabled` is true but `clientCredentials.clientSecretRef.name` is empty; this chart does not create Secrets." -}}
{{- end -}}
{{- if and $bt (not .Values.serviceIdentity.bearerToken.secretRef.key) -}}
{{- fail "serviceIdentity: `bearerToken.enabled` is true but `bearerToken.secretRef.key` is empty; the Secret key is required." -}}
{{- end -}}
{{- if and $bt (not .Values.serviceIdentity.bearerToken.secretRef.name) -}}
{{- fail "serviceIdentity: `bearerToken.enabled` is true but `bearerToken.secretRef.name` is empty; this chart does not create Secrets." -}}
{{- end -}}
{{- end }}
