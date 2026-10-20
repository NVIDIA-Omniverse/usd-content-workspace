{{/*
Expand the name of the chart.
*/}}
{{- define "envoy-auth-extension.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create a default fully qualified app name.
We truncate at 63 chars because some Kubernetes name fields are limited to this (by the DNS naming spec).
If release name contains chart name it will be used as a full name.
*/}}
{{- define "envoy-auth-extension.fullname" -}}
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
{{- define "envoy-auth-extension.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Common labels
*/}}
{{- define "envoy-auth-extension.labels" -}}
helm.sh/chart: {{ include "envoy-auth-extension.chart" . }}
{{ include "envoy-auth-extension.selectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{/*
Selector labels
*/}}
{{- define "envoy-auth-extension.selectorLabels" -}}
app.kubernetes.io/name: {{ include "envoy-auth-extension.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}


{{/*
Create the name of the service account to use
*/}}
{{- define "envoy-auth-extension.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}
{{- default (include "envoy-auth-extension.fullname" .) .Values.serviceAccount.name }}
{{- else }}
{{- default "default" .Values.serviceAccount.name }}
{{- end }}
{{- end }}

{{- define "envoy-auth-extension.failOnRemovedOpenIdValues" -}}
{{-   $openId := .Values.openId | default dict -}}
{{-   $replacements := dict "clientRegistrations" "openId.allowedJwtAudience" "additionalJwtAudience" "openId.allowedJwtAudience" "jwtIssuers" "openId.allowedJwtIssuers" -}}
{{-   $errors := list -}}
{{-   range $key, $replacement := $replacements -}}
{{-     if hasKey $openId $key -}}
{{-       $errors = append $errors (printf "openId.%s was removed, use %s instead" $key $replacement) -}}
{{-     end -}}
{{-   end -}}
{{-   if $errors -}}
{{-     fail (join "; " $errors) -}}
{{-   end -}}
{{- end -}}

{{- define "envoy-auth-extension.jwtAudience" -}}
{{-   $audience := . | default list | sortAlpha | uniq -}}
{{-   $quotedAudience := list -}}
{{-   range $audience -}}
{{-      $quotedAudience = append $quotedAudience (. | quote) -}}
{{-   end -}}
{{-   print "[" (join ", " $quotedAudience ) "]" -}}
{{- end -}}

{{- define "envoy-auth-extension.jwtIssuers" -}}
{{-   $quotedIssuers := list -}}
{{-   range . -}}
{{-      $quotedIssuers = append $quotedIssuers (. | quote) -}}
{{-   end -}}
{{-   print "[" (join ", " $quotedIssuers ) "]" -}}
{{- end -}}

{{- define "envoy-auth-extension.jwtRequiredClaims" -}}
{{-   $rules := list -}}
{{-   range . -}}
{{-     $fields := list -}}
{{-     range $key, $value := . -}}
{{-       $fields = append $fields (printf "%s = %s" ($key | toJson) ($value | toJson)) -}}
{{-     end -}}
{{-     $rules = append $rules (printf "{ %s }" (join ", " $fields)) -}}
{{-   end -}}
{{-   print "[" (join ", " $rules ) "]" -}}
{{- end -}}

{{- define "envoy-auth-extension.servicename" -}}
{{- $name := default (print .Chart.Name "-service") .Values.service.nameOverride }}
{{- printf "%s" $name | trunc 63 | trimSuffix "-" }}
{{- end -}}

{{- define "envoy-auth-extension.imagePullSecrets" -}}
  {{- $pullSecrets := list }}

  {{- range .imagePullSecrets -}}
    {{- $pullSecrets = append $pullSecrets (get . "name") -}}
  {{- end -}}

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
