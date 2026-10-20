#!/usr/bin/env bash
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

# Probe a Discovery Service URL and every REST endpoint it advertises.
#
# Usage:
#   scripts/test-discovery-endpoints.sh <discovery-url> <token>
#   scripts/test-discovery-endpoints.sh https://disco-v2.dev.example.com "$AUTH_TOKEN"
#
# discovery-url may be a base host (https://host) or already include a discovery
# path. The bearer token is sent on discovery and on each probe.
#
# Every endpoint is checked and given a status line. The script does not stop on
# the first failure; it exits non-zero only after the full summary if any check
# failed. HTTP 4xx/5xx still count as reachable unless --require-2xx is set.

set -euo pipefail

REQUIRE_2XX=0
TIMEOUT_SECS=10

# Discovery paths checked on the base URL (in addition to advertised services).
DISCOVERY_PATHS=(
  /api/v2/services
  /api/v1/services
  /api/v1alpha/services
  /api/v1/auth-config
  /api/v2/auth-config
)

usage() {
  cat <<'EOF'
Usage: test-discovery-endpoints.sh [options] <discovery-url> <token>

Options:
  --require-2xx     Fail unless each REST probe returns HTTP 2xx
  --timeout SECS    curl max-time per request (default: 10)
  -h, --help        Show this help

Arguments:
  discovery-url     Base URL or full discovery path URL
  token             Bearer token for Authorization header

Every check prints a status line (PASS/FAIL/SKIP/INFO). Failures do not abort
remaining checks; the process exit code reflects the full run.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --require-2xx)
      REQUIRE_2XX=1
      shift
      ;;
    --timeout)
      TIMEOUT_SECS="${2:?--timeout requires a value}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    --)
      shift
      break
      ;;
    -*)
      echo "error: unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
    *)
      break
      ;;
  esac
done

if [[ $# -ne 2 ]]; then
  usage >&2
  exit 2
fi

DISCOVERY_URL_IN=$1
TOKEN=$2

for command in curl jq; do
  if ! command -v "$command" >/dev/null 2>&1; then
    echo "error: $command is required" >&2
    exit 1
  fi
done

PASS=0
FAIL=0
SKIP=0

# ANSI colors when stdout is a TTY (or FORCE_COLOR=1); honor NO_COLOR.
if [[ -t 1 && -z "${NO_COLOR:-}" ]] || [[ "${FORCE_COLOR:-}" == "1" ]]; then
  C_RESET=$'\033[0m'
  C_BOLD=$'\033[1m'
  C_GREEN=$'\033[32m'
  C_RED=$'\033[31m'
  C_YELLOW=$'\033[33m'
  C_CYAN=$'\033[36m'
  C_DIM=$'\033[2m'
else
  C_RESET= C_BOLD= C_GREEN= C_RED= C_YELLOW= C_CYAN= C_DIM=
fi

report() {
  local status=$1
  local label=$2
  local detail=${3:-}
  local emoji color

  case "$status" in
    PASS)
      emoji="✅"
      color=$C_GREEN
      PASS=$((PASS + 1))
      ;;
    FAIL)
      emoji="❌"
      color=$C_RED
      FAIL=$((FAIL + 1))
      ;;
    SKIP)
      emoji="⏭️"
      color=$C_YELLOW
      SKIP=$((SKIP + 1))
      ;;
    INFO)
      emoji="ℹ️"
      color=$C_CYAN
      SKIP=$((SKIP + 1))
      ;;
    *)
      emoji="•"
      color=$C_DIM
      ;;
  esac

  if [[ -n "$detail" ]]; then
    printf '%s%s %s%s  %s%s%s  %s%s%s\n' \
      "$color" "$emoji" "$status" "$C_RESET" \
      "$C_BOLD" "$label" "$C_RESET" \
      "$C_DIM" "$detail" "$C_RESET"
  else
    printf '%s%s %s%s  %s%s%s\n' \
      "$color" "$emoji" "$status" "$C_RESET" \
      "$C_BOLD" "$label" "$C_RESET"
  fi
}

# Resolve a base host from a URL that may already include a discovery path.
discovery_base_url() {
  local url=$1
  url=${url%/}
  case "$url" in
    */api/v2beta/services) printf '%s\n' "${url%/api/v2beta/services}" ;;
    */api/v2/services) printf '%s\n' "${url%/api/v2/services}" ;;
    */api/v1/services) printf '%s\n' "${url%/api/v1/services}" ;;
    */api/v1alpha/services) printf '%s\n' "${url%/api/v1alpha/services}" ;;
    */api/v1/auth-config) printf '%s\n' "${url%/api/v1/auth-config}" ;;
    */api/v1alpha/auth-config) printf '%s\n' "${url%/api/v1alpha/auth-config}" ;;
    */api/v2/auth-config) printf '%s\n' "${url%/api/v2/auth-config}" ;;
    */api/v2beta/auth-config) printf '%s\n' "${url%/api/v2beta/auth-config}" ;;
    *) printf '%s\n' "$url" ;;
  esac
}

join_url() {
  local base=$1
  local path=$2
  base=${base%/}
  if [[ "$path" != /* ]]; then
    path="/$path"
  fi
  printf '%s%s\n' "$base" "$path"
}

# HTTP GET; prints "curl_status http_code" and writes body to $3 when provided.
http_get() {
  local url=$1
  local body_file=${2:-}

  local curl_args=(
    --silent --show-error --location
    --max-time "$TIMEOUT_SECS"
    --header "$AUTH_HEADER"
    --header "Accept: application/json"
    --write-out '%{http_code}'
  )

  if [[ -n "$body_file" ]]; then
    curl_args+=(--output "$body_file")
  else
    curl_args+=(--output /dev/null)
  fi

  local code
  set +e
  code=$(curl "${curl_args[@]}" "$url")
  local status=$?
  set -e
  printf '%s %s\n' "$status" "${code:-000}"
}

probe_http() {
  local label=$1
  local url=$2
  local body_file=${3:-}

  local result code status
  result=$(http_get "$url" "$body_file")
  status=${result%% *}
  code=${result#* }

  if [[ "$status" -ne 0 || -z "$code" || "$code" == "000" ]]; then
    report FAIL "$label" "unreachable (${url})"
    return 1
  fi

  if [[ $REQUIRE_2XX -eq 1 && "$code" != 2* ]]; then
    report FAIL "$label" "HTTP ${code} (${url})"
    return 1
  fi

  # Discovery document endpoints should be 2xx even without --require-2xx.
  if [[ "$label" == discovery:* && "$code" != 2* ]]; then
    report FAIL "$label" "HTTP ${code} (${url})"
    return 1
  fi

  report PASS "$label" "HTTP ${code} (${url})"
  return 0
}

BASE_URL=$(discovery_base_url "$DISCOVERY_URL_IN")
AUTH_HEADER="Authorization: Bearer ${TOKEN}"
BODY_DIR=$(mktemp -d)
trap 'rm -rf "$BODY_DIR"' EXIT

echo "==> Discovery base: ${BASE_URL}"
echo "==> Checking discovery paths"
echo

V2_BODY="${BODY_DIR}/v2-services.json"
V2_OK=0

for path in "${DISCOVERY_PATHS[@]}"; do
  url=$(join_url "$BASE_URL" "$path")
  label="discovery:${path}"
  body_file=""
  if [[ "$path" == "/api/v2/services" ]]; then
    body_file=$V2_BODY
  elif [[ "$path" == "/api/v1/services" || "$path" == "/api/v1alpha/services" ]]; then
    body_file="${BODY_DIR}$(echo "$path" | tr '/' '-').json"
  fi

  if probe_http "$label" "$url" "$body_file"; then
    if [[ "$path" == "/api/v2/services" ]]; then
      V2_OK=1
    fi
  fi
done

echo
echo "==> Probing advertised service endpoints"

SCHEMA=""
SERVICE_COUNT=0

if [[ $V2_OK -eq 1 && -s "$V2_BODY" ]]; then
  SCHEMA=$(jq -r '."schema-version" // empty' "$V2_BODY" 2>/dev/null || true)
  SERVICE_COUNT=$(jq -r '.services | length // 0' "$V2_BODY" 2>/dev/null || echo 0)

  if [[ -z "$SCHEMA" ]]; then
    report FAIL "discovery:schema" "missing schema-version in /api/v2/services"
  elif [[ "$SCHEMA" != "2" && "$SCHEMA" != "1" ]]; then
    report FAIL "discovery:schema" "unsupported schema-version=${SCHEMA}"
  else
    report PASS "discovery:schema" "schema-version=${SCHEMA} services=${SERVICE_COUNT}"
  fi

  if [[ "$SCHEMA" == "1" ]]; then
    while IFS=$'\t' read -r name rest grpc || [[ -n "${name:-}" ]]; do
      [[ -z "${name:-}" ]] && continue
      if [[ -n "$rest" && "$rest" != "null" ]]; then
        probe_http "service:${name}/rest" "$rest" || true
      else
        report SKIP "service:${name}/rest" "not advertised"
      fi
      if [[ -n "$grpc" && "$grpc" != "null" ]]; then
        report INFO "service:${name}/grpc" "${grpc} (not probed)"
      fi
    done < <(jq -r '.services[]? | [.name // .id // "unknown", (.rest // ""), (.grpc // "")] | @tsv' "$V2_BODY" 2>/dev/null || true)
  elif [[ "$SCHEMA" == "2" ]]; then
    while IFS=$'\t' read -r service endpoint version base path || [[ -n "${service:-}" ]]; do
      [[ -z "${service:-}" ]] && continue
      if [[ -z "$path" || "$path" == "null" ]]; then
        report SKIP "service:${service}/${endpoint}@${version}" "no path"
        continue
      fi
      url=$(join_url "$base" "$path")
      probe_http "service:${service}/${endpoint}@${version}" "$url" || true
    done < <(jq -r '
      .services[]?
      | . as $svc
      | select($svc.rest != null)
      | $svc.rest.endpoints[]?
      | . as $ep
      | ($ep.versions | to_entries[])
      | [
          $svc.name,
          $ep.name,
          .key,
          $svc.rest.url,
          (.value.path // "")
        ]
      | @tsv
    ' "$V2_BODY" 2>/dev/null || true)

    while IFS=$'\t' read -r service grpc_url grpc_service || [[ -n "${service:-}" ]]; do
      [[ -z "${service:-}" ]] && continue
      report INFO "service:${service}/grpc" "${grpc_service} @ ${grpc_url} (not probed)"
    done < <(jq -r '
      .services[]?
      | select(.grpc != null)
      | . as $svc
      | $svc.grpc.endpoints[]?
      | [$svc.name, $svc.grpc.url, .service]
      | @tsv
    ' "$V2_BODY" 2>/dev/null || true)

    while IFS= read -r name || [[ -n "${name:-}" ]]; do
      [[ -z "${name:-}" ]] && continue
      report SKIP "service:${name}" "no rest or grpc block"
    done < <(jq -r '.services[]? | select(.rest == null and .grpc == null) | .name' "$V2_BODY" 2>/dev/null || true)
  fi
else
  report SKIP "advertised-services" "skipped; /api/v2/services did not return a usable body"
fi

echo
printf '%s==> Summary:%s %s✅ pass=%s%s  %s❌ fail=%s%s  %s⏭️ skip/info=%s%s\n' \
  "$C_BOLD" "$C_RESET" \
  "$C_GREEN" "$PASS" "$C_RESET" \
  "$C_RED" "$FAIL" "$C_RESET" \
  "$C_YELLOW" "$SKIP" "$C_RESET"

if [[ $FAIL -gt 0 ]]; then
  exit 1
fi

exit 0
