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

use futures::future::BoxFuture;
use storage_core::traits::{BoxStream, Request, Result, StreamingOptions};

use crate::{
    backend::{
        metadata::{batch::*, batch_error_entry::Code, *},
        url_normalizer::{error::Error, UrlNormalizerService},
    },
    common::future::create_error_future,
};

impl<S: Metadata> Metadata for UrlNormalizerService<S> {
    fn inner(&self) -> Option<&dyn Metadata> {
        None
    }

    fn get_metadata_batch<'a: 'b, 'b>(
        &'a self,
        request: Request<GetMetadataBatchRequest>,
    ) -> BoxFuture<'b, Result<GetMetadataBatchResponse>> {
        self.normalize_resource_locations_and_invoke(
            request,
            |message| &mut message.requests,
            |request| self.inner.get_metadata_batch(request),
        )
    }

    fn query_metadata<'a: 'b, 'b>(
        &'a self,
        request: Request<QueryMetadataRequest>,
        options: StreamingOptions,
    ) -> BoxFuture<'b, Result<BoxStream<QueryMetadataResponsePage>>> {
        self.invoke_with_resource_location(
            request,
            |message| message.resource_location.as_mut(),
            |request| self.inner.query_metadata(request, options),
        )
    }

    fn update_metadata_batch<'a: 'b, 'b>(
        &'a self,
        request: Request<UpdateMetadataBatchRequest>,
    ) -> BoxFuture<'b, Result<UpdateMetadataBatchResponse>> {
        self.normalize_resource_locations_and_invoke(
            request,
            |message| &mut message.requests,
            |request| self.inner.update_metadata_batch(request),
        )
    }

    fn delete_metadata_batch<'a: 'b, 'b>(
        &'a self,
        request: Request<DeleteMetadataBatchRequest>,
    ) -> BoxFuture<'b, Result<DeleteMetadataBatchResponse>> {
        self.normalize_resource_locations_and_invoke(
            request,
            |message| &mut message.requests,
            |request| self.inner.delete_metadata_batch(request),
        )
    }
}

impl<S> UrlNormalizerService<S> {
    /// Normalize resource locations and invoke the underlying batch operation.
    #[track_caller]
    fn normalize_resource_locations_and_invoke<
        'a,
        ReqT,
        ReqE: BatchRequestEntry,
        ResE: BatchResponseEntry + Send + 'a,
        ResT: BatchResponse<ResponseEntry = ResE> + Send + 'a,
    >(
        &self,
        mut request: Request<ReqT>,
        parameter: impl FnOnce(&mut ReqT) -> &mut Vec<ReqE>,
        invoke: impl FnOnce(Request<ReqT>) -> BoxFuture<'a, Result<ResT>>,
    ) -> BoxFuture<'a, Result<ResT>> {
        let requests = parameter(request.get_mut());
        let mut errors = Vec::new();
        let mut response_map: Option<Vec<_>> = None;

        let mut original_request_index = 0;
        requests.retain_mut(|entry| {
            let resource_location = entry.resource_location_mut();
            if let Err(error) = self.normalize_resource_location_in_place(resource_location) {
                errors.push(ResE::from_error_entry(
                    original_request_index as u32,
                    create_batch_error_entry(error),
                ));
                if response_map.is_none() {
                    response_map = Some((0..original_request_index as u32).collect());
                }

                original_request_index += 1;

                false
            } else {
                if let Some(response_map) = response_map.as_mut() {
                    response_map.push(original_request_index as u32);
                }

                original_request_index += 1;

                true
            }
        });

        let Some(response_map) = response_map else {
            return invoke(request);
        };

        if requests.is_empty() {
            return Box::pin(async move { Ok(ResT::from_entries(errors)) });
        }

        let future = invoke(request);

        Box::pin(async move {
            let mut response = future.await?;
            response.update_request_indexes(&response_map);
            response.extend(errors);

            Ok(response)
        })
    }

    #[track_caller]
    fn invoke_with_resource_location<'a, T, R>(
        &self,
        mut request: Request<T>,
        parameter: impl FnOnce(&mut T) -> Option<&mut ResourceLocation>,
        invoke: impl FnOnce(Request<T>) -> BoxFuture<'a, Result<R>>,
    ) -> BoxFuture<'a, Result<R>> {
        if let Err(error) = self.normalize_resource_location_in_place(parameter(request.get_mut())) {
            return create_error_future(error);
        }

        invoke(request)
    }

    #[track_caller]
    fn normalize_resource_location_in_place(
        &self,
        resource_location: Option<&mut ResourceLocation>,
    ) -> std::result::Result<(), Error> {
        use crate::backend::metadata::resource_location::Resource;

        // TODO: Technically, this must be an error.
        let Some(resource) = resource_location
            .map(|location| location.resource.as_mut())
            .unwrap_or_default()
        else {
            return Ok(());
        };

        match resource {
            Resource::ResourceAddress(value) => self.normalize_resource_address(value),
            Resource::FolderAddress(value) => self.normalize_folder_address(Some(&mut value.uri)),
            Resource::ResourceIdentity(..) => Ok(()),
        }
    }
}

fn create_batch_error_entry(error: Error) -> BatchErrorEntry {
    BatchErrorEntry {
        code: Code::InvalidArgument as i32,
        message: error.to_string(),
    }
}
