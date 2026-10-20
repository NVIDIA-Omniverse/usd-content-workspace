# Capture a running deployment's versions

> **Applies to:** any Kubernetes deployment of this stack · **Sources:** Helm and kubectl behavior; Discovery catalog contract in [`discovery-catalog-api.md`](../../../../discovery/reference/discovery-service/operator/docs/api/docs/discovery-catalog-api.md)

These commands only read. Run them, or ask the caller to run them, with the caller's own cluster context and namespace; do not run them yourself unless asked. Record the results in the version record from [README.md](README.md).

## Helm releases

```bash
NS=<namespace>
helm list -n "$NS" -o json | jq -r '.[] | "\(.name)\t\(.chart)\t\(.app_version)\t\(.status)"'
```

`chart` is `<name>-<chart version>`. `app_version` is the chart's declared default, which overrides can change, so also read the images.

Values the deployment was installed with (redact before sharing; they can contain secret references):

```bash
helm get values <release> -n "$NS" -o yaml          # user-supplied overrides only
helm get values <release> -n "$NS" --all -o yaml    # merged with chart defaults
```

## Images actually running

```bash
kubectl get pods -n "$NS" -o json | jq -r '
  .items[] | .metadata.name as $p
  | (.spec.initContainers // [] + .spec.containers)[]
  | "\($p)\t\(.name)\t\(.image)"'
kubectl get pods -n "$NS" -o jsonpath='{range .items[*]}{.metadata.name}{"\t"}{range .status.containerStatuses[*]}{.imageID}{" "}{end}{"\n"}{end}'
```

The `imageID` digest is the strongest identity: tags can be re-pushed. Include init containers and sidecars (for example the Storage Service token provisioner and migration init container) and Helm test-hook images.

## API versions advertised

Query the Auto Discovery catalog from a network location the clients use:

```bash
curl -fsS https://<discovery-host>/api/v2/services | jq '{"schema-version": ."schema-version", services: [.services[] | {name, rest: [.rest.endpoints[]? | {name, versions: (.versions | keys)}], grpc: [.grpc.endpoints[]?.service]}]}'
```

Expect `schema-version` 2. Each gRPC entry is a fully qualified service name that includes the wire version. A service missing from the catalog is not reachable through discovery, whatever Helm reports; check its Service annotations (see [../deploy/discovery.md](../deploy/discovery.md)).

Legacy discovery deployments serve a static catalog from the legacy chart; their response format and route differ (see the operator's [v1 response schema](../../../../discovery/reference/discovery-service/operator/docs/v1-response-schema.md)).

## Cluster facts that change behavior

```bash
kubectl version -o json | jq -r '.serverVersion.gitVersion'   # e.g. native sidecars need 1.29+
kubectl get crd | grep -E 'httpproxies.projectcontour.io|certificates.cert-manager.io'
```

## Client side

- Kit or Client Library: `omni.client.get_version()` inside the running process.
- Navigator: the version shown in the UI or its image tag.
- Generated clients: record the definitions path and version they were generated from.

## Compare with a reference set

After capture, compare each component with the [released inventory](released-inventory.md) or [snapshot versions](source-snapshot.md). Report mismatches explicitly, for example "event-consumer-service image 1.12.0 is newer than the 1.3.0 release (1.11.0); use the 1.12.0 chart values and changelog".
