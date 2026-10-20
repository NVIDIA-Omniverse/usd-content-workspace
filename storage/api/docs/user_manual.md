# Storage API Introduction

The Storage API is designed to empower developers by providing a unified, flexible interface (gRPC and REST/OpenAPI specs) that seamlessly connects Omniverse, or other applications, to virtually any backend storage solution. You have the freedom to build storage integrations tailored to your infrastructure, leveraging the unique features and capabilities of your chosen storage systems. This is designed to be a service within a microservice architecture, but is also capable of being run on a local system.

Below we will go through some specific terminology and capabilities of the Storage APIs, so you can implement your own that best suits your needs.

## Understanding Resource Addressing and Identity

This user manual introduces the fundamental concepts of Resource Address and Resource Identity within the Storage API. These two concepts are crucial for understanding how data is located, accessed, and managed within the storage system.

### 1\. Resource Address: Locating Data

A **Resource Address** is a string that acts as a locator for objects within a storage system. Think of it like a traditional file path or URL that points to a specific location.

**Key characteristics and roles of a Resource Address:**

* **Storage Specific Semantics:** The format and meaning of a Resource Address can vary depending on the underlying storage backend. For example, it might look like an S3 bucket URL (e.g. `s3://bucket/path/to/file.ext`) or a custom database identifier (e.g. `my_custom_database://17A56A6F8734E`).  
* **Identifies Location:** Its primary role is to specify *where* a data object is and can be read from, or should be stored by a write operation. Specific storage systems might introduce query-based addresses which can be used to find a data object given a version identifier (e.g. `s3://bucket/path/to/file.ext?etag=a64be1231acd`) or query criteria (e.g. `my_custom_database://17A56A6F8734E?approval_status=prod`). Not all addresses might point to a writable location.  
* **Potentially Mutable Content:** The content at a given Resource Address is not considered immutable. A write operation to the same Resource Address can change the data associated with that address. This means that reading from the same address at different times might yield different data if modifications have occurred.  
* **Hierarchical Organization:** For storage systems that support a file-system-like structure, Resource Addresses follow RFC3986 (Uniform Resource Identifier Syntax), allowing clients to construct new addresses from existing ones by appending relative paths. This enables navigation and referencing within hierarchical storage.  
* **Routable:** To allow the deployment of multiple storage services within the same application, it is recommended that a storage service be able to identify its own Resource Addresses. This enables a client working with multiple storage services to determine where to send a particular address.  
* **Role in API Functions:**  
  * **Enumerate:** Used to list the contents of a storage location (e.g., a folder or just a common prefix) by providing its Resource Address.  
  * **Stat:** Given a Resource Address, this function returns information about the data object currently stored at that location, including its Resource Identity.  
  * **ReadFromAddress:** This function allows you to read the binary data of the object currently located at a specific Resource Address. This is useful when the most up-to-date content at a given location is desired.  
  * **Write:** When you want to store new data, you provide a Resource Address to specify where the data should be written.   
  * **Delete:** Used to remove a data object from a specific Resource Address, including previous versions if the storage system is versioned.  
  * **Copy:** The destination of a copy operation is specified using a Resource Address.

### 2\. Resource Identity: Identifying Specific Data Object Instances

A **Resource Identity** is a string that specifically identifies **a particular version or instance of a data object**. Unlike a Resource Address, which points to a location, a Resource Identity points to the exact content.

**Key characteristics and roles of a Resource Identity:**

* **Opaque and Storage Service Specific:** The format of a Resource Identity is not standardized; it is determined by the specific storage service implementation. Clients should treat it as an opaque string.  
* **Immutable Content Guarantee:** The most crucial guarantee of a Resource Identity is that it will always retrieve the *same content* every time the `Read` operation is called (provided the data is still available). It is directly linked to a specific version of data.  
* **Not a Unique Key:** While it identifies specific content, different Resource Identity values might, in some very rare cases, resolve to the same underlying object. Therefore, it should not be used as a globally unique key or for content-based hashing for equality comparison. It is suitable for use as a cache key as it should change infrequently for the same data.  
* **Durable:** Resource Identities are designed to be durable and survive the redeployment of storage services. Their lifetime is tied to the data they reference, not to the service itself.  
* **Shareable:** Resource Identities can be shared between different users, allowing them to access the same specific content, provided they have the necessary access permissions.  
* **Routable:** To allow deployment of multiple storage services within the same application, it is recommended to make sure a storage service can identify its own Resource Identifiers, so a client working with multiple storage services can determine where to send that identifier to.  
* **Role in API Functions:**  
  * **Stat:** Returns the Resource Identity of the data object currently located at a given Resource Address.  
  * **EnumerateVersions:** Returns a list of Resource Identities for all available versions of a data object at a specific Resource Address.  
  * **Write:** Upon successful completion of a write operation, the API returns a Resource Identity for the data that was just written. This allows clients to maintain a reference to the specific version of data they just stored.  
  * **Read:** Used to retrieve the binary data of a specific, immutable data object instance by providing its Resource Identity.  
  * **RestoreVersion:** Takes a Resource Identity to specify an old version of a data object that should be restored to become the current version at a given Resource Address.  
  * **Copy:** The source of a copy operation is specified using a Resource Identity, ensuring that a specific version of data is copied.

Summary of Roles and Guarantees:

| Concept | Role | Key Guarantees | Used in API Functions |
| ----- | ----- | ----- | ----- |
| **Resource Address** | Locates data within the storage. Points to a mutable storage location. | Semantics are storage-specific. Content at the address can change over time. | Enumerate, Stat, ReadFromAddress, Write, Delete, Copy |
| **Resource Identity** | Identifies a specific, immutable instance of a data object. | Always retrieves the same content (if available). Durable and shareable. Opaque format. | Stat, EnumerateVersions, Read, Write (as return value), RestoreVersion, Copy (as source) |

By understanding these core concepts, users can effectively interact with the Storage API to manage and access their data.