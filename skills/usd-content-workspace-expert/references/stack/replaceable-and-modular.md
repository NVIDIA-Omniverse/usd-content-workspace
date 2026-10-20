# Reference implementations, replaceable services, modular deployments

> **Applies to:** every service in this repository · **Released:** Developer Guide 1.3.0 inventory · **This repo's snapshot:** see [../versions/source-snapshot.md](../versions/source-snapshot.md) · **Sources:** the [source skill package](../../NOTICE.md); this repo's `api/`, `reference/`, and `conformance/` layout ([`README.md`](../../../../README.md))

Bring this up with every developer and architect, early: **the services in this repository are reference implementations.** They are meant to be extended or replaced so the stack fits tightly into the organization's own infrastructure. The durable part is the API contracts; any service that implements a contract can take the reference service's place.

## The contract is the product, the service is a reference

Each area separates three things:

| Directory | What it is | Can you replace it? |
| --- | --- | --- |
| `<area>/api/` (or the protos and OpenAPI inside a reference service) | The versioned contract clients are built against | No — clients depend on it. Implement it. |
| `<area>/reference/` | NVIDIA's implementation of the contract | Yes. Deploy as is, extend, or swap for your own. |
| `<area>/conformance/` and suites inside components | Tests that check an implementation against the contract | Use them to prove your replacement. |

Examples of tight integration:

- **Storage:** serve content from your own object store, PLM or PDM system, or file servers by implementing the Storage API ([../develop/custom-storage-adapter.md](../develop/custom-storage-adapter.md)).
- **Permissions:** answer authorization from your existing entitlement system instead of Cedar policies ([../develop/custom-permissions-adapter.md](../develop/custom-permissions-adapter.md)).
- **User identity:** resolve users and groups from your directory instead of Microsoft Graph (implement the User Info API; [../apis/userinfo.md](../apis/userinfo.md)).
- **Notifications:** publish and deliver events through your own message bus ([../develop/custom-notifications-adapter.md](../develop/custom-notifications-adapter.md)).
- **Thumbnails:** generate previews with your own renderer ([../develop/custom-thumbnail-implementation.md](../develop/custom-thumbnail-implementation.md)).
- **Everything at once:** a single local executable can implement Discovery, Storage, and Metadata ([../deploy/local-process.md](../deploy/local-process.md)).

## Why replacing one service is easy

Clients never hard-code a service. They read the Discovery catalog and bind to the API versions advertised there. A replacement that:

1. implements the same API wire versions (for example `nvidia.omniverse.storage.fileobject.v1beta`),
2. advertises them through the Discovery capabilities service (or appears in a static catalog), with the Kubernetes Service annotation `discovery.omniverse.nvidia.com/enabled` when using Auto Discovery,
3. accepts the same tokens behind the same ingress and authentication, and
4. passes the area's conformance suite,

is a drop-in change for Kit, the Client Library, Navigator, and the other services. Swap one Helm release (or process) for another; nothing else in the deployment has to change. How to do each step: [../develop/add-your-service.md](../develop/add-your-service.md).

Things to check when replacing a service:

- Other services may call it. Storage calls Permissions and publishes to Event Aggregation; Thumbnails reads Storage and storage events; the Permission UI and Navigator look up names through User Info. Your replacement must serve the APIs those callers use, at the versions they were built against ([../versions/api-contracts.md](../versions/api-contracts.md)).
- Behavior the contract leaves open (versioning semantics, metadata durability, ordering of events) is yours to define; document it for clients.
- You own the replacement's security, scaling, and upgrades. The checklists in [security.md](security.md) and [../operate/](../operate/) still apply.

## Start small, grow as the workflow needs

Deployments are modular. Every deployment needs Discovery; everything else is added when a workflow needs it.

| Stage | Services | Gives users |
| --- | --- | --- |
| 1. Minimum | Discovery + Storage (Metadata comes with the Storage Service) | Browse, open, save, versions, metadata from Kit and Navigator |
| 2. Secure | + Envoy auth extension (or your gateway) with your IdP | Signed-in access |
| 3. Govern | + Permission Service, User Info, Permission UI | Per-path access control, names and groups |
| 4. React | + RabbitMQ, Event Aggregation, Event Consumer | Change notifications for apps, pipelines, caches |
| 5. Enrich | + Thumbnails, Live Edit, caches, packaging | Previews, live collaboration, faster loads, versioned packages |

Stage 1 can run as local processes ([../deploy/local-process.md](../deploy/local-process.md)), on a local cluster ([../deploy/local-development.md](../deploy/local-development.md)), or in production ([../deploy/production-stack.md](../deploy/production-stack.md)). Each later stage is one or more independent Helm releases; adding one does not require reinstalling the others, although some need a setting changed in an existing service (for example enabling permissions or event publishing in Storage). Order and dependencies: [choosing.md](choosing.md).

## How to say it

When a developer asks how to "install the stack", lead with: start with Discovery and Storage, decide which reference services you will keep and which you will replace with your own, and add services only as your workflow requires them.
