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

use std::{
    collections::{hash_map::Entry, HashMap},
    sync::Arc,
};

use async_trait::async_trait;
use futures::{
    future::{try_join_all, BoxFuture},
    TryFutureExt,
};
use regex_automata::{
    dfa::{
        dense::{Config, DFA},
        Automaton, OverlappingState,
    },
    util::syntax::Config as SyntaxConfig,
    Input, MatchKind,
};
use snafu::{IntoError, Location, Snafu};
use storage_core::traits::{Request, Result, StorageErrorExt};
use tracing::{debug, info, trace, warn};

use crate::{
    backend::metadata::{any::AnyMetadata, batch::*, batch_error_entry::Code, *},
    common::{
        endpoint::EndpointUrl,
        error::PrettyPrintErrorExt,
        request::RequestExt,
        routing::{az_blob_any_uri_regex, az_blob_object_uri_regex, s3_any_uri_regex, s3_object_uri_regex},
    },
};

#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)), module, context(suffix(false)))]
pub enum InitError {
    #[snafu(display("failed to build the routing table"))]
    DfaBuild {
        source: Box<dyn std::error::Error + Send + Sync>,
        #[snafu(provide, implicit)]
        location: Location,
    },
}

#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)), module, context(suffix(false)))]
pub enum Error {
    #[snafu(display("resource uri is not supported"))]
    ResourceUriNotSupported {
        #[snafu(provide, implicit)]
        location: Location,
    },
}

impl StorageErrorExt for Error {
    fn get_size(&self) -> usize {
        std::mem::size_of::<Self>()
    }

    fn http_status(&self) -> axum::http::StatusCode {
        use axum::http::StatusCode;

        match self {
            Self::ResourceUriNotSupported { .. } => StatusCode::BAD_REQUEST,
        }
    }

    fn tonic_status(&self) -> tonic::Status {
        use tonic::Status;

        match self {
            Self::ResourceUriNotSupported { .. } => Status::invalid_argument(self.to_string()),
        }
    }
}

/// Metadata API router.
pub struct Router {
    router: DFA<Vec<u32>>,
    rules: Vec<RequestLookupRule>,
}

impl Router {
    /// Create a Metadata API router from Azure Blob and S3 metadata endpoints.
    pub fn try_from_routes(
        azure_routes: impl IntoIterator<Item = (AnyMetadata, impl AsRef<EndpointUrl>)>,
        s3_routes: impl IntoIterator<Item = (AnyMetadata, impl AsRef<EndpointUrl>)>,
    ) -> Result<Self, InitError> {
        let mut routes = Vec::new();
        let mut rules = Vec::new();
        let azure_routes = azure_routes.into_iter().map(|(service, endpoint)| {
            let endpoint = endpoint.as_ref();

            (
                service,
                az_blob_any_uri_regex(endpoint.scheme(), endpoint.hostname_port()),
                az_blob_object_uri_regex(endpoint.scheme(), endpoint.hostname_port()),
            )
        });
        let s3_routes = s3_routes.into_iter().map(|(service, endpoint)| {
            let endpoint = endpoint.as_ref();

            (
                service,
                s3_any_uri_regex(endpoint.scheme(), endpoint.hostname_port(), endpoint.path()),
                s3_object_uri_regex(endpoint.scheme(), endpoint.hostname_port(), endpoint.path()),
            )
        });
        for (service, folder_pattern, object_pattern) in azure_routes.chain(s3_routes) {
            routes.extend([folder_pattern, object_pattern]);
            rules.extend([
                RequestLookupRule::FolderOrObject(service.clone()),
                RequestLookupRule::Object(service),
            ]);
        }

        let router = if routes.is_empty() {
            DFA::never_match()
        } else {
            DFA::builder()
                .configure(Config::new().minimize(true).match_kind(MatchKind::All))
                .syntax(SyntaxConfig::new().unicode(false))
                .build_many(&routes)
        }
        .map_err(|source| init_error::DfaBuild.into_error(Box::new(source)))?;

        info!(memory_usage = ?router.memory_usage(), "metadata routing table created");

        Ok(Self { router, rules })
    }

    /// Resolve a metadata service that could handle requests to the specified uri.
    fn get_metadata_service_for_uri<'a>(
        &'a self,
        uri: &str,
        resolve: impl Fn(&'a RequestLookupRule) -> Option<&'a AnyMetadata>,
    ) -> Option<&'a AnyMetadata> {
        trace!(?uri, "looking up for a route");

        let input = Input::new(uri);
        let mut state = OverlappingState::start();
        loop {
            if let Err(error) = self.router.try_search_overlapping_fwd(&input, &mut state) {
                warn!(?uri, error = %error.pretty(), "resource uri matching failed");

                return None;
            }

            let Some(matched) = state.get_match() else {
                trace!(?uri, "route not found");

                return None;
            };

            let rule_id = matched.pattern().as_usize();
            let rule = &self.rules[rule_id];
            if let Some(service) = resolve(rule) {
                trace!(?uri, ?rule_id, "route found");

                return Some(service);
            }
        }
    }

    /// Resolve a metadata service that could handle requests to the specified resource location.
    fn get_metadata_service_for_resource_location_v1alpha<'a>(
        &'a self,
        resource_location: Option<&ResourceLocation>,
    ) -> Option<&'a AnyMetadata> {
        use crate::backend::metadata::resource_location::Resource;

        match resource_location?.resource.as_ref()? {
            Resource::ResourceAddress(value) => self.get_metadata_service_for_uri(value, |rule| match rule {
                RequestLookupRule::Object(service) => Some(service),
                RequestLookupRule::FolderOrObject(_) => None,
            }),
            Resource::ResourceIdentity(value) => {
                self.get_metadata_service_for_uri(&value.encoded_identity, |rule| match rule {
                    RequestLookupRule::Object(service) => Some(service),
                    RequestLookupRule::FolderOrObject(_) => None,
                })
            },
            Resource::FolderAddress(value) => self.get_metadata_service_for_uri(&value.uri, |rule| match rule {
                RequestLookupRule::FolderOrObject(service) | RequestLookupRule::Object(service) => Some(service),
            }),
        }
    }

    /// Route requests to zero or more different metadata backends.
    async fn route_requests_by_resource_location_v1alpha<
        'a,
        ReqE: BatchRequestEntry,
        ResE: BatchResponseEntry,
        ResT: BatchResponse<ResponseEntry = ResE>,
        I,
    >(
        &'a self,
        requests: Vec<ReqE>,
        invoke: I,
    ) -> Result<ResT>
    where
        I: for<'b> Fn(&'b AnyMetadata, Vec<ReqE>) -> BoxFuture<'b, Result<ResT>>,
    {
        enum State<'s> {
            Pending,
            AllMissing,
            AllSame(AnyMetadataWrapper<'s>),
        }

        let mut state = State::Pending;
        for (request_idx, entry) in requests.iter().enumerate() {
            let maybe_service = self.get_metadata_service_for_resource_location_v1alpha(entry.resource_location());
            if let Some(service) = maybe_service {
                let service = AnyMetadataWrapper(service);
                match state {
                    State::Pending => {
                        // Request 0 is handled by the service.
                        state = State::AllSame(service);
                    },
                    State::AllMissing => {
                        // Requests 0..request_idx are rejected. Request request_idx is forwarded to `service`.
                        // Requests request_idx.. are TBD.
                        debug!(
                            ?request_idx,
                            "route hit after continuous route misses, falling back to split-combine"
                        );

                        return self
                            .route_requests_by_resource_location_v1alpha_continue_with_split_combine(
                                requests.into_iter().enumerate().map(|(idx, entry)| {
                                    let maybe_service = if idx < request_idx {
                                        None
                                    } else if idx == request_idx {
                                        Some(service)
                                    } else {
                                        self.get_metadata_service_for_resource_location_v1alpha(
                                            entry.resource_location(),
                                        )
                                        .map(AnyMetadataWrapper)
                                    };

                                    (entry, maybe_service)
                                }),
                                invoke,
                            )
                            .await;
                    },
                    State::AllSame(prev_service) if prev_service != service => {
                        // Requests 0..request_idx are forwarded to `prev_service`. Request request_idx is
                        // forwarded to `service`. Request request_idx.. are TBD.
                        debug!(
                            ?request_idx,
                            "conflicting route hit after continuous route hits, falling back to split-combine"
                        );

                        return self
                            .route_requests_by_resource_location_v1alpha_continue_with_split_combine(
                                requests.into_iter().enumerate().map(|(idx, entry)| {
                                    let maybe_service = if idx < request_idx {
                                        Some(prev_service)
                                    } else if idx == request_idx {
                                        Some(service)
                                    } else {
                                        self.get_metadata_service_for_resource_location_v1alpha(
                                            entry.resource_location(),
                                        )
                                        .map(AnyMetadataWrapper)
                                    };

                                    (entry, maybe_service)
                                }),
                                invoke,
                            )
                            .await;
                    },
                    State::AllSame(prev_service) => {
                        // Requests 0..=request_idx are forwarded to `service`. Keep going.
                        debug_assert!(prev_service == service);
                        debug!(?request_idx, "contiuous route hits, continuing");
                    },
                }
            } else {
                match state {
                    State::Pending => {
                        // Request 0 is rejected.
                        state = State::AllMissing;
                    },
                    State::AllMissing => {
                        // Requests 0..=request_idx are rejected. Keep going.
                        debug!(?request_idx, "continuous route misses, continuing");
                    },
                    State::AllSame(prev_service) => {
                        // Requests 0..request_idx are forwarded to `prev_service`. Request request_idx is
                        // rejected. Requests request_idx.. are TBD.
                        debug!(
                            ?request_idx,
                            "route miss afer continuous route hits, falling back to split-combine"
                        );

                        return self
                            .route_requests_by_resource_location_v1alpha_continue_with_split_combine(
                                requests.into_iter().enumerate().map(|(idx, entry)| {
                                    let maybe_service = if idx < request_idx {
                                        Some(prev_service)
                                    } else if idx == request_idx {
                                        None
                                    } else {
                                        self.get_metadata_service_for_resource_location_v1alpha(
                                            entry.resource_location(),
                                        )
                                        .map(AnyMetadataWrapper)
                                    };

                                    (entry, maybe_service)
                                }),
                                invoke,
                            )
                            .await;
                    },
                }
            }
        }

        // Forward requests to the same service or reject them.
        let service = match state {
            State::Pending => {
                // The requests collection is empty.
                debug_assert!(requests.is_empty());
                debug!("request batch is empty");
                return Ok(ResT::from_entries([]));
            },
            State::AllMissing => {
                // All requests have been rejected.
                debug!("no routable requests were found");
                return Ok(ResT::from_entries((0..requests.len()).map(|request_index| {
                    ResE::from_error_entry(request_index as u32, create_resource_uri_not_supported_error_entry())
                })));
            },
            State::AllSame(service) => {
                // All requests to be forwarded to `service`.
                service
            },
        };

        debug!("forwarding all requests to the same backend");

        invoke(service.0, requests).await
    }

    /// Route requests to one or more metadata backends.
    async fn route_requests_by_resource_location_v1alpha_continue_with_split_combine<
        'a,
        ReqE: BatchRequestEntry,
        ResE: BatchResponseEntry,
        ResT: BatchResponse<ResponseEntry = ResE>,
        I,
    >(
        &'a self,
        requests: impl IntoIterator<Item = (ReqE, Option<AnyMetadataWrapper<'a>>)>,
        invoke: I,
    ) -> Result<ResT>
    where
        I: for<'b> Fn(&'b AnyMetadata, Vec<ReqE>) -> BoxFuture<'b, Result<ResT>>,
    {
        struct Assignment<T> {
            requests: Vec<T>,
            response_map: Vec<u32>,
        }

        let mut errors = Vec::new();
        let mut assignments: HashMap<AnyMetadataWrapper<'a>, Assignment<ReqE>> =
            HashMap::with_capacity(self.rules.len() / 2);
        for (request_index, (request, maybe_service)) in requests.into_iter().enumerate() {
            if let Some(service) = maybe_service {
                match assignments.entry(service) {
                    Entry::Vacant(entry) => {
                        entry.insert(Assignment {
                            requests: vec![request],
                            response_map: vec![request_index as u32],
                        });
                    },
                    Entry::Occupied(mut entry) => {
                        let assignments = entry.get_mut();
                        assignments.requests.push(request);
                        assignments.response_map.push(request_index as u32);
                    },
                }
            } else {
                errors.push(ResE::from_error_entry(
                    request_index as u32,
                    create_resource_uri_not_supported_error_entry(),
                ));
            }
        }

        debug_assert!(!assignments.is_empty());
        debug_assert!(assignments.len() + errors.len() > 1);

        debug!(
            errors_len = errors.len(),
            assignments_len = assignments.len(),
            "routing requests to multiple backends"
        );

        let results = try_join_all(assignments.into_iter().map(|(service, assignments)| {
            let response_map = assignments.response_map;
            invoke(service.0, assignments.requests).map_ok(move |mut response| {
                response.update_request_indexes(&response_map);
                response
            })
        }))
        .await?;

        debug!("combining responses");

        let mut response = ResT::from_entries(errors);
        for result in results.into_iter() {
            response.merge(result);
        }

        Ok(response)
    }
}

#[async_trait]
impl Metadata for Router {
    fn inner(&self) -> Option<&dyn Metadata> {
        None
    }

    async fn get_metadata_batch(&self, request: Request<GetMetadataBatchRequest>) -> Result<GetMetadataBatchResponse> {
        let (metadata, extensions, request) = request.into_parts();
        self.route_requests_by_resource_location_v1alpha(request.requests, |service, requests| {
            service.get_metadata_batch(Request::from_parts(
                metadata.clone(),
                extensions.clone(),
                GetMetadataBatchRequest { requests },
            ))
        })
        .await
    }

    fn query_metadata<'a: 'b, 'b>(
        &'a self,
        request: Request<QueryMetadataRequest>,
        options: StreamingOptions,
    ) -> BoxFuture<'b, Result<BoxStream<QueryMetadataResponsePage>>> {
        self.get_metadata_service_for_resource_location_v1alpha(request.get_ref().resource_location.as_ref())
            .map(|service| service.query_metadata(request, options))
            .unwrap_or_else(resource_uri_not_supported)
    }

    async fn update_metadata_batch(
        &self,
        request: Request<UpdateMetadataBatchRequest>,
    ) -> Result<UpdateMetadataBatchResponse> {
        let (metadata, extensions, request) = request.into_parts();
        self.route_requests_by_resource_location_v1alpha(request.requests, |service, requests| {
            service.update_metadata_batch(Request::from_parts(
                metadata.clone(),
                extensions.clone(),
                UpdateMetadataBatchRequest { requests },
            ))
        })
        .await
    }

    async fn delete_metadata_batch(
        &self,
        request: Request<DeleteMetadataBatchRequest>,
    ) -> Result<DeleteMetadataBatchResponse> {
        let (metadata, extensions, request) = request.into_parts();
        self.route_requests_by_resource_location_v1alpha(request.requests, |service, requests| {
            service.delete_metadata_batch(Request::from_parts(
                metadata.clone(),
                extensions.clone(),
                DeleteMetadataBatchRequest { requests },
            ))
        })
        .await
    }
}

#[track_caller]
fn resource_uri_not_supported<'a, T>() -> BoxFuture<'a, Result<T>> {
    Box::pin(async move { Err(error::ResourceUriNotSupported.build().into()) })
}

fn create_resource_uri_not_supported_error_entry() -> BatchErrorEntry {
    create_batch_error_entry_v1alpha(Code::InvalidArgument, "resource uri is not supported")
}

#[derive(Clone, Copy)]
struct AnyMetadataWrapper<'a>(&'a AnyMetadata);

impl std::hash::Hash for AnyMetadataWrapper<'_> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.0.inner).hash(state);
    }
}

impl std::cmp::PartialEq for AnyMetadataWrapper<'_> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0.inner, &other.0.inner)
    }
}

impl std::cmp::Eq for AnyMetadataWrapper<'_> {}

enum RequestLookupRule {
    FolderOrObject(AnyMetadata),
    Object(AnyMetadata),
}
