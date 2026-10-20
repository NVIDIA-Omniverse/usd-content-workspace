# Service examples

The Storage API allows data exposure from non-standard storage backends to services using the Storage API.
For instance, local NAS data or an on-premise database can be made available to clients via a bespoke Storage API
instance implementing the specified functions for that particular backend. 

## REST service instance example

This section demonstrates a simple REST service for the Storage API using Python and FastAPI.

### Implementing stat

First, install fastapi, pydantic, asyncio, and uvicorn:

    pip install fastapi pydantic uvicorn asyncio

Next, implement a service offering the stat endpoint, serving content of a local disk directory via the Storage API. We 
assume the directory to be served is stored in the global variable STATIC_DIR:

```{literalinclude} ../examples/python_service/rest_filesystem_service.py
:language: python
:start-after: start stat example
:end-before: end stat example
```
The code checks if the path exists, raises an HTTP 404 if not, gathers required metadata, and builds a JSON response.

Pydantic mapping is used for automatic serialization and type checking:

```{literalinclude} ../examples/python_service/rest_filesystem_service.py
:language: python
:start-after: start pydantic stat example
:end-before: end pydantic stat example
```

The resource identity is defined as the combination of the address (relative file path) and modification date, encoded
as a JSON data package and encoded using base64. This is a design choice to emphasize that the resource identity's 
format is storage service implementation specific and the client should make no assumption about its content.

```{literalinclude} ../examples/python_service/service_utils.py
:language: python
:start-after: start encode identity
:end-before: end encode identity
```

Running the service example is done with uvicorn:

```{literalinclude} ../examples/python_service/grpc_filesystem_service.py
:language: python
:start-after: start example launch
:end-before: end example launch
```

Test the service with `curl`, here we have placed a file `hello.txt` in the `STATIC_DIR` being served:

    > curl --head localhost:8011/object-store/by-address/hello.txt
    HTTP/1.1 204 No Content
    date: Fri, 28 Jun 2024 13:17:37 GMT
    server: uvicorn
    x-nvidia-omniverse-storage-metadata: {"resource_address":"hello.txt","resource_identity":"eyJwIjogImhlbGxvLnR4dCIsIC
    J0IjogIjIwMjQtMDYtMjhUMTM6MTI6MzUuNzY3MTM4KzAwOjAwIn0=",
    "metadata":{"data_object_size":12,"last_modified_timestamp":"2024-06-28T13:12:35.767138+00:00"}}
    content-type: application/json

### Direct download implementation

The read operation streams the file back via the HTTP response body:

```{literalinclude} ../examples/python_service/rest_filesystem_service.py
:language: python
:start-after: start read example
:end-before: end read example
```

Use the previous stat operation result to download the file content from the Storage API service:

    curl --get localhost:8011/object-store/by-identity/eyJwIjogImhlbGxvLnR4dCIsICJ0IjogIjIwMjQtMDYtMjhUMTM6MTI6MzUuNzY3MTM4KzAwOjAwIn0= -v
    *   Trying [::1]:8011...
    *   Trying 127.0.0.1:8011...
    * Connected to localhost (127.0.0.1) port 8011
    > GET /object-store/by-identity/eyJwIjogImhlbGxvLnR4dCIsICJ0IjogIjIwMjQtMDYtMjhUMTM6MTI6MzUuNzY3MTM4KzAwOjAwIn0= HTTP/1.1
    > Host: localhost:8011
    > User-Agent: curl/8.4.0
    > Accept: */*
    >
    < HTTP/1.1 200 OK
    < date: Fri, 28 Jun 2024 13:32:37 GMT
    < server: uvicorn
    < content-type: application/octet-stream
    < transfer-encoding: chunked
    <
    Hello from your demo server, this is the content of the file hello.txt!
    * Connection #0 to host localhost left intact


## gRPC service example

For the gRPC service, generate Python language bindings from the proto files. The client example section contains
detailed information on that.

### gRPC Stat 

The gRPC service is implemented by subclassing the base class and implementing the methods:

```{literalinclude} ../examples/python_service/grpc_filesystem_service.py
:language: python
:start-after: start stat example
:end-before: end stat example
```

The server main loop is:

```{literalinclude} ../examples/python_service/grpc_filesystem_service.py
:language: python
:start-after: start example launch
:end-before: end example launch
```

Run the server to get a working gRPC server in plaintext mode, queryable via `grpcurl` for example:

    grpcurl -plaintext -d "{\"resource_address\": \"hello.txt\"}" -import-path c:\src\test_impl\storage-protos\proto\nvidia\omniverse\storage\fileobject\v1beta -import-path c:\src\test_impl\storage-protos\proto -proto fileobject_service.proto localhost:50051 nvidia.omniverse.storage.fileobject.v1beta.FileObjectService.Stat

The expected reply is:

```json
{
  "resourceInfo": {
    "resourceIdentity": {
      "encodedIdentity": "eyJwIjogImhlbGxvLnR4dCIsICJ0IjogIjIwMjQtMDYtMjhUMTM6MTI6MzUuNzY3MTM4KzAwOjAwIn0="
    },
    "metadata": {
      "dataObjectSize": "12",
      "lastModifiedTimestamp": "2024-06-28T13:12:35.767138242Z"
    }
  }
}
```


### gRPC reflection

Enable reflection in the gRPC server to simplify the grpcurl command. Install the package:

    pip install grpcio-reflection

Modify the gRPC server launch sequence to include the reflection extension:

```{literalinclude} ../examples/python_service/grpc_filesystem_service.py
:language: python
:start-after: start example launch-with-reflection
:end-before: end example launch-with-reflection
```

Relaunch the server, query the enabled services:

    > grpcurl -plaintext localhost:50051 list
    grpc.reflection.v1beta.ServerReflection
    nvidia.omniverse.storage.fileobject.v1beta.FileObjectService

Get more information on the individual functions:

    > grpcurl -plaintext localhost:50051 describe nvidia.omniverse.storage.fileobject.v1beta.FileObjectService
    nvidia.omniverse.storage.fileobject.v1beta.FileObjectService is a service:
    service FileObjectService {
      rpc Stat ( .nvidia.omniverse.storage.fileobject.v1beta.StatRequest ) returns ( .nvidia.omniverse.storage.fileobject.v1beta.StatResponse );
    }

Call the Stat function:

    > grpcurl -plaintext -d "{\"resource_address\": \"hello.txt\"}" localhost:50051 nvidia.omniverse.storage.fileobject.v1beta.FileObjectService.Stat
