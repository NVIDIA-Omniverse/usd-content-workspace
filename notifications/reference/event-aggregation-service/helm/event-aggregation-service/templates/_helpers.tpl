# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

{{/*
Expand the name of the chart.
*/}}
{{- define "event-aggregation-service.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create a default fully qualified app name.
We truncate at 63 chars because some Kubernetes name fields are limited to this (by the DNS naming spec).
If release name contains chart name it will be used as a full name.
*/}}
{{- define "event-aggregation-service.fullname" -}}
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
{{- define "event-aggregation-service.servicename" -}}
{{- $name := default .Chart.Name .Values.service.nameOverride }}
{{- printf "%s" $name | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create chart name and version as used by the chart label.
*/}}
{{- define "event-aggregation-service.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Common labels
*/}}
{{- define "event-aggregation-service.labels" -}}
helm.sh/chart: {{ include "event-aggregation-service.chart" . }}
{{ include "event-aggregation-service.selectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{/*
Selector labels
*/}}
{{- define "event-aggregation-service.selectorLabels" -}}
app.kubernetes.io/name: {{ include "event-aggregation-service.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/*
Create the name of the service account to use
*/}}
{{- define "event-aggregation-service.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}
{{- default (include "event-aggregation-service.fullname" .) .Values.serviceAccount.name }}
{{- else }}
{{- default "default" .Values.serviceAccount.name }}
{{- end }}
{{- end }}

{{/*
Service account name for the smoke-test Job. Separate from the main
service SA so the test runs under its own AAD identity — the smoke-test
is a client of event-aggregation-service, not the service itself.
*/}}
{{- define "event-aggregation-service.smokeTestServiceAccountName" -}}
{{- if .Values.tests.serviceAccount.create -}}
{{- $default := printf "%s-test-smoke" (include "event-aggregation-service.fullname" .) -}}
{{- default $default .Values.tests.serviceAccount.name | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- default "default" .Values.tests.serviceAccount.name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end }}

{{- define "event-aggregation-service.imagePullSecrets" -}}
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

{{- define "event-aggregation-service.tls-secret-name" -}}
{{- $defaultname := print "tls-cert-" (include "event-aggregation-service.fullname" .) -}}
{{- $name := default $defaultname .Values.httpProxy.tls.secretName }}
{{- printf "%s" $name | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Find the "default" client in openId.clientRegistrations. More than one entry
named "default" fails the render, since only one can supply the audience.
*/}}
{{- define "event-aggregation-service.findDefaultClient" -}}
  {{- $defaults := list }}
  {{- range .Values.openId.clientRegistrations }}
    {{- if eq .name "default" }}
      {{- $defaults = append $defaults . }}
    {{- end }}
  {{- end }}
  {{- if gt (len $defaults) 1 }}
    {{- fail "openId.clientRegistrations has more than one entry named \"default\"; keep one." }}
  {{- end }}
  {{- range $defaults }}
    {{- . | toYaml }}
  {{- end }}
{{- end }}

{{/*
The accepted JWT audiences as one comma-separated string:
openId.additionalJwtAudience plus the "default" client registration's
clientId, each trimmed, with empty entries dropped (as the library's
parsed_jwt_audiences reads them). OMNI_EVENTS_JWT_AUDIENCES renders this
helper, and principalSource reads it to decide whether OpenID is configured,
so the chart can never select jwt on audiences the service does not receive.
openId.additionalJwtAudience must be a list; null or an empty string mean no
extra audience, and any other value fails with a message naming the value
path rather than a raw template error from append.
*/}}
{{- define "event-aggregation-service.jwtAudiences" -}}
{{- $additional := .Values.openId.additionalJwtAudience | default list -}}
{{- if not (kindIs "slice" $additional) -}}
{{- fail (printf "openId.additionalJwtAudience must be a YAML list of audiences, for example [\"api://my-api\"]; got kind %s (%v)." (kindOf $additional) $additional) -}}
{{- end -}}
{{- $audiences := list -}}
{{- range append $additional (include "event-aggregation-service.findDefaultClient" . | fromYaml).clientId -}}
{{- if . -}}
{{- $audiences = append $audiences (toString . | trim) -}}
{{- end -}}
{{- end -}}
{{- $audiences | compact | join "," -}}
{{- end -}}

{{/*
The effective principalSource:
1. A non-empty .Values.principalSource, used as given (validateAuthSettings
   refuses an unknown value).
2. Otherwise jwt, in which the service verifies every token itself, when
   OpenID is configured: openId.openIdConfigurationUri is set and
   jwtAudiences is non-empty.
3. Otherwise the render fails, naming which of the two is missing, because no
   safe mode is left to choose.
The chart never selects jwtUnverified by itself. The Envoy auth extension
verifies only the requests that come through the HTTPProxy, and the Service
stays reachable from inside the cluster without it, so jwtUnverified is
always a human decision (see allowUnverifiedJwt).
*/}}
{{- define "event-aggregation-service.principalSource" -}}
{{- if .Values.principalSource -}}
{{- .Values.principalSource -}}
{{- else if and .Values.openId.openIdConfigurationUri (include "event-aggregation-service.jwtAudiences" .) -}}
jwt
{{- else -}}
{{- $uri := ternary "set" "missing" (not (empty .Values.openId.openIdConfigurationUri)) -}}
{{- $audience := ternary "set" "missing" (not (empty (include "event-aggregation-service.jwtAudiences" .))) -}}
{{- fail (printf "principalSource is unset and OpenID is not fully configured (openId.openIdConfigurationUri: %s; JWT audience: %s), so the chart cannot choose how the service authenticates callers. Set openId.openIdConfigurationUri plus at least one JWT audience (a \"default\" entry in openId.clientRegistrations, whose clientId is the audience, or openId.additionalJwtAudience) so the service verifies tokens itself (principalSource jwt), or set principalSource explicitly (disabled for a local or test run without tokens)." $uri $audience) -}}
{{- end -}}
{{- end -}}

{{/*
OMNI_EVENTS_ALLOW_UNVERIFIED_JWT, "true" or "false". It is the library's
opt-in for jwtUnverified, which reads the principal from a token without
verifying it, and only a person makes that choice: the result is "true" only
when the values set both principalSource: jwtUnverified and
allowUnverifiedJwt: true. allowUnverifiedJwt: true without principalSource:
jwtUnverified fails the render, and so does jwtUnverified without
allowUnverifiedJwt: true; allowUnverifiedJwt: false renders "false" in every
other mode. The checks read the explicit .Values.principalSource because the
effective one is jwtUnverified only when it is set explicitly.
*/}}
{{- define "event-aggregation-service.allowUnverifiedJwt" -}}
{{- $allow := .Values.allowUnverifiedJwt -}}
{{- if not (or (kindIs "bool" $allow) (kindIs "invalid" $allow)) -}}
{{- fail (printf "allowUnverifiedJwt must be true, false, or null; got a value of kind %s (%v). Write it unquoted: a quoted \"true\" or \"false\" is a string." (kindOf $allow) $allow) -}}
{{- end -}}
{{- $explicitlyUnverified := eq (toString .Values.principalSource) "jwtUnverified" -}}
{{- if and $allow (not $explicitlyUnverified) -}}
{{- fail "allowUnverifiedJwt applies only to principalSource: jwtUnverified; set both, or remove it." -}}
{{- end -}}
{{- if and $explicitlyUnverified (not $allow) -}}
{{- fail "principalSource jwtUnverified trusts any token that reaches the pod. Set allowUnverifiedJwt: true only when every path to the pods verifies tokens first: the Envoy auth extension covers only HTTPProxy traffic, so the Service must also be unreachable directly (for example through a NetworkPolicy). Otherwise use principalSource: jwt (openId.openIdConfigurationUri plus a default client registration)." -}}
{{- end -}}
{{- if $allow -}}true{{- else -}}false{{- end -}}
{{- end -}}

{{/*
A render-time copy of the auth checks the service runs at startup
(notification-common's AuthSettings validator, and the service's own
permissions.enabled-needs-permissions.endpoint check), so auth values the
service would refuse fail `helm template` and `helm upgrade` instead of
crash-looping the pods. The service's own Settings validators beyond the auth
checks mirrored here, and the library's CORS origin format check, are not
repeated. Prints nothing. The allowUnverifiedJwt checks run first because they
do not depend on the effective principalSource, so a quoted value is reported
as such even when the selection would also fail. jwtAudiences runs next, in
every mode, so a mistyped openId.additionalJwtAudience fails even where the
audiences are not rendered. permissions.enabled is read the way pydantic reads
the rendered string: Go template truthiness would treat a quoted "false" as
true.
*/}}
{{- define "event-aggregation-service.validateAuthSettings" -}}
{{- $_ := include "event-aggregation-service.allowUnverifiedJwt" . -}}
{{- $_ = include "event-aggregation-service.jwtAudiences" . -}}
{{- $ps := include "event-aggregation-service.principalSource" . -}}
{{- $known := list "disabled" "jwtUnverified" "jwt" "opaque" "userInfoService" -}}
{{- if not (has $ps $known) -}}
{{- fail (printf "principalSource %q is not one of: %s." $ps (join ", " $known)) -}}
{{- end -}}
{{- $openIdUri := .Values.openId.openIdConfigurationUri -}}
{{- if and (eq $ps "jwt") (not $openIdUri) -}}
{{- fail "principalSource jwt needs openId.openIdConfigurationUri." -}}
{{- end -}}
{{- if and (eq $ps "jwt") (not (include "event-aggregation-service.jwtAudiences" .)) -}}
{{- fail "principalSource jwt needs at least one JWT audience: a default client registration in openId.clientRegistrations, or openId.additionalJwtAudience." -}}
{{- end -}}
{{- if and (eq $ps "opaque") (not $openIdUri) -}}
{{- fail "principalSource opaque needs openId.openIdConfigurationUri." -}}
{{- end -}}
{{- if and (eq $ps "userInfoService") (not .Values.userInfoService.uri) -}}
{{- fail "principalSource userInfoService needs userInfoService.uri." -}}
{{- end -}}
{{- /* The spellings pydantic reads as a true bool from the rendered string. */ -}}
{{- $permissionsEnabled := has (toString .Values.permissions.enabled | lower) (list "true" "1" "yes" "on" "t" "y") -}}
{{- if and (eq $ps "disabled") $permissionsEnabled -}}
{{- fail "permissions.enabled requires an authenticated caller, but principalSource is disabled. Choose a principalSource that authenticates callers, or set permissions.enabled: false." -}}
{{- end -}}
{{- if and $permissionsEnabled (not .Values.permissions.endpoint) -}}
{{- fail "permissions.enabled needs permissions.endpoint, the permissions service the checks are made against. Set it, or set permissions.enabled: false." -}}
{{- end -}}
{{- end -}}

{{/*
OMNI_EVENTS_CORS_ALLOW_ORIGINS as a JSON array. rest.corsAllowOrigins
must be null or a list of origin strings. The service answers every listed
origin with credentials allowed, so "*" and "null" (compared trimmed and
lower-cased, as the library compares them) would let pages the operator does
not control make credentialed calls; the service refuses both at startup, and
failing here keeps that out of a crash-loop. The origin format itself
(scheme://host[:port], no path) is checked only by the library, at startup.
*/}}
{{- define "event-aggregation-service.corsAllowOrigins" -}}
{{- $origins := .Values.rest.corsAllowOrigins -}}
{{- if not (or (kindIs "invalid" $origins) (kindIs "slice" $origins)) -}}
{{- fail (printf "rest.corsAllowOrigins must be a YAML list of origins, for example [\"https://app.example.com\"]; got kind %s." (kindOf $origins)) -}}
{{- end -}}
{{- range $origins -}}
{{- if kindIs "invalid" . -}}
{{- fail "rest.corsAllowOrigins contains a YAML null. List quoted origin strings such as \"https://app.example.com\"; the \"null\" origin is refused." -}}
{{- else if not (kindIs "string" .) -}}
{{- fail (printf "rest.corsAllowOrigins entries must be origin strings such as \"https://app.example.com\"; got %v (kind %s)." . (kindOf .)) -}}
{{- end -}}
{{- if has (lower (trim .)) (list "*" "null") -}}
{{- fail (printf "rest.corsAllowOrigins must not contain %q: the service allows credentials on CORS requests, so that origin would let pages you do not control make credentialed calls. List exact origins such as https://app.example.com, or leave it empty for no CORS." .) -}}
{{- end -}}
{{- end -}}
{{- $origins | default list | toJson -}}
{{- end -}}
