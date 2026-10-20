.. SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
.. SPDX-License-Identifier: Apache-2.0

NVIDIA usd-content-workspace
============================

**usd-content-workspace** is the service foundation of Omniverse Content Services, bringing APIs, reference services, tests, and client examples into one product monorepo. Use it to build content workflows on your existing storage and infrastructure.

In this documentation you will find how to choose the services a workflow needs, how to connect to a deployment, deploy a service, or implement a compatible one, and how the components fit with the rest of Omniverse Content Services.

* :doc:`Choosing components <getting_started/choosing_components>` - Decide which services your workflow needs and the order to stand them up.
* :doc:`Connect to a deployment <getting_started/connect>` - Browse content on an existing deployment with the Navigator UI, or embed access with ovstorage.
* :doc:`Deploy a service <getting_started/deploy>` - Deploy the reference services to Kubernetes with their Helm charts.
* :doc:`Implement a compatible service <getting_started/implement>` - Build against a contract and check it with the conformance suite.

.. note::

   Each component keeps its own version, build workspace, and deployment requirements. There is no combined build, installer, or release; follow the guide for the component and version you select.

Features
--------

* Access, write, and manage content on S3 or Azure Blob Storage through the Storage API's gRPC and REST interfaces.
* Apply permissions, authorize bearer tokens through Envoy, and resolve users and groups through Microsoft Graph.
* Attach key/value metadata, and publish and consume content events over RabbitMQ.
* Discover service endpoints and generate thumbnails for stored content.
* Implement compatible services from Protobuf and OpenAPI contracts, conformance suites, and the Storage Core Rust framework.

Components
----------

.. list-table::
   :header-rows: 1

   * - Area
     - What it provides
   * - Storage
     - Storage API contract, Storage Service (S3 and Azure Blob Storage), Storage Core framework
   * - Permissions
     - Permission Service, Envoy auth extension, permission UIs
   * - User identity
     - Microsoft Graph User Info Service
   * - Metadata
     - Metadata API definitions, implemented by the Storage Service
   * - Notifications
     - Event Aggregation and Event Consumer services, shared library
   * - Discovery
     - Discovery operator and legacy chart
   * - Thumbnails
     - Thumbnail Service and conversion worker
   * - Examples
     - Storage Navigator UI

See the :doc:`component map <overview/components>` for every contract, implementation, and conformance link.

License
-------

All NVIDIA-authored code is licensed under the `Apache License 2.0 <https://github.com/NVIDIA-Omniverse/usd-content-workspace/blob/main/LICENSE>`_. Components carry their own LICENSE files, and third-party components remain under their own licenses.

.. toctree::
   :maxdepth: 2
   :caption: Getting Started

   getting_started/choosing_components
   getting_started/connect
   getting_started/deploy
   getting_started/implement

.. toctree::
   :maxdepth: 2
   :caption: Overview

   overview/components
   overview/content_services
   overview/source_and_releases

.. toctree::
   :maxdepth: 2
   :caption: AI Coding Agents

   agents

.. toctree::
   :caption: Links

   GitHub Repository <https://github.com/NVIDIA-Omniverse/usd-content-workspace>

Indices and Tables
==================

* :ref:`genindex`
* :ref:`search`
