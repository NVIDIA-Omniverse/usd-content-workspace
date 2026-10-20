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
{{- define "discovery-service.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create a default fully qualified app name.
We truncate at 63 chars because some Kubernetes name fields are limited to this (by the DNS naming spec).
If release name contains chart name it will be used as a full name.
*/}}
{{- define "discovery-service.fullname" -}}
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

{{- define "discovery-service.servicename" -}}
{{- $name := default .Chart.Name .Values.service.nameOverride }}
{{- printf "%s" $name | trunc 63 | trimSuffix "-" }}
{{- end }}

{{- define "discovery-service.tls-secret-name" -}}
{{- $defaultname := print "tls-cert-" (include "discovery-service.fullname" .) -}}
{{- $name := default $defaultname .Values.httpProxy.tls.secretName }}
{{- printf "%s" $name | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create chart name and version as used by the chart label.
*/}}
{{- define "discovery-service.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Common labels
*/}}
{{- define "discovery-service.labels" -}}
helm.sh/chart: {{ include "discovery-service.chart" . }}
{{ include "discovery-service.selectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{/*
Selector labels
*/}}
{{- define "discovery-service.selectorLabels" -}}
app.kubernetes.io/name: {{ include "discovery-service.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/*
Common labels
*/}}


{{/*
Create the name of the service account to use
*/}}
{{- define "discovery-service.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}
{{- default (include "discovery-service.fullname" .) .Values.serviceAccount.name }}
{{- else }}
{{- default "default" .Values.serviceAccount.name }}
{{- end }}
{{- end }}

{{- define "discovery-service.rootRedirectConfig" -}}
{{-   $ret := "" -}}
{{-   if (.Values.httpProxy.rootRedirect.enabled) -}}
{{-     $ret = print $ret .Values.httpProxy.rootRedirect.target.statusCode " " -}}
{{-     $ret = print $ret (ternary "https" "http" .Values.httpProxy.tls.enabled) -}}
{{-     $ret = print $ret "://" -}}
{{-     $ret = print $ret .Values.httpProxy.rootRedirect.target.fqdn -}}
{{-     $ret = print $ret .Values.httpProxy.rootRedirect.target.path -}}
{{-   else -}}
{{-     $ret = print "302 /info/info.html" -}}
{{-   end -}}
{{-   print $ret -}}
{{- end -}}

{{- define "discovery-service.mkServiceJson" -}}
{{-   $nameSpace := default "default" .Release.Namespace }}
{{-   $servicesList := list }}
{{-   range $i, $service := .Values.discovery.services }}
{{-     $serviceItem := dict 
                        "id" $service.id
                        "name" $service.name
                        "type" $service.type
}}
{{-     if hasKey $service.endpoints "grpc" }}
{{-       $serviceItem = set $serviceItem 
                              "grpc" 
                              (include "discovery-service.mkurl" 
                                        (dict "ep" $service.endpoints.grpc
                                              "proto" "grpc"
                                              "ns" $nameSpace
                                              "type" $service.type
                                              "ingress" $.Values.httpProxy
                                        )
                              )
}}
{{-     end }}

{{-     if hasKey $service.endpoints "rest" }}
{{-       $serviceItem = set $serviceItem 
                              "rest" 
                              (include "discovery-service.mkurl" 
                                        (dict "ep" $service.endpoints.rest
                                              "proto" "rest"
                                              "ns" $nameSpace
                                              "type" $service.type
                                              "ingress" $.Values.httpProxy
                                        )
                              )




}}
{{-     end }}

{{-   $servicesList = append $servicesList $serviceItem }}
{{-   end }}

{{-   $discoManifest := dict 
                          "schema-version" 1
                          "services" $servicesList
}}
{{- print ($discoManifest | toPrettyJson) -}}
{{-   end -}}

################################################################################
################################################################################
################################################################################

{{- define "discovery-service.mkAuthConfigJson" -}}
{{-   $authConfig := dict -}}
{{-   $clients := dict -}}
{{-   range .Values.openId.clientRegistrations -}}
{{-     $clients = set $clients .name (dict "client_id" .clientId "scope" .scope) -}}
{{-   end -}}
{{-   if (not (hasKey $clients "client_library")) -}}
{{-      $clients = set $clients "client_library" (get $clients "default") -}}
{{-   end -}}
{{-   if (not (hasKey $clients "navigator")) -}}
{{-      $clients = set $clients "navigator" (get $clients "default") -}}
{{-   end -}}
{{-   $authConfig = set $authConfig "_comment" .Values.openId.description -}}
{{-   $authConfig = set $authConfig "openid_configuration" .Values.openId.openIdConfigurationUri -}}
{{-   $authConfig = set $authConfig "clients" $clients -}}
{{-   print ($authConfig | toPrettyJson) -}}
{{- end -}}

################################################################################
################################################################################
################################################################################

{{- define "discovery-service.mkurl" -}}

{{- $defaultServiceInfo  := (dict 
                              "storage" (dict "serviceName" "storage-service"
                                              "ingressHost" "storage"
                                              "grpc" "8011"
                                              "rest" "8012")

                              "notification-aggregation" (dict "serviceName" "event-aggregation-service"
                                                               "ingressHost" "event-aggregation"
                                                               "grpc" "50051"
                                                               "rest" "8000")

                              "notification-consumer"    (dict "serviceName" "event-consumer-service"
                                                               "ingressHost" "event-consumer"
                                                               "grpc" "50051"
                                                               "rest" "8000")
                              "permission"    (dict "serviceName" "permission-service"
                                                               "ingressHost" "permissions"
                                                               "grpc" "3010"
                                                               "rest" "3000")

                              "userinfo"    (dict "serviceName" "userinfo-service"
                                                               "ingressHost" "userinfo"
                                                               "grpc" "50051"
                                                               "rest" "8080")

                              "thumbnail"    (dict "serviceName" "thumbnail-service"
                                                               "ingressHost" "thumbnails"
                                                               "grpc" "50051"
                                                               "rest" "3000")

                              "live-edit"   (dict "serviceName" "live-edit-service"
                                                               "ingressHost" "live-edit"
                                                               "grpc" "50001")
                           )
-}}

{{- $defaults := get $defaultServiceInfo .type -}}
{{- $hasExplicitHost := not (empty .ep.host) -}}
{{- if and (not $hasExplicitHost) (not (kindIs "map" $defaults)) -}}
{{-   fail (printf "discovery service type %q has no default endpoint configuration; set endpoints.%s.host explicitly" .type .proto) -}}
{{- end -}}

{{- $tls := false -}}
{{- if (and .ingress.enabled .ingress.tls.enabled) -}}
{{-    $tls = true -}}
{{- else -}}
{{-   if .ep.tls -}}
{{-     $tls = true -}}
{{-   end -}}
{{- end -}}

{{- $schema := ternary "https" "http" $tls -}}

{{- $host := "BUG_IN_HOSTNAME_LOGIC" -}}
{{- if $hasExplicitHost -}}
{{-   $host = .ep.host -}}
{{- else -}}
{{-   if .ingress.enabled -}}
{{-     $host = print $defaults.ingressHost "." .ingress.fqdn.domain -}}
{{-     $host = ternary $host (print .proto "." $host) (eq .proto "rest") -}}
{{-   else -}}
{{-     $host = $defaults.serviceName  -}}
{{-     $host = print $host "." .ns ".svc.cluster.local" -}}
{{-   end -}}
{{- end -}}


{{- $port := "BUG_IN_PORT_LOGIC" -}}
{{- if (not (empty .ep.port)) -}}
{{-    $port = print ":" .ep.port  -}}
{{- else if $hasExplicitHost -}}
{{-   $port = "" -}}
{{- else -}}
{{-   if (not .ingress.enabled) -}}
{{-     $port = print ":" (get $defaults .proto) -}}
{{-   else -}}
{{-     $port = ternary "" (print ":" .ingress.port) (empty .ingress.port) -}}
{{-   end -}}
{{- end -}}

{{- $path := "BUG_IN_PATH_LOGIC" -}}
{{- if (not (empty .ep.path)) -}}
{{-   $path = ternary .ep.path (print "/" .ep.path) (hasPrefix "/" .ep.path) -}}
{{- else -}}

{{/* note: GRPC Clients do not support anything but root! -- Fidot, Dec. 2025 */}}
{{-   if (eq .proto "grpc")  -}} 
{{-     $path = "/" -}}
{{-   else if (not .ingress.enabled) -}}
{{-     $path = "/" -}}
{{-   else -}}
{{-     $path = ternary .ingress.path (print "/" .ingress.path) (hasPrefix "/" .ingress.path) -}}
{{-   end -}}
{{- end -}}

{{- print $schema "://" $host $port $path -}}
{{- end }} 


