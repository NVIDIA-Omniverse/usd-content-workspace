# Repeatable deployment scripts and values layout

> **Applies to:** scripted installs of the production stack (Storage Service and Discovery here; other services add their own numbered scripts) · **Released:** version pins from the Developer Guide 1.3.0 inventory and the previous `published` inventory ([released-inventory.md](../versions/released-inventory.md)) · **This repo's snapshot:** not used by default; see "Own builds" below · **Sources:** the [source skill package](../../NOTICE.md) release pins

Use these templates when someone asks for deployment scripts, either to run themselves or for an agent to run on their behalf with their authorization. Generate the files in the user's deployment directory, show the script bodies and the order to run them, and stop at any input that is not known yet instead of inventing it.

The script bodies are in [script-templates/core-scripts.md](script-templates/core-scripts.md). Service-specific scripts for notifications, auth, permissions, user info, thumbnails, Live Edit, and integration tests follow the same conventions; their values come from [notifications.md](notifications.md), [permissions-and-auth.md](permissions-and-auth.md), [userinfo.md](userinfo.md), [thumbnails.md](thumbnails.md), [live-edit.md](live-edit.md), and [../test/integration-tests.md](../test/integration-tests.md).

## Directory layout

```
usd-content-workspace-deploy/
├── .env.example                 # template, committed
├── .env                         # real values, never committed
├── .gitignore
├── release-profile.sh           # pure version selection; no network or cluster calls
├── 00-setup.sh                  # acquire exact charts, namespace, pull secret
├── 01-deploy-storage.sh
├── 02-deploy-discovery.sh
├── 03-deploy-rabbitmq.sh        # if notifications   -> notifications.md
├── 04-deploy-notifications.sh   # if notifications   -> notifications.md
├── 05-deploy-contour.sh         # if ingress         -> production-stack/ingress-and-tls.md
├── 06-deploy-auth.sh            # if auth            -> permissions-and-auth.md
├── 07-deploy-navigator.sh       # if Navigator       -> navigator.md
├── 08-deploy-live-edit.sh       # if Live Edit       -> live-edit.md
├── 09-deploy-integration-tests.sh  # run last        -> ../test/integration-tests.md
├── 99-validate.sh
├── deploy-all.sh
├── values/
│   ├── storage-values.yaml
│   ├── discovery-values.yaml
│   ├── ingress-values.yaml      # shared overlay, if ingress
│   ├── oidc-config.yaml         # if auth
│   ├── navigator-values.yaml
│   └── ...                      # one file per selected service
└── charts/<profile>/            # fetched archives, gitignored
```

Use a separate deployment directory per release profile so values and archives from one profile cannot be reused with another. Changing the profile does not upgrade a running service; follow [../operate/upgrades.md](../operate/upgrades.md).

## `.env.example`

Include only variables for the selected services. Secrets stay empty in the template.

```bash
# Copy to .env and fill in. Never commit .env.

# --- Core ---
STORAGE_RELEASE_PROFILE=1.3.0          # 1.3.0 (current) | published (previous)
SELECTED_SERVICES=(storage discovery)  # add only what is requested
NAMESPACE=                             # e.g. usd-content-workspace
KUBECTL=kubectl                        # "microk8s kubectl" on MicroK8s
CHART_SOURCE=ngc                       # ngc (local NGC CLI profile) | local (exact archives)
NGC_CHART_NAMESPACE=nvidia/omniverse
LOCAL_CHART_DIR=                       # CHART_SOURCE=local only
CHART_SHA256_FILE=                     # trusted SHA256SUMS; required for local archives
PULL_SECRET_NAME=ngcpull-secret        # existing secret preferred
NGC_API_KEY=                           # only if the pull secret must be created; type it locally, never paste in chat

# --- Storage backend ---
S3_BUCKET_NAME=
S3_REGION=
S3_CREDENTIAL_MODE=                    # web-identity | access-key | credentials-file | anonymous
# STORAGE_SERVICE_AWS_ACCESS_KEY_ID_1=       # access-key mode only
# STORAGE_SERVICE_AWS_SECRET_ACCESS_KEY_1=   # access-key mode only
# AZ_STORAGE_ACCOUNT=                        # Azure instead of S3
# AZ_CREDENTIAL_MODE=                        # federated | storage-key | token-file | anonymous
# STORAGE_SERVICE_AZURE_STORAGE_KEY_1=       # storage-key mode only

# --- Ingress ---
# DNS_DOMAIN=                          # e.g. storage.example.com
# TLS_SECRET_NAME=usd-content-workspace-cert    # created by cert-manager or your PKI, not by these scripts

# --- Auth bootstrap (Discovery openId) ---
# OIDC_CONFIGURATION_URI=
# OIDC_CLIENT_ID=
```

`.gitignore`:

```
.env
charts/
*.tgz
rendered/
```

## Script conventions

1. `set +x` then `set -euo pipefail`. Never enable tracing: it prints secrets.
2. Source `.env` and `release-profile.sh`, then call `select_release_profile`.
3. Print each step (`echo ">>> ..."`) before running it.
4. Check prerequisites first: cluster reachable, namespace exists, pull secret exists, archive verified.
5. Validate values with `helm lint` and `helm template ... > /dev/null` using the same ordered `-f` list as the install. Discard rendered output or keep it in a protected directory; it can contain Secrets.
6. After each install, `kubectl rollout status` with a timeout.
7. Do not print `.env`, authorization headers, signed URLs, or Secret contents. Do not log command arguments in failure messages.
8. NGC CLI login and cluster image pulls are separate; a working CLI profile does not create the pull secret.

## `release-profile.sh`

Pure selection: sets version variables and the chart list for the requested tasks. Values must still be prepared from each selected archive's `helm show values`.

```bash
#!/usr/bin/env bash
# select_release_profile PROFILE [TASK ...]
select_release_profile() {
  RELEASE_PROFILE=; CHARTS=()
  local profile="${1:-1.3.0}" task; (( $# )) && shift
  (( $# )) || set -- storage discovery
  case "$profile" in
    1.3.0)
      STORAGE_VERSION=1.2.3; DISCOVERY_CHART=auto-discovery-service; DISCOVERY_VERSION=1.1.1
      DISCOVERY_IMAGE=nvcr.io/nvidia/omniverse/auto-discovery-service:1.1.1; DISCOVERY_API_PATH=/api/v2/services
      AGGREGATION_VERSION=1.6.0; CONSUMER_VERSION=1.11.0; ENVOY_VERSION=2.4.0; PERMISSION_VERSION=0.0.53
      USERINFO_VERSION=0.0.8; NAVIGATOR_VERSION=1.0.4; INTEGRATION_TESTS_VERSION=1.0.23
      SMOKE_TEST_VERSION=1.0.9; LIVE_EDIT_VERSION=0.1.1; THUMBNAIL_VERSION=0.1.0 ;;
    published)
      STORAGE_VERSION=1.2.0; DISCOVERY_CHART=discovery-service; DISCOVERY_VERSION=2.3.11
      DISCOVERY_IMAGE=nvcr.io/nvidia/omniverse/simple-nginx:1.1.0; DISCOVERY_API_PATH=/api/v1/services
      AGGREGATION_VERSION=1.5.6; CONSUMER_VERSION=1.10.0; ENVOY_VERSION=2.3.3; PERMISSION_VERSION=0.0.52
      USERINFO_VERSION=0.0.7; NAVIGATOR_VERSION=1.0.3; INTEGRATION_TESTS_VERSION=1.0.18
      SMOKE_TEST_VERSION=1.0.7; LIVE_EDIT_VERSION=; THUMBNAIL_VERSION= ;;
    *) echo "ERROR: STORAGE_RELEASE_PROFILE must be 1.3.0 or published." >&2; return 1 ;;
  esac
  RABBITMQ_CHART_VERSION=99.3.0; RABBITMQ_IMAGE_TAG=4.1.3-debian-12-r1; PERMISSION_UI_VERSION=0.0.22
  for task in "$@"; do
    case "$task" in
      storage)       CHARTS+=("storage-service:${STORAGE_VERSION}") ;;
      discovery)     CHARTS+=("${DISCOVERY_CHART}:${DISCOVERY_VERSION}") ;;
      notifications) CHARTS+=("rabbitmq:${RABBITMQ_CHART_VERSION}" "event-aggregation-service:${AGGREGATION_VERSION}" "event-consumer-service:${CONSUMER_VERSION}") ;;
      auth)          CHARTS+=("envoy-auth-extension:${ENVOY_VERSION}") ;;
      permissions)   CHARTS+=("permission-service:${PERMISSION_VERSION}") ;;
      permission-ui) CHARTS+=("permission-ui:${PERMISSION_UI_VERSION}") ;;
      userinfo)      CHARTS+=("microsoft-graph-userinfo-service:${USERINFO_VERSION}") ;;
      navigator)     CHARTS+=("storage-navigator:${NAVIGATOR_VERSION}") ;;
      integration-tests) CHARTS+=("storage-api-integration-tests:${INTEGRATION_TESTS_VERSION}") ;;
      live-edit)
        [[ -n "$LIVE_EDIT_VERSION" ]] || { echo "ERROR: live-edit is not in the $profile inventory." >&2; return 1; }
        CHARTS+=("live-edit-service:${LIVE_EDIT_VERSION}") ;;
      thumbnail)
        [[ -n "$THUMBNAIL_VERSION" ]] || { echo "ERROR: thumbnail is not in the $profile inventory." >&2; return 1; }
        CHARTS+=("thumbnail-service:${THUMBNAIL_VERSION}") ;;
      contour) ;;   # third-party; acquired separately
      *) echo "ERROR: unknown task: $task" >&2; return 1 ;;
    esac
  done
  RELEASE_PROFILE="$profile"
}
```

Smoke-test images: the released charts pin `smoke-test-base` 1.0.9 for 1.3.0; previous-profile charts default to 1.0.1 and need `smokeTest.image.tag: "1.0.7"` when smoke tests are enabled. RabbitMQ 99.3.0 defaults to a staging repository prefix; set its registry, prefix, repository, and tag explicitly ([notifications.md](notifications.md)).

## Own builds (snapshot)

To deploy images built from this repository ([building-from-source.md](building-from-source.md)), do not add a third profile to the released map. Instead set `CHART_SOURCE=local`, point `LOCAL_CHART_DIR` at charts you packaged with real versions, provide your own `CHART_SHA256_FILE`, and set `image.repository`/`image.tag` in each values file to your registry. Snapshot charts have different defaults from released ones (for example Storage Service `config.urlNormalizer.enabled: true`); prepare values from the snapshot chart.

## Running

- **Manual:** `cp .env.example .env`, fill it in, review every values file, then run `bash 00-setup.sh`, the numbered scripts in order, and `bash 99-validate.sh`. Or `bash deploy-all.sh`.
- **Agent on the user's behalf:** generate the files, show the values and scripts, and run only with the user's authorization against the cluster context they confirmed. Report each script's real outcome; a skipped or failed step is not a deployment.
- **Offline preparation only:** source `release-profile.sh`, verify archives, and run `helm lint`/`helm template`. Do not run `00-setup.sh` or the deploy scripts; they create cluster resources.
