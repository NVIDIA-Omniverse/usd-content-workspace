{{/*
SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
SPDX-License-Identifier: Apache-2.0

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
*/}}

{{/*
Expand the name of the chart.
*/}}
{{- define "permissions.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create a default fully qualified app name.
We truncate at 63 chars because some Kubernetes name fields are limited to this (by the DNS naming spec).
If release name contains chart name it will be used as a full name.
*/}}
{{- define "permissions.fullname" -}}
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
{{- define "permissions.servicename" -}}
{{- $name := default .Chart.Name .Values.service.nameOverride }}
{{- printf "%s" $name | trunc 63 | trimSuffix "-" }}
{{- end }}


{{/*
Create chart name and version as used by the chart label.
*/}}
{{- define "permissions.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Common labels
*/}}
{{- define "permissions.labels" -}}
helm.sh/chart: {{ include "permissions.chart" . }}
{{ include "permissions.selectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{/*
Selector labels
*/}}
{{- define "permissions.selectorLabels" -}}
app.kubernetes.io/name: {{ include "permissions.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{- define "permissions.smokeTestLabels" -}}
helm.sh/chart: {{ include "permissions.chart" . }}
app.kubernetes.io/name: {{ printf "%s-smoke-test" (include "permissions.name" .) | trunc 63 | trimSuffix "-" }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/component: smoke-test
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{/*
Create the name of the service account to use
*/}}
{{- define "permissions.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}
{{- default (include "permissions.fullname" .) .Values.serviceAccount.name }}
{{- else }}
{{- default "default" .Values.serviceAccount.name }}
{{- end }}
{{- end }}

{{- define "permissions.imagePullSecrets" -}}
  {{- $pullSecrets := list }}

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
{{- define "permissions.tlsSecretName" -}}
{{- $defaultname := print "tls-cert-" (include "permissions.fullname" .) -}}
{{- $name := default $defaultname .Values.httpProxy.tls.secretName }}
{{- printf "%s" $name | trunc 63 | trimSuffix "-" }}
{{- end }}


{{/*
Create JWT audience value from allowedJwtAudience. Renders an empty string
when no audiences are configured.
*/}}
{{- define "permissions.jwtAudience" -}}
{{-   $audience := list -}}
{{-   range .allowedJwtAudience -}}
{{-     $audience = append $audience . -}}
{{-   end -}}
{{-   $audience = $audience | sortAlpha | uniq -}}
{{-   print (join ", " $audience) -}}
{{- end -}}

{{/*
Create JWT issuers value from allowedJwtIssuers. Renders an empty string
when no issuers are configured so that `permission-auth` skips `iss` validation.
*/}}
{{- define "permissions.jwtIssuers" -}}
{{-   $issuers := list -}}
{{-   range .allowedJwtIssuers -}}
{{-     $issuers = append $issuers . -}}
{{-   end -}}
{{-   $issuers = $issuers | sortAlpha | uniq -}}
{{-   print (join ", " $issuers) -}}
{{- end -}}