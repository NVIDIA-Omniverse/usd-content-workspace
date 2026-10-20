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

from argparse import ArgumentParser
from contextlib import contextmanager
from queue import Queue
from typing import (
    IO,
    Iterable,
    Iterator,
)

import requests
from grpc import (
    Channel,
    insecure_channel,
)
from nvidia.omniverse.storage.fileobject.v1alpha.fileobject_pb2 import (
    Chunk,
    Header,
)
from nvidia.omniverse.storage.fileobject.v1alpha.fileobject_service_pb2 import (
    CompletedUploadPart,
    CompleteMultipartUploadRequest,
    CompleteRedirectUploadRequest,
    CreateMultipartUploadResponse,
    FetchWriteTypeInfoRequest,
    UploadMethod,
    UploadPartRequest,
    UploadPartResponse,
    UploadPreference,
    WriteParameters,
    WriteRedirectProperties,
    WriteRequest,
    WriteResponse,
)
from nvidia.omniverse.storage.fileobject.v1alpha.fileobject_service_pb2_grpc import (
    FileObjectServiceStub,
)
from utils import (
    SupportsRead,
    get_content_length,
    part_count_for_multipart_upload,
    split_contents_for_multipart_upload,
    upload_part,
)

MAX_INLINE_CHUNK_SIZE = 3 * (2**20)


# start example
def upload(channel: Channel, address: str, upload_preference: str | None, content: IO[bytes]):
    """Upload the contents of a file object at the specified address."""

    content_length = get_content_length(content)
    if upload_preference:
        chosen_upload_preference = _string_to_upload_preference(upload_preference)
    else:
        chosen_upload_preference = _get_upload_preference(channel, address, content_length)

    service = FileObjectServiceStub(channel)
    with _write_message_queue() as (requests, request_iterator):
        requests.put(
            WriteRequest(
                params=WriteParameters(
                    destination_resource_address=address,
                    data_object_size=content_length,
                    upload_preference=chosen_upload_preference,
                ),
            ),
        )

        write_responses: Iterator[WriteResponse] = service.Write(request_iterator)

        flow_control_message = next(write_responses)

        # A service may immediately respond with a "resource info" message, meaning that a 0-byte object write has
        # been performed immediately without the need for any further actions on the client end.
        if flow_control_message.HasField("resource_info"):
            return

        # If we receive a "write chunks accepted" message, stream the chunks and finally await for a "resource info"
        # response message.
        if flow_control_message.HasField("write_chunks_accepted"):
            for chunk in _slice_content(content):
                requests.put(WriteRequest(chunk=chunk))

            # End chunk transmission.
            requests.put(None)

            resource_info_response = next(write_responses)
            if resource_info_response.HasField("resource_info"):
                return

            raise Exception("Resource info message is expected.")

    # Write redirect and multipart upload methods don't operate over the same stream of Write requests, so we should
    # close the write stream.
    if flow_control_message.HasField("write_redirect"):
        _write_via_redirect(service, address, flow_control_message.write_redirect, content)
        return

    if flow_control_message.HasField("multipart_upload"):
        _write_via_multipart(service, address, flow_control_message.multipart_upload, content)
        return

    raise Exception("Unexpected flow control message.")


# end example


def _slice_content(content: IO[bytes]) -> Iterable[Chunk]:
    """Slice the contents of a file into a series of Chunk messages."""

    while True:
        chunk = content.read(MAX_INLINE_CHUNK_SIZE)
        if not chunk:
            break

        yield Chunk(chunk=chunk)


@contextmanager
def _write_message_queue():
    # Limit the number of messages in the queue so we don't accidentally load the whole file into memory.
    request_messages: Queue[WriteRequest | None] = Queue(5)

    def yield_request_messages():
        while True:
            if message := request_messages.get():
                yield message
            else:
                break

    try:
        yield request_messages, yield_request_messages()
    finally:
        request_messages.put(None)


# start redirect write example
def _write_via_redirect(service: FileObjectServiceStub, address: str, parameters: WriteRedirectProperties, content: IO[bytes]):
    response = requests.request(
        url=parameters.redirect_target_url,
        method=_upload_method_to_string(parameters.method),
        headers={header.name: header.value for header in parameters.additional_headers},
        data=content,
    )
    response.raise_for_status()

    # Optionally, obtain the resource information of the just uploaded file object
    # This is not required to store the data, but helpful to keep the reference to the stored object.
    requested_headers = set(parameters.completion_header_names)

    # Create lowercase mapping for case-insensitive header matching
    response_headers_lower = {key.lower(): key for key in response.headers}

    service.CompleteRedirectUpload(
        CompleteRedirectUploadRequest(
            destination_resource_address=address,
            additional_headers=[
                Header(name=x, value=response.headers[response_headers_lower[x.lower()]])
                for x in requested_headers
                if x.lower() in response_headers_lower
            ],
        )
    )


# end redirect write example


# start multipart write example
def _write_via_multipart(service: FileObjectServiceStub, address: str, parameters: CreateMultipartUploadResponse, content: IO[bytes]):
    # Build a list of redirect URLs to upload the parts to. The first we got from the CreateMultipartUploadResponse
    redirects = [parameters.first_part_write_redirect]

    # Calculate the number of parts, and retrieve the pre-signed URLs from the service to use for individual part upload
    total_part_count, part_size, content_length = part_count_for_multipart_upload(
        content, parameters.minimum_size_per_part, parameters.maximum_size_per_part
    )
    if total_part_count > 1:
        response: UploadPartResponse = service.UploadPart(
            UploadPartRequest(
                upload_id=parameters.upload_id,
                destination_resource_address=address,
                part_number=1,  # Part number 0 has already been delivered by the first CreateMultipartUploadReponse
                part_count=total_part_count - 1,
            ),
        )
        redirects.extend(response.part_write_redirects)

    completed_parts = []
    for part_number, part in enumerate(
        split_contents_for_multipart_upload(
            content,
            min_part_size=parameters.minimum_size_per_part,
            max_part_size=parameters.maximum_size_per_part,
            max_parts=parameters.maximum_parts_number,
        )
    ):
        completed_parts.append(_write_part(redirects[part_number], part_number, part))

    service.CompleteMultipartUpload(
        CompleteMultipartUploadRequest(
            upload_id=parameters.upload_id,
            destination_resource_address=address,
            parts=completed_parts,
        ),
    )


def _write_part(redirect: WriteRedirectProperties, part: int, content: SupportsRead) -> CompletedUploadPart:
    return CompletedUploadPart(
        part_number=part,
        headers=[
            Header(name=name, value=value)
            for name, value in upload_part(
                url=redirect.redirect_target_url,
                method=_upload_method_to_string(redirect.method),
                upload_headers=dict([(header.name, header.value) for header in redirect.additional_headers]),
                return_headers=[name for name in redirect.completion_header_names],
                content=content,
            )
        ],
    )


# end multipart write example


# start upload preference example
def _get_upload_preference(channel: Channel, resource_address: str, size: int) -> UploadPreference:
    client = FileObjectServiceStub(channel)
    response = client.FetchWriteTypeInfo(FetchWriteTypeInfoRequest(destination_resource_address=resource_address))

    for interval in response.write_type_intervals:
        if interval.minimum_data_object_size and interval.minimum_data_object_size > size:
            continue

        if interval.maximum_data_object_size and interval.maximum_data_object_size < size:
            continue

        return interval.preferred_upload_method

    return UploadPreference.UPLOAD_PREFERENCE_UNSPECIFIED


# end upload preference example


def _string_to_upload_preference(value: str | None) -> UploadPreference:
    if not value:
        return UploadPreference.UPLOAD_PREFERENCE_UNSPECIFIED
    if value == "body":
        return UploadPreference.UPLOAD_PREFERENCE_BODY
    if value == "redirect":
        return UploadPreference.UPLOAD_PREFERENCE_REDIRECT
    if value == "multipart":
        return UploadPreference.UPLOAD_PREFERENCE_MULTIPART

    raise ValueError(f"Unknown upload preference {value!r}.")


def _upload_method_to_string(value: UploadMethod) -> str:
    if value == UploadMethod.UPLOAD_METHOD_POST:
        return "POST"
    elif value == UploadMethod.UPLOAD_METHOD_PUT:
        return "PUT"
    raise ValueError(f"Unsupported upload method {value!r}.")


def main():
    parser = ArgumentParser()
    parser.add_argument("resource_address", type=str)
    parser.add_argument("file", type=str)
    parser.add_argument("--upload-preference", default=None, choices=["body", "redirect", "multipart"])

    args = parser.parse_args()

    try:
        with open(args.file, mode="rb") as content:
            with insecure_channel("127.0.0.1:50051") as channel:
                upload(channel, args.resource_address, args.upload_preference, content)
    except Exception as e:
        print(f"Failed to upload: {e}")
        exit(1)


if __name__ == "__main__":
    main()
