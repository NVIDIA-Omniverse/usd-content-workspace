{{/*
SPDX-FileCopyrightText: Copyright (c) 2025 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
SPDX-License-Identifier: Apache-2.0
*/}}
{{/*
Expand the name of the chart.
*/}}
{{- define "event-consumer-service.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create a default fully qualified app name.
We truncate at 63 chars because some Kubernetes name fields are limited to this (by the DNS naming spec).
If release name contains chart name it will be used as a full name.
*/}}
{{- define "event-consumer-service.fullname" -}}
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
{{- define "event-consumer-service.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create a string name to be used in service and headless service
*/}}
{{- define "event-consumer-service.servicename" -}}
{{- $name := default .Chart.Name .Values.service.nameOverride }}
{{- printf "%s" $name | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Service name used by the Contour HTTPProxy. This Service sets
publishNotReadyAddresses: true so that a pod that has gone
not-ready at the readiness reserve -- the soft scaling signal,
not the memory bound -- stays in Envoy's EDS cluster. That both
preserves its existing streaming connections and keeps it
reachable for pinned streams packing into the overflow band.
*/}}
{{- define "event-consumer-service.envoy-servicename" -}}
{{- $base := default .Chart.Name .Values.service.nameOverride }}
{{- printf "%s-envoy" $base | trunc 63 | trimSuffix "-" }}
{{- end }}


{{/*
Common labels
*/}}
{{- define "event-consumer-service.labels" -}}
helm.sh/chart: {{ include "event-consumer-service.chart" . }}
{{ include "event-consumer-service.selectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{/*
Selector labels
*/}}
{{- define "event-consumer-service.selectorLabels" -}}
app.kubernetes.io/name: {{ include "event-consumer-service.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/*
Create the name of the service account to use
*/}}
{{- define "event-consumer-service.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}
{{- default (include "event-consumer-service.fullname" .) .Values.serviceAccount.name }}
{{- else }}
{{- default "default" .Values.serviceAccount.name }}
{{- end }}
{{- end }}

{{/*
Service account name for the smoke-test Job. Separate from the main
service SA so the test runs under its own AAD identity — the smoke-test
is a client of event-consumer-service, not the service itself.
*/}}
{{- define "event-consumer-service.smokeTestServiceAccountName" -}}
{{- if .Values.tests.serviceAccount.create -}}
{{- $default := printf "%s-test-smoke" (include "event-consumer-service.fullname" .) -}}
{{- default $default .Values.tests.serviceAccount.name | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- default "default" .Values.tests.serviceAccount.name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end }}

{{/*
Name of the post-install/post-upgrade Job that applies the event-queue
bounds policy to the broker. Its own name (rather than the release
fullname) so it never collides with the Deployment or the smoke-test Job.
*/}}
{{- define "event-consumer-service.queuePolicyJobName" -}}
{{- printf "%s-queue-policy" (include "event-consumer-service.fullname" .) | trunc 63 | trimSuffix "-" -}}
{{- end }}

{{/*
Refuse a non-numeric value for one of the policy's numeric bounds.

Takes a dict of `path` (the values path, for the message) and `value`.

The range guards in templates/queue-policy-hook-job.yaml are not enough on
their own, because they run through `int`, and Helm's `int` maps ANY
unparseable string to 0 -- so `--set ...deliveryLimit=abc` (or `=""`, or a
value quoted in an operator's own values file) sails through a `>= 0` check
and ships a REAL `delivery-limit: 0`, which dead-letters every requeued
message on its first redelivery. The same coercion turns a string `expiresMs`
into a 0 that the broker REJECTS, failing the release with a broker-side
error rather than an actionable one. Refusing at render is the only point
where the operator's actual typo can be named back to him.

The message names the failing key twice -- once as the values path, once in
the "write X as a bare number" instruction -- and spells out what a coerced 0
would mean for EACH key, so the operator is not left to work out which of the
listed consequences is his. Any new numeric policy bound must be added to that
enumeration, or its failure message silently describes only the other keys.

Numbers arrive as `float64` from a values file (Helm parses YAML through
JSON) and as `int64` from `--set`, so both are accepted.
*/}}
{{- define "event-consumer-service.requireNumericPolicyBound" -}}
{{- if not (or (kindIs "int64" .value) (kindIs "float64" .value)) -}}
{{- fail (printf "policyHook.policy.definition.%s must be a number, but got %s (%v). Helm coerces an unparseable string to 0, so this would otherwise ship a bound of 0 -- silently destructive for deliveryLimit/messageTtlMs/maxLength, rejected by the broker for expiresMs, and below RabbitMQ's supported floor for consumerTimeoutMs (a consumer-timeout of 0 cancels every consumer on every durable queue). Quote nothing here: write %s as a bare number, or leave it null on the keys whose null omits the bound (expiresMs, consumerTimeoutMs). Set policyHook.enabled=false to manage the event-queue policy outside this chart." .path (kindOf .value) .value .path) -}}
{{- end -}}
{{- end }}

{{- define "event-consumer-service.imagePullSecrets" -}}
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
Header policy included on exactly ONE route -- the gRPC
ConsumeNonDurableEvents route in http-proxy.yaml -- not on every
route. REST/SSE is deliberately left unstamped: the service
classifies every REST/SSE stream as re-routable unconditionally,
so a stamp there would carry no information.

What the stamp means: it marks the stream RE-ROUTABLE for
admission classification. A proxied client's retry is
load-balanced afresh, so such streams are refused at the shed
threshold (max_re_routable_streams_per_pod), which reserves the
band between that threshold and the memory cap for pinned streams
that scale-out cannot rescue. This is permanently load-bearing
for admission classification, not a rolling-upgrade migration
artifact; a helm-template retention test pins the header.

Trust boundary: Envoy's "set" semantics overwrite the header on
every request that TRAVERSES the proxy, so a proxied client can
neither forge nor suppress its own classification. That guarantee
does not extend to requests that bypass the proxy, and the stamp
is trusted as-is. A direct (pinned) client that self-applies
X-Via-Proxy only classifies its own streams as re-routable and
gets them shed below the cap -- self-harm rather than an
escalation, which is why no anti-spoof hardening exists.
*/}}
{{- define "event-consumer-service.proxy-request-headers" -}}
requestHeadersPolicy:
  set:
    - name: X-Via-Proxy
      value: "true"
{{- end -}}

{{/*
Route policies come in TWO sibling helpers, deliberately not one:
"streaming-route-policies" below serves the long-lived streaming routes,
and "unary-route-policies" after it serves the non-streaming catch-all.
They are NOT collapsible into one helper called with a different
settings path, because the streaming helper also emits a
loadBalancerPolicy and the catch-all must never carry a strategy:
Contour hashes the strategy into the Envoy cluster name (Clustername()),
so a strategy on the catch-all would split its connection pool off from
the default cluster and rename its per-cluster stats. Their retry
policies differ in kind as well -- see each helper's own comment.

Policies applied to the long-lived STREAMING routes of both HTTPProxy
objects: the load-balancing strategy (steering only -- it never affects
admission) and the retry policy that re-routes a pod's capacity refusal
-- when it arrives as a trailers-only RESOURCE_EXHAUSTED before any
response commits -- onto a fresh load-balancer selection that can land
on another pod, never a guaranteed different one; bounded retries
exhaust into the documented 503 / RESOURCE_EXHAUSTED + backoff.

Call with (dict "settings" <merged proxySettings> "kind" "grpc"|"rest").

The retry CONDITIONS are intentionally not operator-tunable: they mirror
this service's error contract exactly -- a capacity refusal is
RESOURCE_EXHAUSTED on gRPC and 503 on REST. Both spellings are always
emitted explicitly, because Contour honours retriableStatusCodes only
when retryOn contains retriable-status-codes, and an EMPTY retryOn is
not "no conditions" -- Contour substitutes 5xx, which would retry
genuine server errors. perTryTimeout is deliberately never emitted: it
applies to the initial attempt too and would cut healthy 240m streams.
*/}}
{{- define "event-consumer-service.streaming-route-policies" -}}
{{- $streaming := .settings.streaming | default dict -}}
{{- with (($streaming.loadBalancerPolicy).strategy) }}
loadBalancerPolicy:
  strategy: {{ . }}
{{- end }}
{{- if ($streaming.retryPolicy).enabled }}
retryPolicy:
  retryOn:
  {{- if eq .kind "grpc" }}
    - resource-exhausted
  {{- else }}
    - retriable-status-codes
  retriableStatusCodes:
    - 503
  {{- end }}
  count: {{ dig "count" 2 ($streaming.retryPolicy | default dict) }}
{{- end }}
{{- end -}}

{{/*
Retry policy applied to the non-streaming CATCH-ALL route of both
HTTPProxy objects -- the unary REST calls and unary gRPC RPCs, including
the non-idempotent durable-queue create / update / delete.

Call with (dict "settings" <merged proxySettings>). Unlike the streaming
helper this needs NO "kind" branch: connect-failure is a TCP-level
condition and refused-stream is an HTTP/2 one that is simply inert
against the HTTP/1.1 REST backend, so one uniform list serves both
proxies.

The chart retries on two conditions, both of them provably-unsent:
connect-failure is a failed or timed-out upstream TCP connect, and
refused-stream is an upstream REFUSED_STREAM reset, which a gRPC
server's graceful GOAWAY produces for any stream above its
last-stream-id. A request that never left Envoy can be replayed on
another pod safely even when the method is not idempotent, which is what
makes this policy correct for DeleteDurableQueue. This is deliberately
much narrower than the streaming routes' policy: 5xx,
retriable-status-codes, reset and gateway-error all admit cases where
the app may already have processed the request, so none of them appear
here. retriableStatusCodes is never emitted for the same reason.

These two are not an exhaustive enumeration of the provably-unsent
conditions Envoy defines -- reset-before-request is another, and
Contour's enum accepts it from v1.31 -- they are the two this chart
retries on. Name the residual honestly: on the HTTP/1.1 REST leg there
is no refused-stream analogue, so a pooled keep-alive connection to a
terminating pod that is found dead at dispatch (Envoy response flag UC)
is not retried by the proxy and still surfaces to the client.

Both conditions are always emitted explicitly when the policy is
enabled, because an EMPTY retryOn is not "no conditions" -- Contour
substitutes 5xx, exactly the unsafe policy ruled out above. To turn the
policy off, set httpProxy.unary.retryPolicy.enabled: false, which omits
the block outright. perTryTimeout is deliberately never emitted.
*/}}
{{- define "event-consumer-service.unary-route-policies" -}}
{{- $unary := .settings.unary | default dict -}}
{{- if ($unary.retryPolicy).enabled }}
retryPolicy:
  retryOn:
    - connect-failure
    - refused-stream
  count: {{ dig "count" 3 ($unary.retryPolicy | default dict) }}
{{- end }}
{{- end -}}

{{- define "event-consumer-service.tls-secret-name" -}}
{{- $defaultname := print "tls-cert-" (include "event-consumer-service.fullname" .) -}}
{{- $name := default $defaultname .Values.httpProxy.tls.secretName }}
{{- printf "%s" $name | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Find the "default" client in openId.clientRegistrations. More than one entry
named "default" fails the render, since only one can supply the audience.
*/}}
{{- define "event-consumer-service.findDefaultClient" -}}
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
{{- define "event-consumer-service.jwtAudiences" -}}
{{- $additional := .Values.openId.additionalJwtAudience | default list -}}
{{- if not (kindIs "slice" $additional) -}}
{{- fail (printf "openId.additionalJwtAudience must be a YAML list of audiences, for example [\"api://my-api\"]; got kind %s (%v)." (kindOf $additional) $additional) -}}
{{- end -}}
{{- $audiences := list -}}
{{- range append $additional (include "event-consumer-service.findDefaultClient" . | fromYaml).clientId -}}
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
{{- define "event-consumer-service.principalSource" -}}
{{- if .Values.principalSource -}}
{{- .Values.principalSource -}}
{{- else if and .Values.openId.openIdConfigurationUri (include "event-consumer-service.jwtAudiences" .) -}}
jwt
{{- else -}}
{{- $uri := ternary "set" "missing" (not (empty .Values.openId.openIdConfigurationUri)) -}}
{{- $audience := ternary "set" "missing" (not (empty (include "event-consumer-service.jwtAudiences" .))) -}}
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
{{- define "event-consumer-service.allowUnverifiedJwt" -}}
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
{{- define "event-consumer-service.validateAuthSettings" -}}
{{- $_ := include "event-consumer-service.allowUnverifiedJwt" . -}}
{{- $_ = include "event-consumer-service.jwtAudiences" . -}}
{{- $ps := include "event-consumer-service.principalSource" . -}}
{{- $known := list "disabled" "jwtUnverified" "jwt" "opaque" "userInfoService" -}}
{{- if not (has $ps $known) -}}
{{- fail (printf "principalSource %q is not one of: %s." $ps (join ", " $known)) -}}
{{- end -}}
{{- $openIdUri := .Values.openId.openIdConfigurationUri -}}
{{- if and (eq $ps "jwt") (not $openIdUri) -}}
{{- fail "principalSource jwt needs openId.openIdConfigurationUri." -}}
{{- end -}}
{{- if and (eq $ps "jwt") (not (include "event-consumer-service.jwtAudiences" .)) -}}
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
OMNI_EVENTS_CORS_ALLOW_ORIGINS as a JSON array. service.rest.corsAllowOrigins
must be null or a list of origin strings. The service answers every listed
origin with credentials allowed, so "*" and "null" (compared trimmed and
lower-cased, as the library compares them) would let pages the operator does
not control make credentialed calls; the service refuses both at startup, and
failing here keeps that out of a crash-loop. The origin format itself
(scheme://host[:port], no path) is checked only by the library, at startup.
*/}}
{{- define "event-consumer-service.corsAllowOrigins" -}}
{{- $origins := .Values.service.rest.corsAllowOrigins -}}
{{- if not (or (kindIs "invalid" $origins) (kindIs "slice" $origins)) -}}
{{- fail (printf "service.rest.corsAllowOrigins must be a YAML list of origins, for example [\"https://app.example.com\"]; got kind %s." (kindOf $origins)) -}}
{{- end -}}
{{- range $origins -}}
{{- if kindIs "invalid" . -}}
{{- fail "service.rest.corsAllowOrigins contains a YAML null. List quoted origin strings such as \"https://app.example.com\"; the \"null\" origin is refused." -}}
{{- else if not (kindIs "string" .) -}}
{{- fail (printf "service.rest.corsAllowOrigins entries must be origin strings such as \"https://app.example.com\"; got %v (kind %s)." . (kindOf .)) -}}
{{- end -}}
{{- if has (lower (trim .)) (list "*" "null") -}}
{{- fail (printf "service.rest.corsAllowOrigins must not contain %q: the service allows credentials on CORS requests, so that origin would let pages you do not control make credentialed calls. List exact origins such as https://app.example.com, or leave it empty for no CORS." .) -}}
{{- end -}}
{{- end -}}
{{- $origins | default list | toJson -}}
{{- end -}}
