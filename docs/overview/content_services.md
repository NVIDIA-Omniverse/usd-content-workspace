<!-- SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Where usd-content-workspace fits in Omniverse Content Services

Omniverse Content Services is the broader stack for finding, accessing, packaging, collaborating on, and delivering content. **usd-content-workspace** supplies the service foundation; the related repositories supply complementary capabilities.

| Product repository | What it provides | How it fits with this workspace |
| --- | --- | --- |
| **usd-content-workspace** (this repository) | Service contracts, reference services, conformance suites, and sample clients. | Exposes content and supporting services over APIs on your infrastructure. |
| [**ovstorage**](https://github.com/NVIDIA-Omniverse/ovstorage) | A backend-neutral data-access library with storage plugins, language bindings, CLI, and MCP tools. | Applications reach this workspace's Storage Service through the `services-client` plugin, or use other backends directly. |
| [**ovpackage**](https://github.com/NVIDIA-Omniverse/ovpackage) | A CLI and Python API for immutable asset package versions, dependency metadata, content-hash catalogs, installation, and publishing. | Publishes and retrieves packages through its Storage API backend as well as local and cloud backends, so workflows can select and reproduce specific content versions. |
| [**ovlive**](https://github.com/NVIDIA-Omniverse/ovlive) | An OpenUSD live-edit runtime with C, C++, and Python interfaces for sharing edits to `.live` layers through a Live Services endpoint. | Adds collaboration to host applications. Live editing has its own runtime and service contract alongside the content services here. |
| [**usd-caches**](https://github.com/NVIDIA-Omniverse/usd-caches) | A local cache, a shared content cache, and a shared derived-data cache. | Reuses fetched content and generated data near applications and compute, reducing repeated transfers from backing storage. |
| [**usd-search**](https://github.com/NVIDIA-Omniverse/usd-search) | Asset search using text or images, metadata filters, and scene-structure and dependency queries. | Can index content through the Storage API, so applications find assets before accessing and assembling them. |

For example, a content workflow can use **usd-search** to find an asset, **ovstorage** to access its files, **ovpackage** to publish and install a selected version, **ovlive** to collaborate on a USD stage, and **usd-caches** to reuse content across loads. **usd-content-workspace** supplies the service interfaces and infrastructure adapters those workflows use. Each product has its own setup and compatibility requirements, and access permissions vary by repository; availability of source does not establish compatibility between products.
