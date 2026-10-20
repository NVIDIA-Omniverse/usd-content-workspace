<!-- SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Choosing components

Start from what your workflow needs to do, and stand up only the services that serve it. Most workflows start with the Storage Service; the others are optional.

| Your workflow needs to | Use |
| --- | --- |
| Browse content on an existing deployment | The [Navigator UI](connect.md) |
| Embed content access in an application | [ovstorage](https://github.com/NVIDIA-Omniverse/ovstorage), whose `services-client` plugin reaches the Storage Service |
| Serve content from S3 or Azure Blob Storage over gRPC and REST | Storage Service |
| Let clients find the services and their API versions | Discovery operator |
| Enforce access policy | Permission Service, with the Envoy auth extension for bearer-token authorization |
| Look up users and groups | Microsoft Graph User Info Service |
| Read and write key/value metadata on content | Metadata API, implemented by the Storage Service |
| React to content changes | Event Aggregation Service (publish) and Event Consumer Service (consume), over RabbitMQ |
| Generate previews of stored content | Thumbnail Service and its conversion worker |

## Order to stand them up

1. **Storage Service** over your bucket or container. It implements the Storage and Metadata APIs.
2. **Discovery**, so clients resolve service endpoints from one discovery URL. The Storage Service advertises its capabilities through the Discovery `CapabilitiesService`.
3. **Permissions and user identity**, when access must be checked per user or group. The Storage Service calls the Permission Service for access checks.
4. **Notifications**, when consumers must react to changes. The Storage Service publishes change events through the publisher API.
5. **Thumbnails**, when you need previews. The Thumbnail Service reads from Storage gRPC and generates thumbnails from storage create events.

## Related repositories

Packaging, live editing, caching, and search are separate products. See [where usd-content-workspace fits](../overview/content_services.md).
