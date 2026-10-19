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

import concurrent.futures
import os
from datetime import (
    datetime,
    timezone,
)

import grpc
import nvidia.omniverse.storage.fileobject.v1alpha.fileobject_service_pb2
from google.protobuf import timestamp_pb2
from nvidia.omniverse.storage.fileobject.v1alpha import (
    fileobject_pb2,
    fileobject_service_pb2,
)

# start stat example
from nvidia.omniverse.storage.fileobject.v1alpha.fileobject_service_pb2_grpc import (
    FileObjectServiceServicer,
    add_FileObjectServiceServicer_to_server,
)
from service_utils import (
    STATIC_DIR,
    args,
    create_identity,
)


class FileSystemServiceServicer(FileObjectServiceServicer):
    def Stat(self, request, context):
        path = os.path.join(STATIC_DIR, request.resource_address)
        if os.path.exists(path):
            # First build the ResourceIdentity from resource address and modification time
            modification_time = os.path.getmtime(path)
            modification_time_iso = datetime.fromtimestamp(modification_time, tz=timezone.utc).isoformat()
            resource_identity = fileobject_pb2.ResourceIdentity(
                encoded_identity=create_identity(request.resource_address, modification_time_iso)
            )

            # Then create the Metadata info using google Timestamp
            stat_result = os.stat(path)
            modification_time_seconds = int(stat_result.st_mtime)
            nanos = int((stat_result.st_mtime - modification_time_seconds) * 1e9)
            timestamp = timestamp_pb2.Timestamp(seconds=int(stat_result.st_mtime), nanos=nanos)
            metadata = fileobject_pb2.Metadata(data_object_size=stat_result.st_size, last_modified_timestamp=timestamp)

            # Now return the result
            return fileobject_service_pb2.StatResponse(
                resource_info=fileobject_pb2.ResourceInfo(resource_identity=resource_identity, metadata=metadata)
            )
        else:
            context.abort(grpc.StatusCode.NOT_FOUND, f"file not found: {path}")


# end stat example


def launch_without_reflection():
    # start example launch
    server = grpc.server(concurrent.futures.ThreadPoolExecutor(max_workers=10))
    add_FileObjectServiceServicer_to_server(FileSystemServiceServicer(), server)
    server.add_insecure_port("[::]:50051")
    server.start()
    print(f"GRPC Server listening on port 50051 serving directory '{STATIC_DIR}'")
    server.wait_for_termination()
    # end example launch


def launch_with_reflection():
    # start example launch-with-reflection
    server = grpc.server(concurrent.futures.ThreadPoolExecutor(max_workers=10))
    add_FileObjectServiceServicer_to_server(FileSystemServiceServicer(), server)

    # Enable reflection
    from grpc_reflection.v1alpha import reflection

    SERVICE_NAMES = (
        nvidia.omniverse.storage.fileobject.v1alpha.fileobject_service_pb2.DESCRIPTOR.services_by_name["FileObjectService"].full_name,
        reflection.SERVICE_NAME,
    )
    reflection.enable_server_reflection(SERVICE_NAMES, server)
    server.add_insecure_port("[::]:50051")
    server.start()
    print(f"GRPC Server listening on port 50051 serving directory '{STATIC_DIR}'")
    server.wait_for_termination()
    # end example launch-with-reflection


if args.reflection:
    launch_with_reflection()
else:
    launch_without_reflection()
