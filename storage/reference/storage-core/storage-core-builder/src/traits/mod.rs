// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Storage service traits, one per Storage API service and version.
//!
//! Each trait is declared with [`generate_storage_trait!`](crate::generate_storage_trait) and
//! mirrors a proto service. Every method takes `request: Request<FooRequest>` as its first
//! parameter after `&self` and returns `Result<FooResponse>` or `Result<BoxStream<...>>`, where
//! `FooRequest` and `FooResponse` are the messages of the RPC it serves. A method with no RPC
//! behind it still takes a [`Request`]: define its request and response structs beside the trait
//! and take `request: Request<ThatRequest>`.
//!
//! [`Request`] carries everything about the call that is not in the message. The caller's
//! metadata (credentials, tracing headers) is read from `request.metadata()`. Context that layers
//! derive from the call, such as additional headers, cache control or a cached resource info, is
//! inserted into and read from `request.extensions()`. A layer forwards the request it was given;
//! it never builds fresh metadata or extensions below the ingress.
//!
//! Keep it that way. A signature here is copied into every backend, layer and downstream
//! implementation, so a method that takes decomposed fields or a separate context parameter
//! cannot be corrected later without a breaking change, and a layer that is handed fields instead
//! of the request cannot forward what it was not given. If a method lacks context it needs,
//! change it to take the [`Request`]; never add a parameter for the missing piece. The former
//! `Versioning::is_versioning_enabled` took a bare address and later gained a `MetadataMap`
//! parameter for the context it lacked: every layer then hand-carried the pieces, and the
//! request's extensions still never reached the layers below.
//!
//! The only extra parameters come after the request and are not request context:
//! [`StreamingOptions`] for paginated methods, the body of `write` as a [`BoxFuture`], and the
//! version-id token of `enumerate_versions`. The macro enforces the parameter rules at compile
//! time; see [`StorageTraitRequest`] and [`StorageTraitExtraArg`].

pub mod capabilities;
pub mod error;
pub mod filefolder;
pub mod fileobject;
pub mod metadata;
pub mod versioning;

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;

use lazy_static::lazy_static;

pub use async_trait::async_trait;
pub use bytes::Bytes;
pub use futures::Stream;
pub use http::Extensions;
pub use std::future::Future;
pub use std::sync::{Arc, LazyLock, RwLock};
pub use tonic::metadata::{KeyAndValueRef, MetadataMap};
pub use tower::util::Either;

pub use error::{Result, SourceLocation, StorageError, StorageErrorExt, UnimplementedError};

/// A boxed stream of results.
pub type BoxStream<T> = Pin<Box<dyn Stream<Item = Result<T>> + Send + 'static>>;

/// A boxed future that returns a `Result<Bytes>`.
pub type BoxFuture<T> = Pin<Box<dyn Future<Output = Result<T>> + Send>>;

lazy_static! {
    static ref SENSITIVE_HEADERS: HashSet<String> = {
        let mut set = HashSet::from([
            "authorization",
            "proxy-authorization",
            "cookie",
            "set-cookie",
            "api-key",
            "x-api-key",
            "apikey",
            "x-amz-security-token",
            "x-amz-server-side-encryption-customer-key",
            "x-ms-authorization",
            "x-goog-iap-jwt-assertion",
            "x-goog-authenticated-user-email",
            "x-goog-authenticated-user-id",
        ].map(String::from));

        // Add additional headers from environment variable if present
        if let Ok(env_headers) = std::env::var("STORAGE_CORE_SENSITIVE_HEADERS") {
            for header in env_headers.split(',') {
                let trimmed = header.trim();
                if !trimmed.is_empty() {
                    set.insert(trimmed.to_lowercase());
                }
            }
        }

        set
    };
}

/// The request every storage trait method takes: the request message plus everything about the
/// call that is not in the message.
///
/// - `request.metadata()` is the caller's metadata (credentials, tracing headers) as it arrived
///   at the ingress.
/// - `request.extensions()` is per-request context that layers derive and hand down, such as
///   `AdditionalHeaders`, cache control or a cached resource info. A layer adds it with
///   `request.extensions_mut().insert(..)`; a later layer or the backend reads it with
///   `request.extensions().get::<T>()`.
///
/// The ingress (a gRPC or REST handler) builds the request once. Below it, a layer forwards the
/// request it was given, or builds a sub-request from that request's metadata and extensions with
/// `tonic::Request::from_parts(..).into()`.
///
/// A newtype over [`tonic::Request`] (reachable through `Deref`), so that `Debug` output masks
/// sensitive metadata.
pub struct Request<T>(pub tonic::Request<T>);

impl<T> Request<T> {
    /// Wraps `inner` with empty metadata and extensions, so it is only for code that originates a
    /// call with no caller behind it (tests, doc examples, in-process callers). The handlers do not
    /// use it: a gRPC handler converts the `tonic::Request` it received with `.into()`, and a REST
    /// handler builds one with `tonic::Request::from_parts(MetadataMap::from_headers(headers), ..)`.
    /// Below the ingress it would drop the caller's context.
    pub fn new(inner: T) -> Self {
        Request(tonic::Request::new(inner))
    }

    pub fn into_inner(self) -> T {
        self.0.into_inner()
    }
}

impl<T> Deref for Request<T> {
    type Target = tonic::Request<T>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for Request<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T> From<tonic::Request<T>> for Request<T> {
    fn from(request: tonic::Request<T>) -> Self {
        Request(request)
    }
}

impl<T: fmt::Debug> fmt::Debug for Request<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Collect metadata into a HashMap
        let mut metadata_map = HashMap::new();
        for key_and_value in self.metadata().iter() {
            match key_and_value {
                KeyAndValueRef::Ascii(key, value) => {
                    if value.is_sensitive() || SENSITIVE_HEADERS.contains(key.as_str()) {
                        metadata_map.insert(key.as_str(), "[REDACTED]".to_string());
                    } else {
                        metadata_map.insert(key.as_str(), value.to_str().unwrap_or("<invalid>").to_string());
                    }
                },
                KeyAndValueRef::Binary(key, value) => {
                    let key_str = key.as_str();
                    let base_key = key_str.strip_suffix("-bin").unwrap_or(key_str);

                    if value.is_sensitive() || SENSITIVE_HEADERS.contains(base_key) {
                        metadata_map.insert(key_str, "[REDACTED]".to_string());
                    } else {
                        metadata_map.insert(key_str, "<binary>".to_string());
                    }
                },
            }
        }

        f.debug_struct("Request")
            .field("message", self.get_ref())
            .field("metadata", &metadata_map)
            .field("extensions", &format_args!("Extensions {{ ... }}"))
            .finish()
    }
}

/// Request type. Using new type to define custom debug / display and mask sensitive data from metadata.
pub struct Response<T>(pub tonic::Response<T>);

impl<T> Response<T> {
    pub fn new(inner: T) -> Self {
        Response(tonic::Response::new(inner))
    }

    pub fn into_inner(self) -> T {
        self.0.into_inner()
    }
}

impl<T> Deref for Response<T> {
    type Target = tonic::Response<T>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for Response<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T> From<tonic::Response<T>> for Response<T> {
    fn from(response: tonic::Response<T>) -> Self {
        Response(response)
    }
}

impl<T: fmt::Debug> fmt::Debug for Response<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Collect metadata into a HashMap
        let mut metadata_map = HashMap::new();
        for key_and_value in self.metadata().iter() {
            match key_and_value {
                KeyAndValueRef::Ascii(key, value) => {
                    if value.is_sensitive() || SENSITIVE_HEADERS.contains(key.as_str()) {
                        metadata_map.insert(key.as_str(), "[REDACTED]".to_string());
                    } else {
                        metadata_map.insert(key.as_str(), value.to_str().unwrap_or("<invalid>").to_string());
                    }
                },
                KeyAndValueRef::Binary(key, value) => {
                    let key_str = key.as_str();
                    let base_key = key_str.strip_suffix("-bin").unwrap_or(key_str);

                    if value.is_sensitive() || SENSITIVE_HEADERS.contains(base_key) {
                        metadata_map.insert(key_str, "[REDACTED]".to_string());
                    } else {
                        metadata_map.insert(key_str, "<binary>".to_string());
                    }
                },
            }
        }

        f.debug_struct("Response")
            .field("message", self.get_ref())
            .field("metadata", &metadata_map)
            .field("extensions", &format_args!("Extensions {{ ... }}"))
            .finish()
    }
}

/// Pagination options for streaming responses.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Pagination {
    /// The continuation handle for the next page.
    pub continuation_handle: Option<String>,
    /// The maximum page size for the streaming response.
    pub max_page_size: Option<u32>,
}

/// Options for streaming responses.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum StreamingOptions {
    /// Paginated streaming with a continuation token and maximum page size.
    Paginated(Pagination),
    /// Streamed streaming without pagination.
    Streamed,
}

mod sealed {
    /// Private supertrait: downstream crates cannot implement [`super::StorageTraitRequest`]
    /// or [`super::StorageTraitExtraArg`], so widening the storage-trait signature policy
    /// requires an edit (and review) in this module.
    pub trait Sealed {}
}

/// Compile-time policy for the first parameter (after `&self`) of every
/// [`generate_storage_trait!`](crate::generate_storage_trait) method: it must be the whole request
/// wrapped in [`Request`]. That is the RPC's proto request message or, for a method with no RPC
/// behind it, a request struct defined beside the trait.
///
/// Do not decompose a request into individual fields, and do not pass per-request context
/// (`AdditionalHeaders`, `MetadataMap`, `Extensions`, ...) as separate parameters: caller
/// metadata is in `request.metadata()` and layer-derived context rides in `request.extensions()`,
/// so that every layer can read and forward it. A method that lacks context it needs is changed
/// to take the request, never given another parameter.
///
/// Such a request struct is accepted as is; it does not have to be a proto message:
/// ```
/// # use storage_core_builder::generate_storage_trait;
/// # use storage_core_builder::traits::{Request, Result};
/// pub struct ProbeRequest {
///     pub resource_address: String,
/// }
///
/// pub struct ProbeResponse {
///     pub reachable: bool,
/// }
///
/// generate_storage_trait! {
///     Probe {
///         async fn probe(&self, request: Request<ProbeRequest>) -> Result<ProbeResponse>;
///     }
/// }
/// ```
///
/// A method that takes a request's fields instead of the request does not compile (compare the
/// positive example on [`generate_storage_trait!`](crate::generate_storage_trait), which uses
/// the same proto types):
/// ```compile_fail,E0277
/// # use storage_core_builder::generate_storage_trait;
/// # use storage_core_builder::traits::Result;
/// # use storage_core_protos::nvidia::omniverse::storage::versioning::v1alpha::IsVersionedResponse;
/// generate_storage_trait! {
///     Versioning {
///         async fn is_versioned(&self, resource_address: &str) -> Result<IsVersionedResponse>;
///     }
/// }
/// ```
#[diagnostic::on_unimplemented(
    message = "storage trait methods must take `request: Request<_>` as their first parameter after `&self`, found `{Self}`",
    label = "expected `Request<SomeRequest>` here",
    note = "do not decompose the request into separate parameters; pass the whole request message wrapped in `storage_core_builder::traits::Request`",
    note = "caller metadata is in `request.metadata()` and per-request context (e.g. `AdditionalHeaders`) belongs in `request.extensions()`, not in the method signature"
)]
pub trait StorageTraitRequest: sealed::Sealed {}

impl<T> sealed::Sealed for Request<T> {}
impl<T> StorageTraitRequest for Request<T> {}

/// Compile-time allowlist of the parameter types a
/// [`generate_storage_trait!`](crate::generate_storage_trait) method may take *after* its
/// `request: Request<_>` parameter.
///
/// These are the long-standing out-of-band arguments that cannot ride in the request:
/// - [`StreamingOptions`]: pagination for streaming list/enumerate methods.
/// - [`BoxFuture<Bytes>`]: the deferred body of `FileObject::write`.
/// - `Option<String>`: **legacy**, only for `Versioning::enumerate_versions`'s
///   `version_id_token`. It is too generic to be a good allowlist entry (it would also admit,
///   say, an address string); prefer a dedicated newtype for any new out-of-band argument.
///
/// Anything else (`&str`, `String`, `AdditionalHeaders`, `MetadataMap`, `Extensions`, ...) is
/// rejected: request fields belong in the request message, per-request context belongs in
/// `request.extensions()` / `request.metadata()`.
///
/// Per-request context passed next to a proper request does not compile, whether it is
/// `AdditionalHeaders` (insert it with `request.extensions_mut().insert(..)` instead)...
/// ```compile_fail,E0277
/// # use storage_core_builder::generate_storage_trait;
/// # use storage_core_builder::traits::{Request, Result};
/// # use storage_core_protos::nvidia::omniverse::storage::versioning::v1alpha::{IsVersionedRequest, IsVersionedResponse};
/// # use storage_core_util::additional_headers::AdditionalHeaders;
/// generate_storage_trait! {
///     Versioning {
///         async fn is_versioned(
///             &self,
///             request: Request<IsVersionedRequest>,
///             additional_headers: Option<AdditionalHeaders>,
///         ) -> Result<IsVersionedResponse>;
///     }
/// }
/// ```
/// ...or caller metadata (it is already in `request.metadata()`):
/// ```compile_fail,E0277
/// # use storage_core_builder::generate_storage_trait;
/// # use storage_core_builder::traits::{MetadataMap, Request, Result};
/// # use storage_core_protos::nvidia::omniverse::storage::versioning::v1alpha::{IsVersionedRequest, IsVersionedResponse};
/// generate_storage_trait! {
///     Versioning {
///         async fn is_versioned(
///             &self,
///             request: Request<IsVersionedRequest>,
///             metadata: MetadataMap,
///         ) -> Result<IsVersionedResponse>;
///     }
/// }
/// ```
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not an allowed extra parameter on a storage trait method",
    label = "not allowed after `request: Request<_>`",
    note = "after `request: Request<_>` storage trait methods may only take `StreamingOptions` or `BoxFuture<Bytes>` (`Option<String>` is grandfathered for `enumerate_versions`'s `version_id_token` only; do not add new uses)",
    note = "request fields belong in the request message; per-request context (`AdditionalHeaders`, `MetadataMap`, `Extensions`) belongs in `request.extensions()` / `request.metadata()`"
)]
pub trait StorageTraitExtraArg: sealed::Sealed {}

impl sealed::Sealed for StreamingOptions {}
impl StorageTraitExtraArg for StreamingOptions {}
impl sealed::Sealed for BoxFuture<Bytes> {}
impl StorageTraitExtraArg for BoxFuture<Bytes> {}
impl sealed::Sealed for Option<String> {}
// Keep the legacy entry out of rustc's "the following other types implement trait" help so the
// compiler does not steer new code toward it.
#[diagnostic::do_not_recommend]
impl StorageTraitExtraArg for Option<String> {}

/// Used by [`generate_storage_trait!`](crate::generate_storage_trait); asserts the
/// request-parameter policy ([`StorageTraitRequest`]) at compile time.
#[doc(hidden)]
pub const fn __assert_storage_trait_request<T: StorageTraitRequest>() {}

/// Used by [`generate_storage_trait!`](crate::generate_storage_trait); asserts the
/// extra-parameter policy ([`StorageTraitExtraArg`]) at compile time.
#[doc(hidden)]
pub const fn __assert_storage_trait_extra_arg<T: StorageTraitExtraArg>() {}

/// Macro to generate storage service traits with default implementations.
///
/// This macro generates:
/// - The main trait with an `inner()` method and specified async methods
/// - Default implementations that delegate to inner or return UnimplementedError
/// - `Arc<T>` implementation that delegates to the inner type
/// - `Either<A, B>` implementation that matches and delegates
///
/// Method signature policy (the parameter rules are enforced at compile time):
/// - Each method mirrors one RPC: it takes the RPC's whole proto request message wrapped in
///   [`Request`] as its first parameter after `&self` (see [`StorageTraitRequest`]), and returns
///   the RPC's proto response, or a [`BoxStream`] of responses for a streaming RPC. A method with
///   no RPC behind it uses request and response structs defined beside the trait; its request is
///   still wrapped in [`Request`]. Never decompose a request into separate parameters.
/// - Any further parameters must be on the [`StorageTraitExtraArg`] allowlist. Per-request
///   context such as `AdditionalHeaders` is attached with `request.extensions_mut().insert(..)`
///   and read with `request.extensions().get::<T>()`; caller metadata stays in
///   `request.metadata()`. Neither is ever fabricated below the ingress. If a method lacks context
///   it needs, change it to take the [`Request`]; do not add a parameter.
///
/// Usage. This example is also the positive control for every `compile_fail` example on this
/// macro, [`StorageTraitRequest`] and [`StorageTraitExtraArg`]: it uses the same imports
/// (including `MetadataMap`) and proto types, and they differ from it only in the offending method
/// signature, so they fail because of the policy.
/// ```
/// use storage_core_builder::generate_storage_trait;
/// use storage_core_builder::traits::{async_trait, BoxStream, MetadataMap, Request, Result, StreamingOptions};
/// use storage_core_protos::nvidia::omniverse::storage::versioning::v1alpha::{
///     EnumerateVersionsRequest, EnumerateVersionsResponse, IsVersionedRequest, IsVersionedResponse,
/// };
/// use storage_core_util::additional_headers::AdditionalHeaders;
///
/// generate_storage_trait! {
///     Versioning {
///         // rpc IsVersioned(IsVersionedRequest) returns (IsVersionedResponse);
///         async fn is_versioned(&self, request: Request<IsVersionedRequest>) -> Result<IsVersionedResponse>;
///
///         // A server-streaming RPC; `StreamingOptions` is an allowed out-of-band argument.
///         async fn enumerate_versions(
///             &self,
///             request: Request<EnumerateVersionsRequest>,
///             streaming: StreamingOptions,
///         ) -> Result<BoxStream<EnumerateVersionsResponse>>;
///     }
/// }
///
/// /// A layer that attaches per-request context, as `AdditionalHeaderManager` does.
/// struct AttachHeaders<S> {
///     inner: S,
///     headers: AdditionalHeaders,
/// }
///
/// #[async_trait]
/// impl<S: Versioning> Versioning for AttachHeaders<S> {
///     fn inner(&self) -> Option<&dyn Versioning> {
///         Some(&self.inner)
///     }
///
///     async fn is_versioned(&self, mut request: Request<IsVersionedRequest>) -> Result<IsVersionedResponse> {
///         // Context rides in the request's extensions, never as an extra method parameter.
///         request.extensions_mut().insert(self.headers.clone());
///         self.inner.is_versioned(request).await
///     }
///
///     // `enumerate_versions` is not overridden: the generated default forwards it to `inner()`.
/// }
///
/// /// A backend that reads the context back out of the request it was handed.
/// struct Backend;
///
/// #[async_trait]
/// impl Versioning for Backend {
///     fn inner(&self) -> Option<&dyn Versioning> {
///         None
///     }
///
///     async fn is_versioned(&self, request: Request<IsVersionedRequest>) -> Result<IsVersionedResponse> {
///         // A real backend applies these headers and honours the caller metadata on its
///         // outbound storage call; this one only reports whether the headers arrived.
///         let _caller_metadata: &MetadataMap = request.metadata();
///         let headers = request.extensions().get::<AdditionalHeaders>();
///         Ok(IsVersionedResponse { is_versioned: headers.is_some() })
///     }
/// }
///
/// # tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(async {
/// let service = AttachHeaders { inner: Backend, headers: AdditionalHeaders::default() };
/// // Stands in for the ingress, which builds the request once (a gRPC handler from the tonic
/// // request it received, a REST handler from the HTTP headers); layers and backends only forward it.
/// let request = Request::new(IsVersionedRequest {
///     resource_address: "https://bucket.s3.us-west-2.amazonaws.com/a.txt".into(),
/// });
/// let response = service.is_versioned(request).await;
/// assert!(matches!(response, Ok(IsVersionedResponse { is_versioned: true })));
/// # });
/// ```
///
/// The former `Versioning::is_versioning_enabled` signatures, which this policy was written
/// against, do not compile. They decompose `IsVersionedRequest` into a bare address and leak
/// caller metadata and `AdditionalHeaders` into the method signature. The v1alpha one:
/// ```compile_fail,E0277
/// # use storage_core_builder::generate_storage_trait;
/// # use storage_core_builder::traits::{MetadataMap, Result};
/// # use storage_core_util::additional_headers::AdditionalHeaders;
/// generate_storage_trait! {
///     Versioning {
///         async fn is_versioning_enabled(
///             &self,
///             top_level_address: &str,
///             metadata: MetadataMap,
///             additional_headers: Option<AdditionalHeaders>,
///         ) -> Result<bool>;
///     }
/// }
/// ```
/// The v1beta one:
/// ```compile_fail,E0277
/// # use storage_core_builder::generate_storage_trait;
/// # use storage_core_builder::traits::Result;
/// # use storage_core_util::additional_headers::AdditionalHeaders;
/// generate_storage_trait! {
///     Versioning {
///         async fn is_versioning_enabled(
///             &self,
///             top_level_address: &str,
///             additional_headers: Option<AdditionalHeaders>,
///         ) -> Result<bool>;
///     }
/// }
/// ```
///
/// Stable rustdoc does not check the `E0277` code on `compile_fail` examples. The positive
/// control above, and mutation-testing the assertions in `generate_storage_trait_impl!`, are what
/// show these fail because of the signature policy.
#[macro_export]
macro_rules! generate_storage_trait {
    // Rule for methods without where clauses
    (
        $trait_name:ident {
            $(
                async fn $method_name:ident$(<$($generic:tt),*>)?(
                    &self, // every storage trait method must take `request: Request<_>` right after `&self`
                    $request_name:ident: $request_type:ty
                    $(, $param_name:ident: $param_type:ty)*
                    $(,)?
                ) -> $return_type:ty;
            )*
        }
    ) => {
        $crate::generate_storage_trait_impl! {
            $trait_name {
                $(
                    async fn $method_name$(<$($generic),*>)?(
                        &self,
                        $request_name: $request_type
                        $(, $param_name: $param_type)*
                    ) -> $return_type;
                )*
            }
        }
    };

    // Rule for methods with where clauses
    (
        $trait_name:ident {
            $(
                async fn $method_name:ident$(<$($generic:tt),*>)?(
                    &self, // every storage trait method must take `request: Request<_>` right after `&self`
                    $request_name:ident: $request_type:ty
                    $(, $param_name:ident: $param_type:ty)*
                    $(,)?
                ) -> $return_type:ty
                where $($constraint:tt)*;
            )*
        }
    ) => {
        $crate::generate_storage_trait_impl! {
            $trait_name {
                $(
                    async fn $method_name$(<$($generic),*>)?(
                        &self,
                        $request_name: $request_type
                        $(, $param_name: $param_type)*
                    ) -> $return_type
                    where $($constraint)*;
                )*
            }
        }
    };
}

// Implementation detail of `generate_storage_trait!`; not public API. It enforces the same
// signature policy, so calling it directly is not a way around the checks.
#[doc(hidden)]
#[macro_export]
macro_rules! generate_storage_trait_impl {
    (
        $trait_name:ident {
            $(
                async fn $method_name:ident$(<$($generic:tt),*>)?(
                    &self,
                    $request_name:ident: $request_type:ty
                    $(, $param_name:ident: $param_type:ty)*
                ) -> $return_type:ty
                $(where $($where_clause:tt)*)?;
            )*
        }
    ) => {
        // Compile-time signature policy; see `StorageTraitRequest` / `StorageTraitExtraArg`.
        // One never-called fn per method, mirroring its generics so the bounds are checked in
        // the method's own scope. Errors point at the offending parameter type.
        const _: () = {
            $(
                #[allow(dead_code)]
                fn $method_name$(<$($generic),*>)?()
                $(where $($where_clause)*)?
                {
                    $crate::traits::__assert_storage_trait_request::<$request_type>();
                    $( $crate::traits::__assert_storage_trait_extra_arg::<$param_type>(); )*
                }
            )*
        };

        /// Implements the storage service API.
        #[$crate::traits::async_trait]
        pub trait $trait_name
        where
            Self: std::marker::Send + std::marker::Sync + 'static,
        {
            /// Returns the inner trait implementation if it exists.
            /// This is used to provide a fallback implementation.
            /// If the inner implementation is not available, methods will return an
            /// `UnimplementedError`, otherwise they will delegate to the inner implementation.
            fn inner(&self) -> Option<&dyn $trait_name>;

            fn backend(&self) -> &'static str {
                "unknown"
            }

            $(
                /// Generated method implementation.
                async fn $method_name$(<$($generic),*>)?(
                    &self,
                    $request_name: $request_type
                    $(, $param_name: $param_type)*
                ) -> $return_type
                $(where $($where_clause)*)?
                {
                    if let Some(inner) = self.inner() {
                        inner.$method_name($request_name $(, $param_name)*).await
                    } else {
                        Err(Box::new($crate::traits::error::UnimplementedError::new()))
                    }
                }
            )*
        }

        /// Implement the trait for `Arc<T>` where `T` implements the trait.
        ///
        /// The `?Sized` relaxation allows `Arc<dyn $trait_name>` — required
        /// for users who want to type-erase a layer chain (see `BoxFileObject`
        /// and friends) so that nested generics don't blow up compile-time
        /// memory.
        #[$crate::traits::async_trait]
        impl<T> $trait_name for std::sync::Arc<T>
        where
            T: $trait_name + ?Sized,
        {
            fn inner(&self) -> Option<&dyn $trait_name> {
                (**self).inner()
            }

            fn backend(&self) -> &'static str {
                (**self).backend()
            }

            $(
                async fn $method_name$(<$($generic),*>)?(
                    &self,
                    $request_name: $request_type
                    $(, $param_name: $param_type)*
                ) -> $return_type
                $(where $($where_clause)*)?
                {
                    (**self).$method_name($request_name $(, $param_name)*).await
                }
            )*
        }

        /// Implement the trait for `Either<A, B>` where both `A` and `B` implement the trait.
        #[$crate::traits::async_trait]
        impl<A, B> $trait_name for $crate::traits::Either<A, B>
        where
            A: $trait_name,
            B: $trait_name,
        {
            fn inner(&self) -> Option<&dyn $trait_name> {
                match self {
                    $crate::traits::Either::Left(a) => a.inner(),
                    $crate::traits::Either::Right(b) => b.inner(),
                }
            }

            fn backend(&self) -> &'static str {
                match self {
                    $crate::traits::Either::Left(a) => a.backend(),
                    $crate::traits::Either::Right(b) => b.backend(),
                }
            }

            $(
                async fn $method_name$(<$($generic),*>)?(
                    &self,
                    $request_name: $request_type
                    $(, $param_name: $param_type)*
                ) -> $return_type
                $(where $($where_clause)*)?
                {
                    match self {
                        $crate::traits::Either::Left(a) => a.$method_name($request_name $(, $param_name)*).await,
                        $crate::traits::Either::Right(b) => b.$method_name($request_name $(, $param_name)*).await,
                    }
                }
            )*
        }
    };
}
