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

"""FastAPI application setup and route configuration for REST service.

This module sets up the main FastAPI application and mounts all service
sub-applications for different API versions. The Storage API uses semantic
versioning with alpha/beta/stable releases, where different API versions
can have different feature sets and are mounted separately.

Services are organized by version:
- v1alpha: Includes all services including experimental features (Copy, Move, ListRoutes)
- v1beta: Includes only stable, production-ready services

Each service is mounted at its versioned path (e.g., /v1alpha/fileobject).
"""

# =============================================================================
# FastAPI Application Setup
# =============================================================================
from fastapi import (
    FastAPI,
    Request,
)
from fastapi.responses import JSONResponse
from local_filesystem_service.backends.storage_backend_interface import ReadOnlyError
from starlette import status

# Main FastAPI application
# HTTP routes for redirect-based operations will be registered lazily
# after backend initialization - see __main__.py
app = FastAPI()

# =============================================================================
# API Version Routing
# =============================================================================
# The Storage API uses semantic versioning with alpha/beta/stable releases.
# Different API versions can have different feature sets and are mounted separately.
# We follow channel based releases, so alpha in one specific version is a strict superset of beta, which in turn is a
# strict superset of the release version if any is available.

# Capabilities service - Advertises service capabilities and available routes
capabilities_service = FastAPI()
capabilities_service_alpha = FastAPI()
app.mount("/v1alpha/capabilities", capabilities_service_alpha)
app.mount("/v1beta/capabilities", capabilities_service)

# FileObject service - Core data object operations (read, write, delete, enumerate)
fileobject_service_beta = FastAPI()  # Beta version - no copy/move endpoints
fileobject_service_alpha = FastAPI()  # Alpha version - includes copy and move endpoints
app.mount("/v1alpha/fileobject", fileobject_service_alpha)
app.mount("/v1beta/fileobject", fileobject_service_beta)

# FileFolder service - Folder operations (list, create, delete folders)
filefolder_service_alpha = FastAPI()
filefolder_service_beta = FastAPI()
app.mount("/v1alpha/filefolder", filefolder_service_alpha)
app.mount("/v1beta/filefolder", filefolder_service_beta)

# Versioning service - Enumerate and manage file versions
versioning_service_alpha = FastAPI()
app.mount("/v1alpha/versioning", versioning_service_alpha)
versioning_service_beta = FastAPI()
app.mount("/v1beta/versioning", versioning_service_beta)

# Metadata service - User-defined key-value metadata storage
metadata_service = FastAPI()
app.mount("/v1alpha/metadata", metadata_service)


# =============================================================================
# Read-only Enforcement
# =============================================================================
# Read-only rejection is raised at the backend mutation boundary (ReadOnlyError) so
# every write path is covered regardless of which handler reached it. Map it to HTTP
# 412 Precondition Failed here — mounted sub-applications handle their own exceptions,
# so the handler is registered on every app instance rather than only the parent.
async def _read_only_exception_handler(request: "Request", exc: Exception) -> JSONResponse:
    return JSONResponse(status_code=status.HTTP_412_PRECONDITION_FAILED, content={"detail": str(exc)})


for _service_app in (
    app,
    capabilities_service,
    capabilities_service_alpha,
    fileobject_service_alpha,
    fileobject_service_beta,
    filefolder_service_alpha,
    filefolder_service_beta,
    versioning_service_alpha,
    versioning_service_beta,
    metadata_service,
):
    _service_app.add_exception_handler(ReadOnlyError, _read_only_exception_handler)
