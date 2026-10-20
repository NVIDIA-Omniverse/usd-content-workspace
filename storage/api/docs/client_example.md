# Client examples

Python is an excellent choice for writing a client that consumes the Storage API. This document provides examples for
both REST and gRPC APIs.

## Discovery

The Storage API endpoint is not auto-discovered. To connect, you need the Storage API endpoint address.
An example REST endpoint on localhost could be http://127.0.0.1:8011/fileobject, and a valid gRPC connection string could be 
`127.0.0.1:50051`.

## REST

### Client library selection

The REST interface is a pure HTTP interface. Any library supporting basic web operations will work. We’ll use the
Python `requests` module for this text, a popular library with [comprehensive documentation](https://requests.readthedocs.io/en/latest/).

It can be installed for example with pip:

    pip install requests

### Initial call

Here’s the minimal code required for a successful roundtrip to the Storage API:

```{literalinclude} ../examples/python_client/rest_simple.py
:language: python
:start-after: start example
:end-before: end example
```

This code issues a single HTTP HEAD command on the calculated address. A successful stat returns the HTTP code 204,
indicating success without body. The metadata is provided as a JSON struct within the
`x-nvidia-omniverse-storage-metadata` response header, which is parsed and printed to stdout.

### Data download

The Storage API service determines how data is downloaded. The simplest method is to stream the data using the HTTP 
response’s content body. A simple client can request the download method via a query parameter, and a well-behaved storage
service will respect the preference given if the performance envelop permits it.

Here’s a working download loop:

```{literalinclude} ../examples/python_client/rest_download.py
:language: python
:start-after: start body example
:end-before: end body example
```

This method is suitable for small files and simple clients. However, in a highly scalable environment, the Storage API 
service should not bear the load of all data downloads. In such cases, the service will likely return a redirect
message with instructions for downloading data from the origin server.

Here’s how to implement this:

```{literalinclude} ../examples/python_client/rest_download.py
:language: python
:start-after: start redirect example
:end-before: end redirect example
```

Combining these elements, we get a complete flexible download client:

```{literalinclude} ../examples/python_client/rest_download.py
:language: python
:start-after: start complete download example
:end-before: end complete download example
```

Note it is the client's responsibility to optimize the redirected HTTP download, e.g. using proper multipart operations.

### Data upload

HTTP provides some built-in large file transmission control mechanisms, so the mainstream HTTP client libraries won't
require additional actions to perform an upload.

To write a file object, call the `PUT /fileobject/by-address/<address>` endpoint providing the contents of the file
object as the payload and specifying the length of the file object in the `data_object_size` query parameter. You can
optionally provide an `upload_preference` query parameter, specifying the preferred content upload method, just like 
on download. Upon processing the request, the service will respond with either HTTP 201 or HTTP 300
status code, identifying either a successful write or the following write instruction respectively:

```{literalinclude} ../examples/python_client/rest_upload.py
:language: python
:start-after: start example
:end-before: end example
```

HTTP 300 should be handled manually, namely by introspecting the contents of the response body. If the `redirect`
response property is present and is not null, a redirect write must be performed:

```{literalinclude} ../examples/python_client/rest_upload.py
:language: python
:start-after: start redirect write example
:end-before: end redirect write example
```

Otherwise, if the response contains a non-null `multipart` response property, the contents of the file object must be
cut into several chunks, not exceeding the multipart upload limits and uploaded as follows:

```{literalinclude} ../examples/python_client/rest_upload.py
:language: python
:start-after: start multipart write example
:end-before: end multipart write example
```

#### Upload preference selection

A client may prepare itself in advance to a certain write workflow, deducing the upload preference from the response
of the `GET /fileobject/by-address/<address>/upload-options` endpoint, based on the size of the written file object:

```{literalinclude} ../examples/python_client/rest_upload.py
:language: python
:start-after: start upload preference example
:end-before: end upload preference example
```

In such a case, when a matching `upload_preference` is passed in the query parameters of the following
`PUT /fileobject/by-address/<address>` request, it's guaranteed that the service will select this exact upload method.

### Listing folders

REST interface exposes a token-based pagination "list" and "list with stat" endpoints allowing to list the contents of
folders in one or more chunks:

```{literalinclude} ../examples/python_client/rest_list.py
:language: python
:start-after: start example
:end-before: end example
```

### Deleting file
To delete a file object, call the `DELETE /fileobject/by-address/<address>` endpoint. The endpoint always returns a 204 status code, regardless of whether the object exists.

```{literalinclude} ../examples/python_client/rest_delete.py
:language: python
:start-after: start example
:end-before: end example
```

## GRPC

### Client library selection

Like REST, multiple different client libraries for Python are available to implement the gRPC client code.
We’ll use the official `grpcio` library for our examples. Install it with

    pip install grpcio grpcio-tools

The `grpcio-tools` package includes the protobuf compiler needed to generate the service stub files.


### Python language bindings generation

Assuming the storage proto files are in a `proto` subdirectory, create the language bindings in the current directory 
with the following command:

    python -m grpc_tools.protoc --python_out=. --pyi_out=. --grpc_python_out=. --proto_path=proto proto\nvidia\omniverse\storage\fileobject\v1beta\fileobject.proto proto\nvidia\omniverse\storage\fileobject\v1beta\fileobject_service.proto proto\nvidia\omniverse\storage\capabilities\v1beta\capabilities.proto proto\nvidia\omniverse\storage\filefolder\v1beta\filefolder_service.proto

This command creates the required Python files in the `nvidia` subdirectory, from which they can be imported. Make sure to adjust
the paths and number of files matching the version of the storage API definitions, new files might be added and paths might change.


### Simple stat

With the generated files, calling stat is even simpler than the REST version:

```{literalinclude} ../examples/python_client/grpc_simple.py
:language: python
:start-after: start example
:end-before: end example
```

A connection to the gRPC server is established once via a channel context manager. The created service stub connecting
via that channel can be reused for multiple calls.

### Data download

Downloading data from the Storage API is straightforward. We just need to loop over the replies:

```{literalinclude} ../examples/python_client/grpc_download.py
:language: python
:start-after: start example
:end-before: end example
```

In this case, we handle the download by iterating over the response stream from the `ReadFromAddress` method. This method
returns first the metadata and then either a stream of chunks, which we write into the destination file as they come in,
or responds with redirection data similar to the REST redirect described above.

### Data upload

In contrast to HTTP, gRPC requires a more explicit transmission management, to avoid traffic congestion and ensure
optional performance of a client.

Write must be initiated by sending a `WriteRequest`, including at least the `resource_address` of the written file object and
its size specified in the `data_object_size` field. A client may optionally provide an `upload_preference` parameter
identifying a preferred file object upload method as before. 

Upon receiving a `WriteRequest`, the service will respond with a control message, which can be either a `WriteChunksAccepted`
message, identifying the willingness of the service to accept the file object contents in one or more following `Chunk`
messages, or a `WriteRedirect` message, which contains an instruction to follow either an indirect or a multipart write flow.

On a successful write, the service will respond with a `ResourceInfo` message containing the metadata of the newly created
data object:

```{literalinclude} ../examples/python_client/grpc_upload.py
:language: python
:start-after: start example
:end-before: end example
```

Redirect and multipart write routines are similar to those of the HTTP protocol.

```{literalinclude} ../examples/python_client/grpc_upload.py
:language: python
:start-after: start redirect write example
:end-before: end redirect write example
```

```{literalinclude} ../examples/python_client/grpc_upload.py
:language: python
:start-after: start multipart write example
:end-before: end multipart write example
```

#### Opportunistic writes

In some cases, clients can use "opportunistic writes", that is such write operations, where `Chunk` messages
are streamed without awaiting for the `WriteChunksAccepted` response from the service. While this can be beneficial
performance-wise for small uploads, for any "substantially large" file objects, clients are advised to call the
`FetchWriteTypeInfo` in advance to familiarize themselves with the method upload limits, as otherwise a rejected
sequence of `Chunk` messages may cause traffic congestion.

#### Upload preference selection

In a similar way to the REST API, gRPC API provides means of deducing the upload method selected by the storage
service, namely by introspecting the response of the `FetchWriteTypeInfo` method call:

```{literalinclude} ../examples/python_client/grpc_upload.py
:language: python
:start-after: start upload preference example
:end-before: end upload preference example
```

### Listing folders

To perform file and folder listings, use the `List` operation provided by the `FileFolderService` API, which
returns a stream of file and folder entries. For clients requiring additional metadata for file entries, the
`ListStat` operation should be employed as follows:

```{literalinclude} ../examples/python_client/grpc_list.py
:language: python
:start-after: start example
:end-before: end example
```

Depending on the storage backend's capabilities, ListStat might be significantly slower than List or there might be no 
performance difference at all. 

### Deleting file
To delete a file object, call the `Delete` operation provided by the `FileFolderService` API. This operation is idempotent, meaning it returns an `OK` response even if the specified file object does not exist. Additionally, it does not return any content in the response.

```{literalinclude} ../examples/python_client/grpc_delete.py
:language: python
:start-after: start example
:end-before: end example
```
