# Version 1 Response Schema

### Deprecated v1 Response

A version 1 response will contain JSON content with the following attributes:
At the root of the response is an Object that contains two fields, “schema-version” which contains an Integer value, and a “services” entry which holds a list of services (specified below)..

Each service object has the following properties:
- “id” - a unique name for the service (e.g. slug)
- “name” - a display-friendly name of the service
- “type” - a single value of an enumeration of service types
- “rest” - an optional endpoint to a RESTful service
- “grpc” - an optional endpoint to a gRPC service

The “type” enumeration should be one of the following values:
- “thumbnail” - for a thumbnail generation service
- “search” - for a search service instance
- “tagging” - for a file tagging service instance
- “permission” - for a permission checking service instance
- “storage” - for a Storage API service instance


```json
{
  "services": [
    {
      "id": "thumbnail-service-01",
      "name": "Thumbnail Service",
      "type": "thumbnail",
      "rest": "https://host:port/path/to/service",
      "grpc": "grpc://host:port/path/to/service"
    },
    {
      "id": "tagging-service-01",
      "name": "Tagging Service",
      "type": "tagging",
      "rest": "https://host:port/path/to/service",
      "grpc": "grpc://host:port/path/to/service"
    },
    {
      "id": "search-service-01",
      "name": "Search Service",
      "type": "search",
      "rest": "https://host:port/path/to/service",
      "grpc": "grpc://host:port/path/to/service"
    },
    {
      "id": "permission-service-01",
      "name": "Permission Service",
      "type": "permission",
      "rest": "https://host:port/path/to/service",
      "grpc": "grpc://host:port/path/to/service"
    },
    {
      "id": "storage-service-01",
      "name": "Storage API 1",
      "type": "storage",
      "rest": "https://host:port/path/to/service",
      "grpc": "grpc://host:port/path/to/service"
    },
    {
      "id": "storage-service-02",
      "name": "Storage API 2",
      "type": "storage",
      "rest": "https://host:port/path/to/service",
      "grpc": "grpc://host:port/path/to/service"
    }
  ]
}
```