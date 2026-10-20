# SPDX-FileCopyrightText: Copyright (c) 2025-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

"""gRPC service implementation for NVIDIA Omniverse Storage API.

This package provides gRPC-based access to the filesystem storage service,
implementing the NVIDIA Omniverse Storage API protocol over gRPC. It parallels
the REST service implementation but uses Protocol Buffers and gRPC for
communication.

Modules:
    server: gRPC server setup and initialization
    capabilities: Service discovery and capability advertisement
    fileobject: Core file operations (read, write, delete, enumerate)
    filefolder: Folder operations (list, create, delete)
    versioning: File version enumeration and management
    metadata: User-defined metadata storage and retrieval
    helpers: Common utility functions for gRPC handlers

The service supports multiple API versions (v1alpha, v1beta) and provides
both alpha and beta stability guarantees for different features.
"""
