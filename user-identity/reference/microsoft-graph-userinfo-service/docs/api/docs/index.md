(userinfo-api-index-title)=
# Omniverse User Info Service

(userinfo-api-index-introduction)=
## Introduction

User Info Service is a centralized, read-only identity service for the NVIDIA Omniverse cloud platform. It provides a single point of access for retrieving user and group information from the organization's identity provider. Any service running on the platform — whether built by NVIDIA (such as Omniverse Storage API) or by the deploying organization — that needs to know "who is this user?" or "what groups do they belong to?" calls User Info Service instead of talking to the identity provider directly.

The service exposes the organization's user and group directory, eventually consistent with the identity provider, through both REST and gRPC APIs. It supports looking up users by identifier or email, searching users and groups by name (prefix and substring matching), resolving group memberships, listing the members of a group, and identifying the currently authenticated user from their access token.

The following sections describe the service use cases, authentication and authorization, the REST and gRPC API specifications, and the data synchronization model.

```{toctree}
:maxdepth: 1
:caption: Contents:
use-cases
directory-synchronization
authentication-and-authorization
rest-api
grpc
references
```

(userinfo-api-index-terms-and-abbreviations)=
## Terms and Abbreviations

| Term | Definition |
|---|---|
| ACL | Access Control List — a set of rules that grants or denies permissions on a resource to specific users or groups. |
| API | Application Programming Interface. |
| gRPC | gRPC Remote Procedure Call — a high-performance RPC framework using Protocol Buffers. |
| GUID | Globally Unique Identifier — a 128-bit identifier used by Microsoft Entra ID for user and group objects. |
| HTTP | Hypertext Transfer Protocol. |
| IdP | Identity Provider — a service that authenticates users and issues identity tokens (e.g. Microsoft Entra ID, Okta, Google Identity). |
| JSON | JavaScript Object Notation — a lightweight data interchange format. |
| JWT | JSON Web Token — a compact, URL-safe token format used for bearer authentication. |
| LDAP | Lightweight Directory Access Protocol — a traditional protocol for querying on-premises directories. |
| OIDC | OpenID Connect — an identity layer built on top of OAuth 2.0 for authenticating users. |
| REST | Representational State Transfer — an architectural style for web APIs using HTTP methods. |
| RFC | Request for Comments — a standards document published by the IETF. |
| RPC | Remote Procedure Call — a protocol for invoking a function on a remote service. |
| SCIM | System for Cross-domain Identity Management — a standard REST API for provisioning and querying users and groups (RFC 7644). |
| SDK | Software Development Kit. |
| UI | User Interface. |
| UPN | User Principal Name — a login identifier in the format `user@domain`, commonly used in Microsoft Entra ID. |
| USD | Universal Scene Description — an open framework for describing, composing, and reading 3D scenes. |

(userinfo-api-index-see-next)=
## See next

* {ref}`Use cases <userinfo-api-use-cases-title>`
