# Component maps

Each page maps one component to its docs, Helm chart, API contracts, changelog, and tests in this repository, and states the released and snapshot versions it applies to. Read [../versions/README.md](../versions/README.md) before quoting a version from any of them.

| Area | Component | Map |
| --- | --- | --- |
| Storage | Storage API (contracts, conformance suite, Python filesystem reference) | [storage-api.md](storage-api.md) |
| Storage | Storage Core (Rust library for building storage services) | [storage-core.md](storage-core.md) |
| Storage | NVIDIA Storage Service | [storage-service.md](storage-service.md) |
| Metadata | Metadata API | [metadata-api.md](metadata-api.md) |
| Discovery | Auto Discovery operator and legacy Discovery chart | [discovery.md](discovery.md) |
| Permissions and identity | Permission Service | [permission-service.md](permission-service.md) |
| Permissions and identity | Envoy auth extension | [envoy-auth-extension.md](envoy-auth-extension.md) |
| Permissions and identity | Permission UI and Storage Permission UI | [permission-ui.md](permission-ui.md) |
| Permissions and identity | Microsoft Graph User Info Service | [userinfo-service.md](userinfo-service.md) |
| Notifications | notification-protos (API contracts) | [notification-protos.md](notification-protos.md) |
| Notifications | notification-common | [notification-common.md](notification-common.md) |
| Notifications | Event Aggregation Service | [event-aggregation-service.md](event-aggregation-service.md) |
| Notifications | Event Consumer Service | [event-consumer-service.md](event-consumer-service.md) |
| Thumbnails | Thumbnail Service | [thumbnail-service.md](thumbnail-service.md) |
| Shared | Observability and URL normalizer (`common/`) | [common-libraries.md](common-libraries.md) |
| Clients | Storage Navigator | [navigator.md](navigator.md) |
| External | Omniverse Client Library (Kit integration) | [client-library.md](client-library.md) |
| External | Live Edit | [live-edit.md](live-edit.md) |

Every component is a reference implementation. To replace one with your own service, see [../stack/replaceable-and-modular.md](../stack/replaceable-and-modular.md).
