# Artifact access: NGC charts, images, API resources, or your own builds

> **Applies to:** every chart and image in the stack · **Released:** NGC inventory for usd-content-workspace Developer Guide 1.3.0 (see [released-inventory.md](../versions/released-inventory.md)) · **This repo's snapshot:** charts carry `version: "0.0.0"` placeholders; image defaults read from each chart's `values.yaml` at commit `189896e` · **Sources:** the [source skill package](../../NOTICE.md); [NGC collection](https://catalog.ngc.nvidia.com/orgs/nvidia/teams/omniverse/collections/storage_apis); chart `values.yaml` files listed below; [`IMPORT_STATUS.md`](../../../../IMPORT_STATUS.md)

There are two ways to get runnable artifacts:

1. **Released artifacts from NGC** — signed-off charts and images in the `nvidia/omniverse` org/team. Use these for new installations unless you have a reason not to.
2. **Your own builds from this repository** — images built from the component Dockerfiles and charts packaged from the component `helm/` directories. Use these when you need a snapshot change that is not released, or a patched build. See [building-from-source.md](building-from-source.md).

Do not mix the two in one release of a component (for example a snapshot chart with a released image) without reading both the chart's `values.yaml` and the component `CHANGELOG.md`: chart keys and defaults change between versions.

## Four separate checks

Each of these can fail independently. A success in one says nothing about the next.

| Check | Credential | Proves |
| --- | --- | --- |
| NGC entitlement | NGC account in an org/team entitled to the usd-content-workspace NGC collection | You can see the collection |
| Chart or resource download | NGC API key in the local NGC CLI profile or Helm repository credentials | You can fetch that exact `name:version` |
| Image pull in the cluster | A Kubernetes `docker-registry` Secret for `nvcr.io`, referenced by the chart | The kubelet can pull that image |
| Running service | Backend identity (S3/Azure), service identity, caller tokens | The service works |

An NGC catalog page returning HTTP 200 does not prove that an archive downloads or an image pulls. A 401/403 is an access observation: check entitlement and local authentication; do not change versions to work around it.

The NGC API key is an **acquisition** credential only. It is not a Storage API credential, not a backend credential, and must never be configured as an application bearer token.

## Get entitlement and an API key

1. Create or use an NGC account ([NGC catalog user guide](https://docs.nvidia.com/ngc/latest/ngc-catalog-user-guide.html)).
2. Get access to the [usd-content-workspace NGC collection](https://catalog.ngc.nvidia.com/orgs/nvidia/teams/omniverse/collections/storage_apis) in org/team `nvidia/omniverse`. It holds the prebuilt containers, Helm charts, and API resources. The collection page lists the subscriptions that include it: NVIDIA AI Enterprise, NVIDIA Omniverse Enterprise, or the free NVIDIA Developer Program. Sign in to NGC with the account that holds the subscription. The collection still uses its earlier catalog name and URL, which will change; follow the link above rather than searching the catalog by name.
3. Generate an API key with the registry permissions your org requires ([NGC API keys](https://docs.nvidia.com/ngc/latest/ngc-private-registry-user-guide.html#ngc-api-keys)).
4. Configure the NGC CLI yourself in your terminal with `ngc config set`. Type the key into the local prompt, never into a chat or a file under version control.

Agents must not ask for, print, or read the key. They can use an already-configured CLI profile when the user authorizes downloads.

## Download charts and API resources

Use exact versions from the [released inventory](../versions/released-inventory.md). Chart and image versions are independent; chart `appVersion` is not a reliable image tag.

With the NGC CLI (stores credentials in the local profile):

```bash
# Chart archive, e.g. storage-service 1.2.3
ngc registry chart pull nvidia/omniverse/storage-service:1.2.3 --dest ./charts/1.3.0

# API resource (specifications and examples), e.g. storage-api
ngc registry resource download-version "nvidia/omniverse/storage-api:<version>"
```

With Helm against the NGC chart repository (the URL pattern `https://helm.ngc.nvidia.com/nvidia/omniverse/charts/<chart>-<version>.tgz` is the one the Storage Service skill uses). Keep the key out of process arguments and shell history:

```bash
read -rs NGC_API_KEY   # type the key; nothing is echoed
helm repo add ngc-omniverse https://helm.ngc.nvidia.com/nvidia/omniverse \
  --username '$oauthtoken' --password-stdin <<<"${NGC_API_KEY}"
helm pull ngc-omniverse/storage-service --version 1.2.3 --destination ./charts/1.3.0
unset NGC_API_KEY
```

After download, before rendering or installing:

- `helm show chart <archive>` and confirm `name` and `version`.
- `helm show values <archive>` and prepare values from **that** output, not from this repository's snapshot `values.yaml`.
- Record a SHA-256 of the archive with the deployment. A self-computed hash is an identity record, not a publisher attestation; compare against a publisher digest when you have one.
- Check rendered image references, including init containers, sidecars, and Helm test hooks (smoke-test images).

## Image pull secrets

Create the pull secret once per namespace, or use your organization's secret provisioning (External Secrets, Vault agent, and so on). Prefer an existing secret.

```bash
read -rs NGC_API_KEY
kubectl create secret docker-registry ngcpull-secret \
  --docker-server=nvcr.io \
  --docker-username='$oauthtoken' \
  --docker-password="${NGC_API_KEY}" \
  -n <namespace> --dry-run=client -o yaml | kubectl apply -f -
unset NGC_API_KEY
```

Charts reference pull secrets differently. The snapshot charts use these keys (check the released chart's values too):

| Chart (snapshot path) | Default image repository | Pull-secret key | Snapshot default |
| --- | --- | --- | --- |
| Storage Service ([`values.yaml`](../../../../storage/reference/storage-service/helm/values.yaml)) | `nvcr.io/nvidia/omniverse/storage-service` | `image.pullSecrets: [{name: ...}]` | `[]`; `image.tag: "0.0.0"` (set it) |
| Auto Discovery ([`values.yaml`](../../../../discovery/reference/discovery-service/operator/helm/auto-discovery-service/values.yaml)) | `nvcr.io/nvidia/omniverse/auto-discovery-service` | `image.pullSecrets`, also top-level `imagePullSecrets` | `[{name: regcred}]` — override it, or pods reference a Secret that may not exist |
| Legacy Discovery ([`values.yaml`](../../../../discovery/reference/discovery-service/chart/helm/values.yaml)) | `nginx:1.31` (Docker Hub) in the snapshot; the released 2.3.11 profile uses `nvcr.io/nvidia/omniverse/simple-nginx:1.1.0` | `image.pullSecrets` | empty |
| Storage Navigator ([`values.yaml`](../../../../examples/navigator-ui/helm/storage-navigator/values.yaml)) | `nvcr.io/nvidia/omniverse/storage-navigator` | `image.pullSecrets`, also top-level `imagePullSecrets` | `[]` |
| Smoke tests (Storage, legacy Discovery, event services) | `nvcr.io/nvidia/omniverse/smoke-test-base` | `smokeTest.image.pullSecrets` | tag `1.0.5` (Storage) / `1.0.1` (others) in the snapshot; the 1.3.0 release pins `1.0.9` |

Other components' charts: see [notifications.md](notifications.md), [permissions-and-auth.md](permissions-and-auth.md), [userinfo.md](userinfo.md), [thumbnails.md](thumbnails.md), [live-edit.md](live-edit.md).

## Offline or mirrored installs

- Mirror exact archives and images into your registry; keep the original `name:version` and digests.
- Override `image.repository`/`image.tag` (and smoke-test images) to the mirror, and point pull secrets at the mirror's credentials.
- Verify archives with the owner's checksum file before rendering. Do not fetch substitute versions when an exact artifact is missing; that blocks only the dependent steps.
- `helm lint` and `helm template` can run offline against local archives. Treat rendered output as sensitive (it can contain Secret manifests).

## Released artifacts versus building your own

| | Released (NGC) | Built from this repository |
| --- | --- | --- |
| Versions | Fixed release inventory | Snapshot; often ahead of the release (for example Storage Service `[Unreleased]` enables URL normalization by default) |
| Chart version | Real version | `Chart.yaml` holds `0.0.0`; set `--version`/`--app-version` when packaging and always set `image.tag` |
| Images | `nvcr.io/nvidia/omniverse/*` | Your registry. Base images are public; Navigator's optional permission panel builds from in-repo source and is left out of a component-directory Docker build; see [building-from-source.md](building-from-source.md) |
| Support | Release documentation and NGC | Source-level only: CI here checks and tests some components and builds some images ([`IMPORT_STATUS.md`](../../../../IMPORT_STATUS.md)), but runs no runtime integration tests |

Use released artifacts for production unless you own the build pipeline, image scanning, and upgrade testing for your custom builds.
