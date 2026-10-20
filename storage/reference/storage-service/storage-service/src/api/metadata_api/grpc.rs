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

pub mod v1beta {
    use async_stream::try_stream;
    use futures::{stream::BoxStream, TryStreamExt};
    use storage_core::{
        ingress::ToGrpcService,
        traits::{async_trait, Request as CoreRequest, StreamingOptions},
    };
    use storage_service_protos::nvidia::omniverse::metadata::v1beta::{
        metadata_service_server::{MetadataService, MetadataServiceServer},
        *,
    };
    use tonic::{Request, Response, Status};
    use tracing::error;

    use crate::{
        api::metadata_api::{adapters, v1beta::Metadata},
        backend::metadata::Metadata as MetadataTrait,
        common::request::RequestExt,
    };

    type ResponseResult<T> = std::result::Result<Response<T>, Status>;

    const RESPONSE_PAGE_SIZE: usize = 32;

    impl<T: MetadataTrait> ToGrpcService for Metadata<T> {
        type Service = MetadataServiceServer<Metadata<T>>;

        fn file_descriptor_set(&self) -> Option<&[u8]> {
            Some(FILE_DESCRIPTOR_SET)
        }

        fn service(&self) -> Self::Service {
            MetadataServiceServer::new(self.clone())
        }
    }

    #[async_trait]
    impl<T: MetadataTrait> MetadataService for Metadata<T> {
        type GetMetadataBatchStream = BoxStream<'static, Result<GetMetadataBatchResponse, Status>>;
        type UpdateMetadataBatchStream = BoxStream<'static, Result<UpdateMetadataBatchResponse, Status>>;
        type DeleteMetadataBatchStream = BoxStream<'static, Result<DeleteMetadataBatchResponse, Status>>;
        type QueryMetadataStream = BoxStream<'static, Result<QueryMetadataResponse, Status>>;

        async fn get_metadata_batch(
            &self,
            request: Request<GetMetadataBatchRequest>,
        ) -> ResponseResult<Self::GetMetadataBatchStream> {
            let (metadata, extensions, body) = CoreRequest::from(request).into_parts();
            let request = CoreRequest::from_parts(metadata, extensions, adapters::get_metadata_batch_request(body));

            let response = self
                .inner
                .get_metadata_batch(request)
                .await
                .map_err(|error| error.tonic_status())
                .inspect_err(|status| error!(?status, "get metadata batch request failed"))?;

            let results = adapters::get_metadata_batch_response(response).results;
            Ok(Response::new(paginate_results(results, |results| {
                GetMetadataBatchResponse { results }
            })))
        }

        async fn query_metadata(
            &self,
            request: Request<QueryMetadataRequest>,
        ) -> ResponseResult<Self::QueryMetadataStream> {
            let (metadata, extensions, body) = CoreRequest::from(request).into_parts();
            let request = CoreRequest::from_parts(metadata, extensions, adapters::query_metadata_request(body));

            let response = self
                .inner
                .query_metadata(request, StreamingOptions::Streamed)
                .await
                .map_err(|error| error.tonic_status())
                .inspect_err(|status| error!(?status, "query metadata request failed"))?;

            Ok(Response::new(Box::pin(
                response
                    .map_ok(|page| adapters::query_metadata_response(page.response))
                    .map_err(|error| error.tonic_status())
                    .inspect_err(|status| error!(?status, "query metadata request failed")),
            )))
        }

        async fn update_metadata_batch(
            &self,
            request: Request<UpdateMetadataBatchRequest>,
        ) -> ResponseResult<Self::UpdateMetadataBatchStream> {
            let (metadata, extensions, body) = CoreRequest::from(request).into_parts();
            let request = CoreRequest::from_parts(metadata, extensions, adapters::update_metadata_batch_request(body));

            let response = self
                .inner
                .update_metadata_batch(request)
                .await
                .map_err(|error| error.tonic_status())
                .inspect_err(|status| error!(?status, "update metadata batch request failed"))?;

            let results = adapters::update_metadata_batch_response(response).results;
            Ok(Response::new(paginate_results(results, |results| {
                UpdateMetadataBatchResponse { results }
            })))
        }

        async fn delete_metadata_batch(
            &self,
            request: Request<DeleteMetadataBatchRequest>,
        ) -> ResponseResult<Self::DeleteMetadataBatchStream> {
            let (metadata, extensions, body) = CoreRequest::from(request).into_parts();
            let request = CoreRequest::from_parts(metadata, extensions, adapters::delete_metadata_batch_request(body));

            let response = self
                .inner
                .delete_metadata_batch(request)
                .await
                .map_err(|error| error.tonic_status())
                .inspect_err(|status| error!(?status, "delete metadata batch request failed"))?;

            let results = adapters::delete_metadata_batch_response(response).results;
            Ok(Response::new(paginate_results(results, |results| {
                DeleteMetadataBatchResponse { results }
            })))
        }
    }

    fn paginate_results<T, F, P>(results: Vec<T>, map: F) -> BoxStream<'static, Result<P, Status>>
    where
        F: Fn(Vec<T>) -> P + Send + Sync + 'static,
        T: Send + Sync + 'static,
        P: Send + Sync + 'static,
    {
        Box::pin(try_stream! {
            let mut results = results.into_iter();
            while let Some(first) = results.next() {
                let mut current = Vec::with_capacity(RESPONSE_PAGE_SIZE);
                current.push(first);

                while current.len() < RESPONSE_PAGE_SIZE {
                    if let Some(next) = results.next() {
                        current.push(next);
                    } else {
                        break;
                    }
                }

                yield map(current);
            }
        })
    }
}
