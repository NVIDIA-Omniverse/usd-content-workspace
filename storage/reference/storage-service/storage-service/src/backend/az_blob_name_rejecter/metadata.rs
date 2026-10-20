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

use futures::future::{ready, BoxFuture};
use snafu::IntoError;
use storage_core::traits::{BoxStream, Request, Result, StreamingOptions};

use crate::{
    backend::{
        az_blob_name_rejecter::{
            error::{error, Error},
            AzureBlobNameRejecterService,
        },
        metadata::{batch::*, batch_error_entry::Code, resource_location::Resource, *},
    },
    common::{error::PrettyPrintErrorExt, future::create_error_future, request::RequestExt},
};

impl<S> AzureBlobNameRejecterService<S> {
    fn invoke_with_resource_locations<'a, ReqE, ResE, ResT>(
        &self,
        mut requests: Vec<ReqE>,
        invoke: impl FnOnce(Vec<ReqE>) -> BoxFuture<'a, Result<ResT>> + Send + 'a,
    ) -> BoxFuture<'a, Result<ResT>>
    where
        ReqE: BatchRequestEntry + Send + 'a,
        ResT: BatchResponse<ResponseEntry = ResE> + Send + 'a,
        ResE: BatchResponseEntry + Send + 'a,
    {
        let mut errors = Vec::new();
        for (request_index, entry) in requests.iter().enumerate() {
            if let Err(error) = self.validate_resource_location(entry.resource_location()) {
                tracing::debug!(
                    error = %error.pretty(),
                    request_index,
                    resource_location = ?entry.resource_location(),
                    "metadata resource validation failed"
                );

                errors.push(ResE::from_error_entry(
                    request_index as u32,
                    create_batch_error_entry(&error),
                ));
            }
        }

        if errors.is_empty() {
            return invoke(requests);
        }

        if errors.len() == requests.len() {
            return Box::pin(ready(Ok(ResT::from_entries(errors))));
        }

        Box::pin(async move {
            let response_map = {
                let mut response_map = Vec::with_capacity(requests.len() - errors.len());
                let mut error_indexes = errors.iter().map(BatchResponseEntry::request_index).peekable();
                let mut request_index = 0u32;
                requests.retain(|_| {
                    let retain = if error_indexes.peek() == Some(&request_index) {
                        error_indexes.next();
                        false
                    } else {
                        response_map.push(request_index);
                        true
                    };

                    request_index += 1;

                    retain
                });

                response_map
            };

            let mut response = invoke(requests).await?;
            response.update_request_indexes(&response_map);
            response.extend(errors);

            Ok(response)
        })
    }

    fn validate_resource_location(&self, location: Option<&ResourceLocation>) -> Result<(), Error> {
        use error::*;

        let resource = location
            .and_then(|location| location.resource.as_ref())
            .ok_or_else(|| ResourceLocationMissing.build())?;
        match resource {
            Resource::ResourceAddress(address) => {
                self.validate_blob_path(address, |source| ResourceAddressInvalid.into_error(source))
            },
            Resource::ResourceIdentity(identity) => self.validate_blob_path(&identity.encoded_identity, |source| {
                ResourceIdentityInvalid.into_error(source)
            }),
            Resource::FolderAddress(folder) => {
                self.validate_blob_path(&folder.uri, |source| FolderAddressInvalid.into_error(source))
            },
        }
    }
}

impl<S: Metadata> Metadata for AzureBlobNameRejecterService<S> {
    fn inner(&self) -> Option<&dyn Metadata> {
        None
    }

    fn get_metadata_batch<'a: 'b, 'b>(
        &'a self,
        request: Request<GetMetadataBatchRequest>,
    ) -> BoxFuture<'b, Result<GetMetadataBatchResponse>> {
        let (metadata, extensions, message) = request.into_parts();
        self.invoke_with_resource_locations(message.requests, |requests| {
            self.inner.get_metadata_batch(Request::from_parts(
                metadata,
                extensions,
                GetMetadataBatchRequest { requests },
            ))
        })
    }

    fn query_metadata<'a: 'b, 'b>(
        &'a self,
        request: Request<QueryMetadataRequest>,
        options: StreamingOptions,
    ) -> BoxFuture<'b, Result<BoxStream<QueryMetadataResponsePage>>> {
        match self.validate_resource_location(request.get_ref().resource_location.as_ref()) {
            Ok(()) => self.inner.query_metadata(request, options),
            Err(error) => create_error_future(error),
        }
    }

    fn update_metadata_batch<'a: 'b, 'b>(
        &'a self,
        request: Request<UpdateMetadataBatchRequest>,
    ) -> BoxFuture<'b, Result<UpdateMetadataBatchResponse>> {
        let (metadata, extensions, message) = request.into_parts();
        self.invoke_with_resource_locations(message.requests, |requests| {
            self.inner.update_metadata_batch(Request::from_parts(
                metadata,
                extensions,
                UpdateMetadataBatchRequest { requests },
            ))
        })
    }

    fn delete_metadata_batch<'a: 'b, 'b>(
        &'a self,
        request: Request<DeleteMetadataBatchRequest>,
    ) -> BoxFuture<'b, Result<DeleteMetadataBatchResponse>> {
        let (metadata, extensions, message) = request.into_parts();
        self.invoke_with_resource_locations(message.requests, |requests| {
            self.inner.delete_metadata_batch(Request::from_parts(
                metadata,
                extensions,
                DeleteMetadataBatchRequest { requests },
            ))
        })
    }
}

fn create_batch_error_entry(error: &Error) -> BatchErrorEntry {
    match error {
        Error::DFALookup { .. } => create_batch_error_entry_v1alpha(Code::Internal, "unexpected error"),
        _ => create_batch_error_entry_v1alpha(Code::InvalidArgument, error.to_string()),
    }
}
