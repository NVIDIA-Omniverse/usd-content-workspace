# notification-protos — API contract map

> **Applies to:** Notifications publisher and consumer contracts · **Released:** NGC resources `nvidia/omniverse/notifications-aggregation-api:1.0.0-beta.3` and `notifications-consumer-api:1.0.0-beta.3` (released inventory; the previous published inventory had `1.0.0-beta.2`); HTML reference [notification-api/1.0.0-beta.2](https://docs.omniverse.nvidia.com/ovstorage/notification-api/1.0.0-beta.2/index.html) has its own version · **This repo's snapshot:** proto packages `v1beta`; Buf module `pb.api.nvidia.com/omniverse/notifications`; no version file — the latest release commit is "Release notification-protos 0.0.0-manual.38b13644", which changed only the Buf dependency (`buf.yaml`, `buf.lock`), not the `.proto` files · **Sources:** [`notifications/api/`](../../../../notifications/api/README.md), the [source skill package](../../NOTICE.md) upstream `notification-protos` skill

The `.proto` files define the contract; the guides explain how to use it. Read the proto before generating code.

## What it is

The published gRPC contract for both notification services. No service ships from this directory.

| Side | Proto package | File | Served by |
|---|---|---|---|
| Publish | `nvidia.omniverse.notifications.publisher.v1beta` | [`protos/nvidia/omniverse/notifications/publisher/v1beta/event_publisher.proto`](../../../../notifications/api/protos/nvidia/omniverse/notifications/publisher/v1beta/event_publisher.proto) | [Event Aggregation Service](event-aggregation-service.md) |
| Consume | `nvidia.omniverse.notifications.consumer.v1beta` | [`protos/nvidia/omniverse/notifications/consumer/v1beta/event_consumer.proto`](../../../../notifications/api/protos/nvidia/omniverse/notifications/consumer/v1beta/event_consumer.proto) | [Event Consumer Service](event-consumer-service.md) |

`protos/` is a Buf v2 module ([`buf.yaml`](../../../../notifications/api/protos/buf.yaml), `buf.lock`) that depends on `buf.build/protocolbuffers/wellknowntypes` (for `Struct` and `Timestamp`) on the public Buf Schema Registry, locked in `buf.lock` to commit `7727a3b7399540398e5736b9916f0faa` (earlier imports named an NVIDIA-hosted copy of that dependency; the content digest is unchanged), lints with `STANDARD` + `COMMENTS`, and checks breaking changes at `FILE` level. Go and Java package options are set in both files.

The REST equivalents are OpenAPI 3.1 documents that live with each service, not here: [aggregation `openapi.yaml`](../../../../notifications/reference/event-aggregation-service/openapi.yaml) and [consumer `openapi.yaml`](../../../../notifications/reference/event-consumer-service/openapi.yaml), both `info.version: v1beta`. In this snapshot both documents carry their NVIDIA API-governance ID as the `info.x-nspect.id` extension and have no top-level `tags` list. Both `CHANGELOG.md` files say this replaces a top-level `nspect_id` tag; this snapshot does not record which form the released 1.6.0 / 1.11.0 images serve at `/openapi.json`. Neither form affects requests or responses.

Pre-generated Python stubs are checked in under each service (`nvidia/omniverse/notifications/...`) and each conformance suite (`conformance_tests/src/nvidia/...`). They are generated with `grpcio-tools==1.76.0` so they import under the smoke-test image's grpcio; regenerate from the proto rather than copying them into another project.

## Version mapping

| Contract feature | First service version | In beta.2 | In beta.3 (released) | In this snapshot |
|---|---|---|---|---|
| `ConsumeDurableEventsWithAcks`, `ManageDeadLetteredEvents`, `ConsumeDurableEvents` deprecated | Consumer 1.10.0 | per the source package | yes | yes |
| `CreateDurableQueueRequest.queue_id`, `CreateDurableQueueResponse.created`, `nack_delivery_tags` | Consumer 1.11.0 | no | yes (per the source package, which inspected the archive) | yes |
| `ConsumeNonDurableEventsResponse.supports_filter_update_acks`, `filter_update_ack`, `FilterUpdateAck` | Consumer 1.11.0 | no | unverified in this snapshot | yes |
| Publisher limits documented on fields (1000 events, 65536-byte message, 256/2048 characters) | Aggregation 1.7.0 (enforcement) | — | comments unverified | yes |

Whether the beta.3 archives are byte-identical to this snapshot's protos is unverified. Generate clients from the definitions that match the service version you deploy; a newer field sent to an older service is ignored or rejected.

## Route by task

| Question | Go to |
|---|---|
| Field-level reference, error semantics, examples | [API reference in this tree](../apis/notifications.md) |
| Concepts and first integration | [`docs/index.md`](../../../../notifications/reference/event-consumer-service/docs/index.md), [`docs/getting-started.md`](../../../../notifications/reference/event-consumer-service/docs/getting-started.md) |
| Sending events | [`docs/publishing-events.md`](../../../../notifications/reference/event-consumer-service/docs/publishing-events.md) |
| Receiving, subscriptions, filtering, DLQ | [`docs/consuming-events.md`](../../../../notifications/reference/event-consumer-service/docs/consuming-events.md) |
| RPC and message details | [`docs/api-reference.md`](../../../../notifications/reference/event-consumer-service/docs/api-reference.md) |
| Who may publish or subscribe | [`docs/permissions.md`](../../../../notifications/reference/event-consumer-service/docs/permissions.md) |
| Building clients or a compatible service | [develop](../develop/custom-notifications-adapter/README.md) |

The upstream skill routed to `docs/docs/*.md` inside the protos repository; in this repository those guides live under `notifications/reference/event-consumer-service/docs/`. Other components carry generated copies of these protos (for example the permission service and storage service Rust crates); treat `notifications/api/` as the source.
