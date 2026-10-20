# Core deployment scripts

> **Applies to:** the script layout in [../script-templates.md](../script-templates.md) · **Released:** profiles `1.3.0` and `published` · **This repo's snapshot:** chart names, ports, and labels checked against the snapshot Storage and operator charts (`app.kubernetes.io/name` = chart name; Storage REST 8012; operator internal 8080) · **Sources:** the [source skill package](../../../NOTICE.md)

All scripts assume they sit next to `.env` and `release-profile.sh`. Adapt the service list and values; do not add steps for services that were not requested.

## Shared helper: `lib.sh`

```bash
#!/usr/bin/env bash
# Archive verification. Prints the measured SHA-256 on success.
sha256_of() { if command -v sha256sum >/dev/null; then sha256sum -- "$1"; else shasum -a 256 -- "$1"; fi | awk '{print tolower($1)}'; }

verify_chart_archive() {
  local archive="$1" name="$2" version="$3" sums="${4:-}" base expected actual meta
  base="${archive##*/}"
  [[ -f "$archive" ]] || { echo "ERROR: missing archive $base" >&2; return 1; }
  if [[ "${CHART_SOURCE:-ngc}" == local && -z "$sums" ]]; then
    echo "ERROR: local archives need the owner's SHA256SUMS file" >&2; return 1
  fi
  actual="$(sha256_of "$archive")"
  if [[ -n "$sums" ]]; then
    expected="$(awk -v f="$base" '{sub(/^\*/,"",$2)} $2==f {print tolower($1)}' "$sums")"
    [[ "$(wc -l <<<"$expected")" -eq 1 && "$expected" =~ ^[0-9a-f]{64}$ ]] \
      || { echo "ERROR: no unique trusted checksum for $base" >&2; return 1; }
    [[ "$actual" == "$expected" ]] || { echo "ERROR: checksum mismatch for $base" >&2; return 1; }
  fi
  meta="$(helm show chart "$archive")" || { echo "ERROR: helm cannot read $base" >&2; return 1; }
  grep -qx "name: ${name}" <<<"$meta"            || { echo "ERROR: chart name mismatch in $base" >&2; return 1; }
  grep -Eqx "version: \"?${version}\"?" <<<"$meta" || { echo "ERROR: chart version mismatch in $base" >&2; return 1; }
  printf '%s\n' "$actual"
}
```

A self-measured hash is an identity record for the deployment, not proof of authenticity. Compare against a publisher digest when one exists.

## `00-setup.sh`

```bash
#!/usr/bin/env bash
set +x; set -euo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$DIR/.env"; source "$DIR/release-profile.sh"; source "$DIR/lib.sh"
select_release_profile "${STORAGE_RELEASE_PROFILE:-1.3.0}" "${SELECTED_SERVICES[@]}"
CHART_DIR="$DIR/charts/$RELEASE_PROFILE"; mkdir -p "$CHART_DIR"

echo "=== Setup: charts, namespace, pull secret (profile $RELEASE_PROFILE) ==="
for chart in "${CHARTS[@]}"; do
  name="${chart%:*}"; version="${chart#*:}"; archive="$CHART_DIR/${name}-${version}.tgz"
  if [[ ! -f "$archive" ]]; then
    case "${CHART_SOURCE:-ngc}" in
      local) cp -n "${LOCAL_CHART_DIR:?set LOCAL_CHART_DIR}/${name}-${version}.tgz" "$archive" ;;
      ngc)
        echo ">>> Fetching ${name}:${version}"
        # Output is suppressed: it can contain signed download URLs.
        ngc registry chart pull "${NGC_CHART_NAMESPACE:-nvidia/omniverse}/${chart}" --dest "$CHART_DIR" >/dev/null 2>&1 \
          || { echo "ERROR: NGC could not deliver ${name}:${version}. Check entitlement, local NGC login, and network; do not change versions."; exit 1; } ;;
      *) echo "ERROR: CHART_SOURCE must be ngc or local"; exit 1 ;;
    esac
  fi
  verify_chart_archive "$archive" "$name" "$version" "${CHART_SHA256_FILE:-}" > "$archive.sha256"
done

echo ">>> Namespace $NAMESPACE"
$KUBECTL create namespace "$NAMESPACE" --dry-run=client -o yaml | $KUBECTL apply -f -

if ! $KUBECTL get secret "$PULL_SECRET_NAME" -n "$NAMESPACE" >/dev/null 2>&1; then
  : "${NGC_API_KEY:?Provide an existing $PULL_SECRET_NAME, or set NGC_API_KEY locally in .env}"
  echo ">>> Creating pull secret $PULL_SECRET_NAME"
  $KUBECTL create secret docker-registry "$PULL_SECRET_NAME" --docker-server=nvcr.io \
    --docker-username='$oauthtoken' --docker-password="$NGC_API_KEY" \
    -n "$NAMESPACE" --dry-run=client -o yaml | $KUBECTL apply -f - >/dev/null
fi
echo "=== DONE: setup ==="
```

`ngc registry chart pull` writes the archive under `--dest`; if your NGC CLI version writes a different file name or a directory, adjust the `archive` path after checking its output once by hand.

## `01-deploy-storage.sh`

Prepare `values/storage-values.yaml` from the selected chart's `helm show values` and [../production-stack/storage-service.md](../production-stack/storage-service.md). The script does not create identities; for web identity it expects the role annotation in the values.

```bash
#!/usr/bin/env bash
set +x; set -euo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$DIR/.env"; source "$DIR/release-profile.sh"; source "$DIR/lib.sh"
select_release_profile "${STORAGE_RELEASE_PROFILE:-1.3.0}" storage
CHART="$DIR/charts/$RELEASE_PROFILE/storage-service-${STORAGE_VERSION}.tgz"
verify_chart_archive "$CHART" storage-service "$STORAGE_VERSION" "${CHART_SHA256_FILE:-}" >/dev/null

VALUES=(-f "$DIR/values/storage-values.yaml")
[[ -f "$DIR/values/ingress-values.yaml" ]] && VALUES+=(-f "$DIR/values/ingress-values.yaml")

echo "=== Deploy: Storage Service ${STORAGE_VERSION} ==="
$KUBECTL get secret "$PULL_SECRET_NAME" -n "$NAMESPACE" >/dev/null || { echo "ERROR: run 00-setup.sh first"; exit 1; }
helm lint "$CHART" "${VALUES[@]}"
helm template storage-service "$CHART" -n "$NAMESPACE" "${VALUES[@]}" >/dev/null

case "${S3_CREDENTIAL_MODE:-${AZ_CREDENTIAL_MODE:?set S3_CREDENTIAL_MODE or AZ_CREDENTIAL_MODE}}" in
  access-key)
    : "${STORAGE_SERVICE_AWS_ACCESS_KEY_ID_1:?}"; : "${STORAGE_SERVICE_AWS_SECRET_ACCESS_KEY_1:?}"
    echo ">>> Creating storage-service-secrets-aws (referenced by extraEnvFrom)"
    $KUBECTL create secret generic storage-service-secrets-aws -n "$NAMESPACE" \
      --from-literal=STORAGE_SERVICE_AWS_ACCESS_KEY_ID_1="$STORAGE_SERVICE_AWS_ACCESS_KEY_ID_1" \
      --from-literal=STORAGE_SERVICE_AWS_SECRET_ACCESS_KEY_1="$STORAGE_SERVICE_AWS_SECRET_ACCESS_KEY_1" \
      --dry-run=client -o yaml | $KUBECTL apply -f - >/dev/null ;;
  storage-key)
    : "${STORAGE_SERVICE_AZURE_STORAGE_KEY_1:?}"
    $KUBECTL create secret generic storage-service-secrets-azure -n "$NAMESPACE" \
      --from-literal=STORAGE_SERVICE_AZURE_STORAGE_KEY_1="$STORAGE_SERVICE_AZURE_STORAGE_KEY_1" \
      --dry-run=client -o yaml | $KUBECTL apply -f - >/dev/null ;;
  web-identity|federated|credentials-file|token-file|anonymous)
    echo ">>> Using the credential provider configured in storage-values.yaml" ;;
  *) echo "ERROR: unsupported credential mode"; exit 1 ;;
esac

helm upgrade --install storage-service "$CHART" -n "$NAMESPACE" "${VALUES[@]}"
$KUBECTL rollout status deployment/storage-service -n "$NAMESPACE" --timeout=180s
echo "=== DONE: Storage Service ==="
```

## `02-deploy-discovery.sh`

`discovery-values.yaml` differs by profile: operator values for `1.3.0` ([../discovery.md](../discovery.md#install-the-operator)), static `discovery.services` for `published` (set `image.repository`/`image.tag` from `DISCOVERY_IMAGE`). Never pass one schema to the other chart.

```bash
#!/usr/bin/env bash
set +x; set -euo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$DIR/.env"; source "$DIR/release-profile.sh"; source "$DIR/lib.sh"
select_release_profile "${STORAGE_RELEASE_PROFILE:-1.3.0}" discovery
CHART="$DIR/charts/$RELEASE_PROFILE/${DISCOVERY_CHART}-${DISCOVERY_VERSION}.tgz"
verify_chart_archive "$CHART" "$DISCOVERY_CHART" "$DISCOVERY_VERSION" "${CHART_SHA256_FILE:-}" >/dev/null

VALUES=(-f "$DIR/values/discovery-values.yaml")
[[ -f "$DIR/values/ingress-values.yaml" ]] && VALUES+=(-f "$DIR/values/ingress-values.yaml")
[[ -f "$DIR/values/oidc-config.yaml"    ]] && VALUES+=(-f "$DIR/values/oidc-config.yaml")

echo "=== Deploy: ${DISCOVERY_CHART} ${DISCOVERY_VERSION} ==="
helm lint "$CHART" "${VALUES[@]}"
helm template "$DISCOVERY_CHART" "$CHART" -n "$NAMESPACE" "${VALUES[@]}" >/dev/null
helm upgrade --install "$DISCOVERY_CHART" "$CHART" -n "$NAMESPACE" "${VALUES[@]}"
$KUBECTL rollout status "deployment/${DISCOVERY_CHART}" -n "$NAMESPACE" --timeout=120s
echo "=== DONE: Discovery; annotate target Services for the operator (see discovery.md) ==="
```

## `99-validate.sh`

Checks only what it can prove; prints PASS/FAIL/SKIP and never prints command arguments.

```bash
#!/usr/bin/env bash
set +x; set -euo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$DIR/.env"; source "$DIR/release-profile.sh"
select_release_profile "${STORAGE_RELEASE_PROFILE:-1.3.0}" "${SELECTED_SERVICES[@]}"
PASS=0; FAIL=0; PIDS=(); TMP="$(mktemp -d)"
cleanup() { for p in "${PIDS[@]:-}"; do kill "$p" 2>/dev/null || true; done; rm -rf "$TMP"; }
trap cleanup EXIT
check() { local n="$1"; shift; if "$@" >/dev/null 2>&1; then echo "PASS: $n"; PASS=$((PASS+1)); else echo "FAIL: $n"; FAIL=$((FAIL+1)); fi; }
running() { $KUBECTL get pods -n "$NAMESPACE" -l "app.kubernetes.io/name=$1" -o jsonpath='{.items[*].status.phase}' | grep -q Running; }
forward() {   # forward NAME SVC LOCAL:REMOTE
  $KUBECTL port-forward --address 127.0.0.1 -n "$NAMESPACE" "service/$2" "$3" >"$TMP/$1.log" 2>&1 & PIDS+=($!)
  for _ in $(seq 1 100); do grep -q '^Forwarding from 127.0.0.1' "$TMP/$1.log" && return 0; sleep 0.1; done; return 1
}

check "storage-service pods Running" running storage-service
if forward storage storage-service 18012:8012; then
  check "Storage REST capabilities" curl -fsS --max-time 15 http://127.0.0.1:18012/v1alpha/capabilities/services
else echo "FAIL: storage port-forward"; FAIL=$((FAIL+1)); fi

check "${DISCOVERY_CHART} pods Running" running "$DISCOVERY_CHART"
if forward discovery "$DISCOVERY_CHART" 18080:8080; then
  check "Discovery ${DISCOVERY_API_PATH}" curl -fsS --max-time 15 "http://127.0.0.1:18080${DISCOVERY_API_PATH}"
  if [[ "$DISCOVERY_API_PATH" == /api/v2/services ]]; then
    check "catalog schema-version 2" bash -c "curl -fsS http://127.0.0.1:18080/api/v2/services | jq -e '.\"schema-version\" == 2'"
    check "storage registered" bash -c "curl -fsS http://127.0.0.1:18080/api/v2/services | jq -e '[.services[].grpc.endpoints[]?.service] | any(test(\"^nvidia\\\\.omniverse\\\\.storage\\\\.\"))'"
  fi
  [[ -f "$DIR/values/oidc-config.yaml" ]] && check "auth-config served" curl -fsS --max-time 15 http://127.0.0.1:18080/api/v1/auth-config
else echo "FAIL: discovery port-forward"; FAIL=$((FAIL+1)); fi

# Other services append their own checks here (see their deploy references).
echo "=== Validation: $PASS passed, $FAIL failed ==="
echo "NOTE: in-cluster checks only. Validate external URLs, auth (401 without a token), and read/write with real identities separately."
(( FAIL == 0 ))
```

## `deploy-all.sh`

```bash
#!/usr/bin/env bash
set +x; set -euo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
[[ -f "$DIR/.env" ]] || { echo "Copy .env.example to .env and fill it in."; exit 1; }
for s in 00-setup.sh 01-deploy-storage.sh 02-deploy-discovery.sh 03-deploy-rabbitmq.sh 04-deploy-notifications.sh \
         05-deploy-contour.sh 06-deploy-auth.sh 07-deploy-navigator.sh 08-deploy-live-edit.sh 09-deploy-integration-tests.sh; do
  [[ -f "$DIR/$s" ]] && { echo ">>> $s"; bash "$DIR/$s"; }
done
bash "$DIR/99-validate.sh"
```

Integration tests run last so they can discover every installed service, including Live Edit.

## Optional observability overlay

For Storage Service OTLP export, layer a file such as this (keys from the snapshot [`observability.rst`](../../../../../storage/reference/storage-service/docs/helm/observability.rst); confirm they exist in your chart version):

```yaml
observability:
  metrics:
    enabled: true
    endpoint: { grpc: "http://otel-collector.observability.svc.cluster.local:4317" }
    exportIntervalMs: 15000
  tracing:
    enabled: true
    endpoint: { grpc: "http://otel-collector.observability.svc.cluster.local:4317" }
  logging:
    structured: { enabled: true }
config:
  logging: { level: info }
```

`observability.metrics.enabled` takes precedence over `service.metrics.enabled` (Prometheus on 8013). Other services' telemetry keys differ; see [../../operate/monitoring.md](../../operate/monitoring.md).
