// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use axum::{
    body::Body,
    extract::{self, State},
    response::Response as AxumResponse,
    routing::*,
};
use http::{HeaderMap, StatusCode};
use serde::{Deserialize, Serialize};
use storage_core::ingress::{OpenAPI, ToGrpcService, ToRestService};
use storage_core::traits::{async_trait, Extensions, MetadataMap};
use storage_core_util::error_limited;
use tonic::Status;
use utoipa::{OpenApi, ToSchema};

use super::proto::helloworld::{
    greeter_server::{Greeter as GreeterService, GreeterServer},
    HelloReply, HelloRequest, FILE_DESCRIPTOR_SET,
};
use super::service::Greeter;
use super::traits::Greeter as GreeterTrait;

/// The ToGrpcService trait is how the storage-core library knows how to take your Service and actually
/// add it to the underlying Tonic server.
impl<L> ToGrpcService for Greeter<L>
where
    L: GreeterTrait,
{
    type Service = GreeterServer<Greeter<L>>;

    /// This method returns the actual Tonic service that will be used to handle the requests.
    fn service(&self) -> Self::Service {
        GreeterServer::new(self.clone())
    }

    /// This method returns the file descriptor set for the Greeter Service.
    /// If you return a file descriptor set, the storage-core library will add it to the reflection service.
    /// If None is returned, the Greeter Service will not be added to the reflection service.
    fn file_descriptor_set(&self) -> Option<&[u8]> {
        Some(FILE_DESCRIPTOR_SET)
    }
}

/// Tonic generates a trait for each service that is defined in the proto file. This code here is a shim
/// layer that implements the Tonic trait using the trait we've defined in our project.
#[async_trait]
impl<T: GreeterTrait> GreeterService for Greeter<T> {
    async fn say_hello(&self, request: tonic::Request<HelloRequest>) -> Result<tonic::Response<HelloReply>, Status> {
        Ok(tonic::Response::new(
            self.greeter
                .say_hello(request.into())
                .await
                .map_err(|e| e.tonic_status())?,
        ))
    }
}

/// The ToRestService trait is how the storage-core library knows how to take your Service and actually
/// add it to the underlying Axum server.
impl<L> ToRestService for Greeter<L>
where
    L: GreeterTrait,
{
    /// This method returns the actual Axum router that will be used to handle the requests.
    fn service(&self) -> axum::Router {
        axum::Router::new()
            .route("/greeter/say_hello", post(say_hello))
            .with_state(self.clone())
    }

    /// This method returns the OpenAPI specification for the Greeter Service.
    /// If you return an OpenAPI specification, the storage-core library will add it to the OpenAPI service.
    /// If None is returned, the Greeter Service will not be added to the OpenAPI service.
    fn openapi(&self) -> Option<OpenAPI> {
        Some(OpenAPI::new(
            "greeter",
            "/greeter/openapi.json",
            false,
            ApiDoc::openapi(),
        ))
    }
}

#[utoipa::path(
    post,
    path = "/greeter/say_hello",
    request_body = HelloBody,
    responses(
        (status = 200, description = "Hello reply", body = HelloResponse)
    )
)]

/// The say_hello function is the actual handler for the say_hello endpoint.
/// It marshalls the request and response into the appropriate formats for the Greeter Service.
async fn say_hello<S: GreeterTrait>(
    headers: HeaderMap,
    State(state): State<Greeter<S>>,
    extract::Json(payload): extract::Json<HelloBody>,
) -> Result<AxumResponse, StatusCode> {
    let request = HelloRequest { name: payload.name };
    let result = state
        .greeter
        .say_hello(tonic::Request::from_parts(MetadataMap::from_headers(headers), Extensions::new(), request).into())
        .await;

    match result {
        Ok(response) => Ok(AxumResponse::builder()
            .status(StatusCode::OK)
            .body(Body::from(
                serde_json::to_string(&HelloResponse {
                    message: response.message,
                })
                .map_err(|e| {
                    error_limited!(?e, "Failed to serialize say hello response");
                    StatusCode::INTERNAL_SERVER_ERROR
                })?,
            ))
            .map_err(|e| {
                error_limited!(?e, "Failed to build say hello response");
                StatusCode::INTERNAL_SERVER_ERROR
            })?),
        Err(e) => Err(e.http_status()),
    }
}

/// The HelloBody struct is the request body for the say_hello endpoint.
/// It is used to deserialize the request body from the HTTP request.
#[derive(Deserialize, Serialize, ToSchema)]
pub struct HelloBody {
    pub name: String,
}

/// The HelloResponse struct is the response body for the say_hello endpoint.
/// It is used to serialize the response body to the HTTP response.
#[derive(Deserialize, Serialize, ToSchema)]
pub struct HelloResponse {
    pub message: String,
}

/// The ApiDoc struct is used to generate the OpenAPI specification for the Greeter Service.
/// It is used to add the Greeter Service to the OpenAPI service.
#[derive(OpenApi)]
#[openapi(
    paths(say_hello),
    components(schemas(HelloBody, HelloResponse)),
    tags(
        (name = "greeter", description = "Greeter API")
    )
)]
pub struct ApiDoc;
