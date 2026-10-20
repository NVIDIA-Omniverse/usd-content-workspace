<!-- SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved. -->
<!-- SPDX-License-Identifier: Apache-2.0 -->

# Connect to a deployment

If a deployment already exists, you need only its discovery URL.

## Navigator UI

The [Storage Navigator UI](../../examples/navigator-ui/README.md) is a browser client for a deployment's Discovery, Storage, and Metadata services. It needs Node.js 22+ and runs locally with `npm run` scripts or in Kubernetes with its [Helm chart](../../examples/navigator-ui/helm/).

## From an application

To access content from your own code, use [ovstorage](https://github.com/NVIDIA-Omniverse/ovstorage) with its `services-client` plugin pointed at the deployment, or generate a client from the [Storage API contracts](../../storage/api/README.md).
