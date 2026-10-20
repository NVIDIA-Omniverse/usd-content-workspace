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
{{- define "permission-ui.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create a default fully qualified app name.
We truncate at 63 chars because some Kubernetes name fields are limited to this (by the DNS naming spec).
If release name contains chart name it will be used as a full name.
*/}}
{{- define "permission-ui.fullname" -}}
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
{{- define "permission-ui.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Common labels
*/}}
{{- define "permission-ui.labels" -}}
helm.sh/chart: {{ include "permission-ui.chart" . }}
{{ include "permission-ui.selectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{/*
Selector labels
*/}}
{{- define "permission-ui.selectorLabels" -}}
app.kubernetes.io/name: {{ include "permission-ui.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/*
Create the name of the service account to use
*/}}
{{- define "permission-ui.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}
{{- default (include "permission-ui.fullname" .) .Values.serviceAccount.name }}
{{- else }}
{{- default "default" .Values.serviceAccount.name }}
{{- end }}
{{- end }}

{{- define "permission-ui.imagePullSecrets" -}}
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
{{- define "permission-ui.tlsSecretName" -}}
{{- $defaultname := print "tls-cert-" (include "permission-ui.fullname" .) -}}
{{- $name := default $defaultname .Values.httpProxy.tls.secretName }}
{{- printf "%s" $name | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create a string name to be used in the Service K8s object
*/}}
{{- define "permission-ui.servicename" -}}
{{- $name := default .Chart.Name .Values.service.nameOverride }}
{{- printf "%s" $name | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Extract authority URL from OpenID configuration URI
*/}}
{{- define "permission-ui.authAuthority" -}}
  {{- if .Values.openId.openIdConfigurationUri }}
    {{- .Values.openId.openIdConfigurationUri | trimSuffix "/.well-known/openid-configuration" }}
  {{- end }}
{{- end }}

{{/*
Find permission-ui client in clientRegistrations
*/}}
{{- define "permission-ui.findPermissionUiClient" -}}
  {{- $clientRegistrations := .Values.openId.clientRegistrations }}
  {{- if $clientRegistrations }}
    {{- range $clientRegistrations }}
      {{- if eq .name "permission-ui" }}
        {{- . | toYaml }}
      {{- end }}
    {{- end }}
  {{- end }}
{{- end }}

{{/*
Find default client in clientRegistrations
*/}}
{{- define "permission-ui.findDefaultClient" -}}
  {{- $clientRegistrations := .Values.openId.clientRegistrations }}
  {{- if $clientRegistrations }}
    {{- range $clientRegistrations }}
      {{- if eq .name "default" }}
        {{- . | toYaml }}
      {{- end }}
    {{- end }}
  {{- end }}
{{- end }}

{{/*
Get client registration from clientRegistrations, preferring "permission-ui" over "default"
*/}}
{{- define "permission-ui.clientRegistration" -}}
  {{- $permissionUiClient := include "permission-ui.findPermissionUiClient" . }}
  {{- if $permissionUiClient }}
    {{- $permissionUiClient }}
  {{- else }}
    {{- include "permission-ui.findDefaultClient" . }}
  {{- end }}
{{- end }}

{{/*
Construct redirect URI with protocol based on TLS settings
*/}}
{{- define "permission-ui.redirectUri" -}}
  {{- $protocol := "http://" }}
  {{- if .Values.httpProxy.tls.enabled }}
    {{- $protocol = "https://" }}
  {{- end }}
  {{- $fqdn := print (ternary "" (print .Values.httpProxy.fqdn.host ".") (empty .Values.httpProxy.fqdn.host)) .Values.httpProxy.fqdn.domain }}
  {{- $path := .Values.httpProxy.path | default "/" | trimSuffix "/" }}
  {{- $redirectPath := print $path "/openid" }}
  {{- print $protocol $fqdn $redirectPath }}
{{- end }}



