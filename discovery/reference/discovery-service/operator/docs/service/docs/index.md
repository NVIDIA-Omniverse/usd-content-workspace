(auto-discovery-service-index-index)=
# Omniverse Auto-Discovery Service

The Omniverse Auto-Discovery Service maintains a current catalog of APIs advertised by services in a Kubernetes cluster. It watches opted-in Kubernetes Services, queries each service through the Discovery Capabilities API, resolves internal and external addresses, and serves separate internal and external catalog views over HTTP.

Kubernetes discovery is enabled by default. A deployment can instead use static endpoint discovery, which bypasses Kubernetes completely. Target Kubernetes Services are not discovered until they opt in with the configured discovery annotation.

(auto-discovery-service-index-use-cases)=
## Use Cases

- Let clients find compatible REST resource versions and gRPC services without hardcoded service addresses.
- Maintain separate catalogs for cluster-internal and externally reachable APIs.
- Preserve the last known capability entry during a transient target-service query failure.
- Serve a legacy version 1 discovery response while clients migrate to the capability-oriented version 2 response.
- Publish authentication bootstrap information and route several discovery API versions through one ingress hostname.

(auto-discovery-service-index-architecture)=
## Architecture

In Kubernetes mode, Service and Contour HTTPProxy changes trigger reconciliation, and a periodic interval provides a fallback trigger. Each reconciliation lists the current resources, filters to opted-in Services, calls `CapabilitiesService.ListServices` on each target, resolves addresses, and replaces the internal and external catalog snapshots when they change.

The internal HTTP listener serves entries using cluster DNS addresses. The external listener serves only entries for which an explicit external address or suitable HTTPProxy route can be resolved. Both listeners expose the same service-specific HTTP paths, but they read different catalog snapshots.

In static mode, configured gRPC targets are queried directly at the reconciliation interval. Static entries are published to both catalog views, and no Kubernetes API access is attempted.

(auto-discovery-service-index-contents)=
## Contents

- {ref}`Deployment and Configuration <auto-discovery-service-deployment-deployment-and-configuration>`
- {ref}`Discovery and Reconciliation <auto-discovery-service-discovery-discovery-and-reconciliation>`
- {ref}`Endpoint Resolution <auto-discovery-service-endpoint-resolution-endpoint-resolution>`
- {ref}`Service-Specific HTTP API <auto-discovery-service-service-api-service-specific-http-api>`
- {ref}`Compatibility, Authentication, and Ingress <auto-discovery-service-compatibility-and-access-compatibility-authentication-and-ingress>`
- {ref}`Operations <auto-discovery-service-operations-operations>`

```{toctree}
:maxdepth: 1
:caption: Contents:

deployment
discovery
endpoint-resolution
service-api
compatibility-and-access
operations
```

(auto-discovery-service-index-see-next)=
## See next

- {ref}`Deployment and Configuration <auto-discovery-service-deployment-deployment-and-configuration>`
