# Migrating Discovery Clients from v1 to v2

This guide describes how clients of the original v1 discovery response can adopt the response returned by `GET /api/v2beta/services`.

Version 2 is not a field-for-field replacement for version 1. In particular, v1 describes one service *type* and up to two transport URLs per service. v2 reports the capabilities discovered from each service and groups them by transport, endpoint, and version.

## 1. Read the new endpoint and validate the schema version

Change the discovery request to:

```text
GET /api/v2beta/services
```

Discovery services also serve the same response at `GET /api/v2/services`. Use that path only as a fallback for discovery services that return `404 Not Found` for `/api/v2beta/services`.

Require `"schema-version": 2` in the response before processing it. Do not infer the version from the endpoint path alone.

The v2 response returns a top-level object with a `services` array, as v1 did.

## 2. Update the service model

Replace the v1 model:

```text
id, name, type, rest?: string, grpc?: string
```

with a model equivalent to:

```text
name, capability_source, rest?: { url, endpoints }, grpc?: { url, endpoints }
```

| v1 field | v2 equivalent | Migration action |
| --- | --- | --- |
| `id` | No direct equivalent | Use `name` as the discovered-service identifier only if it is suitable for the client. Do not assume a stable v1 `id` is available. |
| `name` | `name` | Continue using it as the service name. |
| `type` | No direct equivalent | Select services by the API endpoint and version they expose, rather than by a fixed type enum. |
| `rest` URL string | `rest.url` | Read the base URL only when the optional `rest` block is present. |
| `grpc` URL string | `grpc.url` | Read the base URL only when the optional `grpc` block is present. |
| Not available | `capability_source` | This field is for debugging reference only. |

Both `rest` and `grpc` are optional. Their absence means that transport is not available for the service; it is not an empty endpoint list.

## 3. Discover REST APIs by endpoint and version

In v1, a client commonly used the service `type` to decide whether to call the single `rest` URL. In v2, inspect `rest.endpoints` instead:

1. Select the service by `name` or by the REST API endpoint required.
2. Confirm that `rest` is present.
3. Find the item in `rest.endpoints` whose `name` is the API required.
4. Select a supported key in that endpoint's `versions` map, such as `v1` or `v1alpha`.
5. Join `rest.url` with the selected version's `path`.

For example, the callable URL for `fileobject` version `v1` is constructed from:

```json
{
  "url": "https://storage.example.com",
  "endpoints": [
    {
      "name": "fileobject",
      "versions": {
        "v1": { "path": "/api/v1/fileobject" }
      }
    }
  ]
}
```

Result: `https://storage.example.com/api/v1/fileobject`.


## 4. Discover gRPC APIs by fully qualified service name

In v2, `grpc.endpoints` is a flat array of objects with a `service` field:

```json
{
  "url": "https://grpc.storage.example.com",
  "endpoints": [
    {
      "service": "nvidia.omniverse.storage.fileobject.v1.FileObjectService"
    }
  ]
}
```

To use gRPC:

1. Confirm that `grpc` is present.
2. Find a `grpc.endpoints[*].service` value supported by the client.
3. Connect to `grpc.url`.
4. Invoke the selected fully qualified gRPC service using the generated client for that API version.

Version and API identity are encoded in the fully qualified service name. Clients should match the complete `service` string or parse it only according to their own protobuf naming convention. Do not expect the REST-style `name` and `versions` map under `grpc.endpoints`.

## 5. Replace type-based routing

The v1 `type` enumeration (`thumbnail`, `search`, `tagging`, `permission`, and `storage`) is not emitted in v2. Replace code such as “find the service whose type is `storage`” with a capability requirement, for example:

- REST: endpoint `fileobject`, version `v1`
- gRPC: service `nvidia.omniverse.storage.fileobject.v1.FileObjectService`

If a client needs a product-level classification that cannot be derived from the advertised API names, keep that mapping in client configuration. It cannot be reconstructed reliably from the v2 response.

## 6. Roll out safely

1. Add a v2 response parser alongside the existing v1 parser.
2. In a test environment, compare the APIs selected from v2 with the APIs the client currently uses from v1.
3. Test services that expose REST only, gRPC only, both transports, and multiple API versions.
4. Deploy clients that prefer v2 after verifying `"schema-version": 2`; retain the v1 path only for the agreed compatibility period.
5. Remove v1 parsing and type-based selection after all clients have moved.

## Error handling and compatibility rules

- Reject or explicitly handle an unknown `schema-version`; do not deserialize it as v2 by default.
- Treat every transport block and every advertised version as optional.
- Do not manufacture a REST URL when `path` is missing.
- Do not derive a gRPC service name from a REST endpoint name.
- Avoid relying on the order of `services` or `grpc.endpoints`.
- If several services advertise the same capability, select one using a client-defined policy (for example, an allowlist or preferred service name).
