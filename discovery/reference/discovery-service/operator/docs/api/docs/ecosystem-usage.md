(discovery-capabilities-api-ecosystem-usage-ecosystem-usage)=
# Ecosystem Usage

(discovery-capabilities-api-ecosystem-usage-purpose)=
## Purpose

API discovery separates API selection from deployment-specific service locations. Each target service reports the REST resources and gRPC services it implements through the Capabilities API, including the versions and paths needed to identify those interfaces. A discovery service collects those reports, pairs each one with the URLs at which the target can be reached, and serves the result as a single catalog through the Discovery Catalog API. Clients read the catalog and select a compatible interface without relying on a fixed list of service types.

(discovery-capabilities-api-ecosystem-usage-participants)=
## Participants

The discovery workflow involves three roles:

1. A **target service** implements the Capabilities API and reports the public APIs it supports.
2. A **discovery service** finds target services, calls their Capabilities API, and serves the combined catalog through the Discovery Catalog API.
3. A **client** reads the catalog and uses its entries to locate a compatible REST resource version or fully qualified gRPC service.

The contracts define what a target service reports and what a client receives. They do not prescribe how a discovery service finds target services, determines their URLs, authenticates callers, or schedules refreshes. A discovery service may make those choices independently, as long as the catalog it serves follows the Discovery Catalog API.

(discovery-capabilities-api-ecosystem-usage-selection-workflow)=
## API Selection Workflow

A client calls `GET /api/v2beta/services` on a discovery service (falling back to `GET /api/v2/services` on older discovery services), checks that `schema-version` is `2`, and searches the `services` array for an entry that implements the API it needs. It may cache the catalog for as long as the response's `Cache-Control` header allows.

For REST, a client selects a service entry, then a named REST endpoint and one of its advertised versions. The client joins the entry's REST `url` with the version's `path` to form the request URL. When the catalog gives no `path`, the client takes the path from that resource's own API specification.

For gRPC, a client selects a fully qualified service name from the advertised endpoints and connects to the entry's gRPC `url` with a client generated for that service contract. The fully qualified name carries the API namespace and version.

Callers should treat transport blocks, endpoint lists, version details, and pagination tokens as optional unless the selected specification marks them as required. A caller should not derive a gRPC service name from a REST endpoint name or assume that the two representations have identical version identifiers.

(discovery-capabilities-api-ecosystem-usage-see-next)=
## See next

- {ref}`Discovery Catalog API v2beta <discovery-capabilities-api-discovery-catalog-api-discovery-catalog-api-v2beta>`
