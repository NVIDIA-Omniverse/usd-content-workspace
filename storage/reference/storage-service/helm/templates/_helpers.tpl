{{/*
Expand the name of the chart.
*/}}
{{- define "storage-service.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create a default fully qualified app name.
We truncate at 63 chars because some Kubernetes name fields are limited to this (by the DNS naming spec).
If release name contains chart name it will be used as a full name.
*/}}
{{- define "storage-service.fullname" -}}
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
Create a string name to be used in service and headless service
*/}}
{{- define "storage-service.servicename" -}}
{{- $name := default .Chart.Name .Values.service.nameOverride }}
{{- printf "%s" $name | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/* 
Default TLS secret name 
*/}}
{{- define "storage-service.tls-secret-name" -}}
{{- $defaultname := print "tls-cert-" (include "storage-service.fullname" .) -}}
{{- $name := default $defaultname .Values.httpProxy.tls.secretName }}
{{- printf "%s" $name | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create chart name and version as used by the chart label.
*/}}
{{- define "storage-service.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Common labels
*/}}
{{- define "storage-service.labels" -}}
helm.sh/chart: {{ include "storage-service.chart" . }}
{{ include "storage-service.selectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{/*
Selector labels
*/}}
{{- define "storage-service.selectorLabels" -}}
app.kubernetes.io/name: {{ include "storage-service.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/*
Create the name of the service account to use
*/}}
{{- define "storage-service.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}
{{- default (include "storage-service.fullname" .) .Values.serviceAccount.name }}
{{- else }}
{{- default "default" .Values.serviceAccount.name }}
{{- end }}
{{- end }}

{{/*
Service account name for the smoke-test Job.
*/}}
{{- define "storage-service.smokeTestServiceAccountName" -}}
{{- $default := printf "%s-test-smoke" (include "storage-service.fullname" .) -}}
{{- default $default .Values.tests.serviceAccount.name | trunc 63 | trimSuffix "-" -}}
{{- end }}

{{/*
Return the proper image name.
*/}}
{{- define "storage-service.image" -}}
  {{- printf "%s:%s" .repository .tag -}}
{{- end -}}

{{/*
Return the proper image pull secrets, by merging the lists into a map.
*/}}
{{- define "storage-service.imagePullSecrets" -}}
  {{- if (not (empty .)) -}}
imagePullSecrets:
    {{- range . }}
  - name: {{ .name }}
    {{- end -}}
  {{- end -}}
{{- end -}}

{{/*
AWS credentials configuration.
*/}}
{{- define "storage-service.config.aws-credentials" -}}
  {{- with (required "credentials must be set when AWS credentials are configured" .credentials) }}
    {{- if .anonymous }}
credentials.anonymous = true
    {{- end }}
    {{- with .accessKey }}
credentials.access_key.access_key_id = {{ required "credentials.accessKey.accessKeyId must be set when credentials.accessKey is configured" .accessKeyId | quote }}
credentials.access_key.secret_access_key = {{ required "credentials.accessKey.secretAccessKey must be set when credentials.accessKey is configured" .secretAccessKey | quote }}
      {{- with .sessionToken }}
credentials.access_key.session_token = {{ quote . }}
      {{- end }}
    {{- end }}
    {{- with .assumeRole }}
credentials.assume_role.role_arn = {{ required "credentials.assumeRole.roleArn must be set when credentials.assumeRole is configured" .roleArn | quote }}
      {{- with .externalId }}
credentials.assume_role.external_id = {{ quote . }}
      {{- end }}
      {{- with .sessionName }}
credentials.assume_role.session_name = {{ quote . }}
      {{- end }}
      {{- with .sessionLength }}
credentials.assume_role.session_length = {{ quote . }}
      {{- end }}
    {{- end }}
    {{- with .webIdentityToken }}
credentials.web_identity_token.web_identity_token_file = {{ required "credentials.webIdentityToken.webIdentityTokenFile must be set when credentials.webIdentityToken is configured" .webIdentityTokenFile | quote }}
credentials.web_identity_token.role_arn = {{ required "credentials.webIdentityToken.roleArn must be set when credentials.webIdentityToken is configured" .roleArn | quote }}
      {{- with .roleSessionName }}
credentials.web_identity_token.role_session_name = {{ quote . }}
      {{- end }}
    {{- end }}
    {{- with .credentialsFile }}
credentials.credentials_file.path = {{ required "credentials.credentialsFile.path must be set when credentials.credentialsFile is configured" .path | quote }}
    {{- end }}
  {{- end }}
{{- end -}}

{{/*
Create an endpoint URL from either a hostname, or a URL.
*/}}
{{- define "storage-service.config.endpoint-url" }}
  {{- if (contains "://" .endpoint) }}
    {{- .endpoint -}}
  {{- else }}
    {{- printf "https://%s" .endpoint }}
  {{- end }}
{{- end }}
