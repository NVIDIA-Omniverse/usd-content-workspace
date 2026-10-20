// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use storage_service_protos::nvidia::omniverse::metadata::v1beta as mp;
use storage_service_protos::nvidia::omniverse::storage::metadata::v1alpha as ssp;

pub fn get_metadata_batch_request(request: mp::GetMetadataBatchRequest) -> ssp::GetMetadataBatchRequest {
    ssp::GetMetadataBatchRequest {
        requests: request
            .requests
            .into_iter()
            .map(|entry| ssp::GetMetadataBatchRequestEntry {
                resource_location: entry.resource_location.map(resource_location_to_ssp),
                user_metadata_key: entry.user_metadata_key,
                namespace: entry.namespace,
            })
            .collect(),
    }
}

pub fn query_metadata_request(request: mp::QueryMetadataRequest) -> ssp::QueryMetadataRequest {
    ssp::QueryMetadataRequest {
        resource_location: request.resource_location.map(resource_location_to_ssp),
        namespace: request.namespace,
    }
}

pub fn update_metadata_batch_request(request: mp::UpdateMetadataBatchRequest) -> ssp::UpdateMetadataBatchRequest {
    ssp::UpdateMetadataBatchRequest {
        requests: request
            .requests
            .into_iter()
            .map(|entry| ssp::UpdateMetadataBatchRequestEntry {
                resource_location: entry.resource_location.map(resource_location_to_ssp),
                user_metadata_key: entry.user_metadata_key,
                user_metadata: entry.user_metadata,
                expected_etag: entry.expected_etag,
                namespace: entry.namespace,
            })
            .collect(),
    }
}

pub fn delete_metadata_batch_request(request: mp::DeleteMetadataBatchRequest) -> ssp::DeleteMetadataBatchRequest {
    ssp::DeleteMetadataBatchRequest {
        requests: request
            .requests
            .into_iter()
            .map(|entry| ssp::DeleteMetadataBatchRequestEntry {
                resource_location: entry.resource_location.map(resource_location_to_ssp),
                user_metadata_key: entry.user_metadata_key,
                expected_etag: entry.expected_etag,
                namespace: entry.namespace,
            })
            .collect(),
    }
}

pub fn get_metadata_batch_response(response: ssp::GetMetadataBatchResponse) -> mp::GetMetadataBatchResponse {
    use mp::get_metadata_batch_response_entry::Outcome as Out;
    use ssp::get_metadata_batch_response_entry::Outcome as In;

    mp::GetMetadataBatchResponse {
        results: response
            .results
            .into_iter()
            .map(|entry| mp::GetMetadataBatchResponseEntry {
                request_index: entry.request_index,
                outcome: entry.outcome.map(|outcome| match outcome {
                    In::Success(value) => Out::Success(user_metadata_value_to_mp(value)),
                    In::Error(error) => Out::Error(batch_error_to_mp(error)),
                }),
            })
            .collect(),
    }
}

pub fn query_metadata_response(response: ssp::QueryMetadataResponse) -> mp::QueryMetadataResponse {
    mp::QueryMetadataResponse {
        results: response
            .results
            .into_iter()
            .map(|entry| mp::QueryMetadataResponseEntry {
                user_metadata_key: entry.user_metadata_key,
                user_metadata: entry.user_metadata.map(user_metadata_value_to_mp),
            })
            .collect(),
    }
}

pub fn update_metadata_batch_response(response: ssp::UpdateMetadataBatchResponse) -> mp::UpdateMetadataBatchResponse {
    use mp::update_metadata_batch_response_entry::Outcome as Out;
    use ssp::update_metadata_batch_response_entry::Outcome as In;

    mp::UpdateMetadataBatchResponse {
        results: response
            .results
            .into_iter()
            .map(|entry| mp::UpdateMetadataBatchResponseEntry {
                request_index: entry.request_index,
                outcome: entry.outcome.map(|outcome| match outcome {
                    In::Success(success) => {
                        Out::Success(mp::UpdateMetadataBatchResponseEntrySuccess { etag: success.etag })
                    },
                    In::Error(error) => Out::Error(batch_error_to_mp(error)),
                }),
            })
            .collect(),
    }
}

pub fn delete_metadata_batch_response(response: ssp::DeleteMetadataBatchResponse) -> mp::DeleteMetadataBatchResponse {
    use mp::delete_metadata_batch_response_entry::Outcome as Out;
    use ssp::delete_metadata_batch_response_entry::Outcome as In;

    mp::DeleteMetadataBatchResponse {
        results: response
            .results
            .into_iter()
            .map(|entry| mp::DeleteMetadataBatchResponseEntry {
                request_index: entry.request_index,
                outcome: entry.outcome.map(|outcome| match outcome {
                    In::Success(_) => Out::Success(mp::DeleteMetadataBatchResponseEntrySuccess {}),
                    In::Error(error) => Out::Error(batch_error_to_mp(error)),
                }),
            })
            .collect(),
    }
}

fn resource_location_to_ssp(value: mp::ResourceLocation) -> ssp::ResourceLocation {
    use mp::resource_location::Resource as In;
    use ssp::resource_location::Resource as Out;
    use storage_service_protos::nvidia::omniverse::storage::{
        filefolder::v1alpha::FolderAddress, fileobject::v1alpha::ResourceIdentity,
    };

    ssp::ResourceLocation {
        resource: value.resource.map(|resource| match resource {
            In::ResourceAddress(uri) => Out::ResourceAddress(uri),
            In::ResourceIdentity(id) => Out::ResourceIdentity(ResourceIdentity {
                encoded_identity: id.encoded_identity,
            }),
            In::FolderAddress(folder) => Out::FolderAddress(FolderAddress { uri: folder.uri }),
        }),
    }
}

fn user_metadata_value_to_mp(value: ssp::UserMetadataValue) -> mp::UserMetadataValue {
    mp::UserMetadataValue {
        value: value.value,
        etag: value.etag,
    }
}

fn batch_error_to_mp(value: ssp::BatchErrorEntry) -> mp::BatchErrorEntry {
    mp::BatchErrorEntry {
        code: value.code,
        message: value.message,
    }
}
