# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

{{/*
Expand the name of the chart.
*/}}
{{- define "discovery.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{- define "discovery.v1EndpointsConfigMapName" -}}
{{- print (include "discovery.fullname" .)  "-v1-endpoints" }}
{{- end }}

{{/*
Create a default fully qualified app name.
We truncate at 63 chars because some Kubernetes name fields are limited to this (by the DNS naming spec).
If release name contains chart name it will be used as a full name.
*/}}
{{- define "discovery.fullname" -}}
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
{{- define "discovery.servicename" -}}
{{/* NOTE - this is for backwards compat with old Disco v1 Clients */}}
{{- $name := default "auto-discovery-service" .Values.service.nameOverride }}
{{- printf "%s" $name | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create chart name and version as used by the chart label.
*/}}
{{- define "discovery.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Common labels
*/}}
{{- define "discovery.labels" -}}
helm.sh/chart: {{ include "discovery.chart" . }}
{{ include "discovery.selectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{/*
Selector labels
*/}}
{{- define "discovery.selectorLabels" -}}
app.kubernetes.io/name: {{ include "discovery.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/*
Create the name of the service account to use
*/}}
{{- define "discovery.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}
{{- default (include "discovery.fullname" .) .Values.serviceAccount.name }}
{{- else }}
{{- default "default" .Values.serviceAccount.name }}
{{- end }}
{{- end }}

{{- define "discovery.imagePullSecrets" -}}
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
{{- define "discovery.tlsSecretName" -}}
{{- $defaultname := print "tls-cert-" (include "discovery.fullname" .) -}}
{{- $name := default $defaultname .Values.httpProxy.tls.secretName }}
{{- printf "%s" $name | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Render the OpenID auth bootstrap config served at /api/{v1,v1alpha,v2,v2beta}/auth-config.
*/}}
{{- define "discovery.authConfigJson" -}}
{{- $openIdConfigUri := required "openId.openIdConfigurationUri is required when openId.enabled is true" .Values.openId.openIdConfigurationUri -}}
{{- if or (eq $openIdConfigUri "https://....") (not (hasPrefix "http" $openIdConfigUri)) -}}
{{- fail "openId.openIdConfigurationUri must be a real OpenID provider metadata URL when openId.enabled is true" -}}
{{- end -}}
{{- if empty .Values.openId.clientRegistrations -}}
{{- fail "openId.clientRegistrations must contain at least one client when openId.enabled is true" -}}
{{- end -}}
{{- $authConfig := dict -}}
{{- $clients := dict -}}
{{- range .Values.openId.clientRegistrations -}}
{{- $name := required "openId.clientRegistrations[].name is required" .name -}}
{{- $clientId := required "openId.clientRegistrations[].clientId is required" .clientId -}}
{{- $scope := required "openId.clientRegistrations[].scope is required" .scope -}}
{{- if or (eq $clientId "some_client_id") (eq $clientId "") -}}
{{- fail "openId.clientRegistrations[].clientId must be set to a real client id when openId.enabled is true" -}}
{{- end -}}
{{- $clients = set $clients $name (dict "client_id" $clientId "scope" $scope) -}}
{{- end -}}
{{- if and (not (hasKey $clients "client_library")) (hasKey $clients "default") -}}
{{- $clients = set $clients "client_library" (get $clients "default") -}}
{{- end -}}
{{- if and (not (hasKey $clients "navigator")) (hasKey $clients "default") -}}
{{- $clients = set $clients "navigator" (get $clients "default") -}}
{{- end -}}
{{- $authConfig = set $authConfig "_comment" (.Values.openId.description | default "OpenID Authentication") -}}
{{- $authConfig = set $authConfig "openid_configuration" $openIdConfigUri -}}
{{- $authConfig = set $authConfig "clients" $clients -}}
{{- print ($authConfig | toJson) -}}
{{- end }}
