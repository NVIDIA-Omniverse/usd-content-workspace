# SPDX-FileCopyrightText: Copyright (c) 2024-2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
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

import asyncio
import os.path
import urllib.parse
from datetime import (
    datetime,
    timezone,
)
from typing import Optional

import uvicorn
from fastapi import (
    FastAPI,
    HTTPException,
)
from pydantic import BaseModel
from service_utils import (
    STATIC_DIR,
    create_identity,
    path_from_identity,
)
from starlette.responses import (
    JSONResponse,
    StreamingResponse,
)

app = FastAPI()


# start pydantic stat example
class Metadata(BaseModel):
    data_object_size: int
    last_modified_timestamp: Optional[str] = None


# end pydantic stat example


# start stat example
@app.head("/fileobject/by-address/{resource_address:path}")
async def stat_api(resource_address: str):
    path = os.path.join(STATIC_DIR, resource_address)
    if os.path.exists(path):
        size = os.path.getsize(path)
        modification_time = datetime.fromtimestamp(os.path.getmtime(path), tz=timezone.utc).isoformat()
        metadata = Metadata(data_object_size=size, last_modified_timestamp=modification_time)
        return JSONResponse(
            status_code=204,
            content=None,
            headers={
                "x-nvidia-omniverse-storage-metadata": metadata.model_dump_json(),
                "x-nvidia-omniverse-storage-resource-identity": create_identity(resource_address, modification_time),
            },
        )
    else:
        raise HTTPException(status_code=404, detail=f"{path} not found on disk")


# end stat example


# start read example
@app.get("/fileobject/by-identity/{resource_identity:path}")
async def read(resource_identity: str, download_preference: Optional[str] = None):
    relative_path = path_from_identity(urllib.parse.unquote_plus(resource_identity))
    path = os.path.join(STATIC_DIR, relative_path)
    if os.path.exists(path):

        def load_file():
            with open(path, "rb") as f:
                while chunk := f.read(1024):
                    yield chunk

        return StreamingResponse(load_file(), media_type="application/octet-stream")
    else:
        raise HTTPException(status_code=404, detail=f"{path} not found on disk")


# end read example


# start main loop
config = uvicorn.Config(app, port=8011)
server = uvicorn.Server(config)
asyncio.run(server.serve())
# end main loop
