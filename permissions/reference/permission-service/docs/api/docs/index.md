(permission-api-index-omniverse-storage-permission-api)=
# Omniverse Storage Permission API

(permission-api-index-introduction)=
## Introduction

This document outlines the architecture of the Permission Service that allows Omniverse APIs to integrate authentication
and authorization with the cloud infrastructure used by Omniverse customers.

The following sections describe the Permission API specification for gRPC and REST and how Omniverse services need to
integrate with the Permission Service.

```{toctree}
:maxdepth: 1
:caption: Contents:
authorization-for-a-user-principal
authorization-for-a-service-principal
rest-api
grpc
references
```

(permission-api-index-assumptions-constraints-dependencies)=
## Assumptions, Constraints, Dependencies

* OpenID Connect is used by the Permission API to implement authentication by default. The API implementation may use
  another authentication mechanism and change this if needed.
* To reduce the amount of calls made to the Permission Service, clients are encouraged to cache authorization results for
  a limited amount of time. When the Permission Service returns a `Cache-Control` header, it informs clients how long an
  authorization decision may be cached.

(permission-api-index-definitions-acronyms-abbreviations)=
## Definitions, Acronyms, Abbreviations

| Term  | Definition                           |
|-------|--------------------------------------|
| AuthN | Authentication                       |
| AuthZ | Authorization                        |
| ACL   | Access Control List                  |
| IdP   | Identity Provider                    |
| RBAC  | Role-Based Access Control            |
| ABAC  | Attribute-Based Access Control       |
| ReBAC | Relationship-Based Access Control    |
| PARC  | Principal, Action, Resource, Context |

(permission-api-index-architecture-details)=
## Architecture Details

The Permission Service provides the API for reading information from the underlying authorization system managed by the
customer.
The service supports any authorization model (RBAC, ABAC, ReBAC, ACL, etc.) that allows to check if a Principal (user or
service) can perform the specified Action for a Resource with the provided Context (abbreviated as PARC).

For example:

- Can user _Alice_ [PRINCIPAL] _generate a thumbnail_ [ACTION] for _/Images/Picture.jpg image_ [RESOURCE]?
- Can user _Bob_ [PRINCIPAL] _upload_ [ACTION] _a 1GB file_ [RESOURCE] to _/Projects/ folder_ [RESOURCE]?
- Can _USD Search Service_ [PRINCIPAL] _read tags_ [ACTION] for _/Projects/Scene.usd file_ [RESOURCE]?

Applications and services need to provide principal, action, resource and context for evaluating permissions:

* The principal must contain a payload from an ID token received from the Identity Provider for the current user or the
  service interacting with the application;
* The action is a name of the request made by the principal to the application. The action payload must contain the
  action name and the service name performing the action;
* The resource contains the detailed information about the object that the principal tries to operate on, e.g. a record
  from the application database;
* The context is optional and may provide additional information for the authorization request like the caller IP
  address and geolocation.

The Permission Service does not provide APIs for configuring/changing authorization rules – access control is managed by
the underlying authorization system provided by customers.

Therefore, the Permission Service implements two primary use cases for AuthN/AuthZ:

1. Authorization for a user principal – applications and services verify that a user can perform an action in the
   system;
2. Authorization for a service principal – services verify that another service can perform an action in the system;

(permission-api-index-see-next)=
### See next

* {ref}`Authorization for a user principal <permission-api-authorization-for-a-user-principal-authorization-for-a-user-principal>`
* {ref}`Authorization for a service principal <permission-api-authorization-for-a-service-principal-authorization-for-a-service-principal>`
* {ref}`REST API <permission-api-rest-api-rest-api>`
* {ref}`gRPC specification <permission-api-grpc-grpc-specification>`
* {ref}`References <permission-api-references-references>`
