// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

pub mod capabilities;
pub mod error;
pub mod filefolder;
pub mod fileobject;
pub mod metadata;
mod rest_utils;
pub(crate) mod slow_stream;
pub mod versioning;

use tower::{layer::LayerFn, util::Either};

use crate::layer::{Identity, Layer, Stack};
use std::{marker::PhantomData, sync::atomic::AtomicU64};

static REQUEST_ID: AtomicU64 = AtomicU64::new(0);

/// A builder for constructing backend services with composable middleware layers.
///
/// `ServiceBuilder` implements the builder pattern for creating layered backend services.
/// It allows you to compose multiple middleware layers (authentication, logging, rate limiting, etc.)
/// around a core backend implementation before finalizing the service.
///
/// The builder uses a type-level stack to track the layer composition, ensuring type safety
/// while allowing flexible layer ordering and composition.
///
/// # Type Parameters
///
/// * `L` - The current layer stack type (starts with [`Identity`], grows with [`Stack`])
/// * `T` - The target service type that will be created from the layered backend
///
/// # Order
///
/// The order in which layers are added impacts how requests are handled. Layers
/// that are added first will be called with the request first. The argument to
/// [`backend`](Self::backend) will be last to see the request.
///
/// # Example
///
/// ```rust,ignore
/// use std::sync::Arc;
/// use storage_core::{
///     api_services::fileobject::v1alpha::{BoxFileObjectBuilder, FileObjectBuilder},
///     layer::LayerArc,
///     metrics::{OperationMetricsLayer, SdkMetricsLayer},
///     list_cache::ListCacheLayer,
///     stat_cache::StatCacheLayer,
///     egress_throttle::EgressThrottleLayer,
/// };
/// use storage_core_util::config;
///
/// // Build a file object service with multiple layers
/// let figment = config::generate_figment(&config_args)?;
/// let backend = Arc::new(S3Storage::new(&figment, &secrets)?);
///
/// let list_cache_layer = ListCacheLayer::from_figment(&figment)?
///     .map(|l| LayerArc(Arc::new(l)));
/// let stat_cache_layer = StatCacheLayer::from_figment(&figment)?
///     .map(|l| LayerArc(Arc::new(l)));
/// let egress_throttle_layer = EgressThrottleLayer::from_figment(&figment)?
///     .map(|l| LayerArc(Arc::new(l)));
/// let operation_metrics_layer = LayerArc(Arc::new(OperationMetricsLayer::new("s3")));
/// let sdk_metrics_layer = LayerArc(Arc::new(SdkMetricsLayer::new("s3")));
///
/// let service = FileObjectBuilder::new()
///     .layer(operation_metrics_layer.clone())
///     .option_layer(list_cache_layer.clone())
///     .option_layer(stat_cache_layer.clone())
///     .option_layer(egress_throttle_layer.clone())
///     .layer(sdk_metrics_layer.clone())
///     .backend(backend.clone());
///
/// // Request flow: OperationMetricsLayer -> ListCacheLayer -> StatCacheLayer -> EgressThrottleLayer -> SdkMetricsLayer -> S3Storage
///
/// // Erase only the final service type, similar to Tower's boxed services.
/// let service = FileObjectBuilder::new()
///     .layer(operation_metrics_layer.clone())
///     .layer(sdk_metrics_layer.clone())
///     .boxed(backend.clone());
///
/// // Erase after every layer when a very deep stack causes compile-time memory
/// // pressure. Request flow is still operation metrics -> SDK metrics -> backend.
/// let service = BoxFileObjectBuilder::new(backend.clone())
///     .layer(operation_metrics_layer.clone())
///     .layer(sdk_metrics_layer.clone())
///     .build();
/// ```
pub struct ServiceBuilder<L, T> {
    layer: L,
    _phantom: PhantomData<T>,
}

/// A generated service wrapper that can erase its backend implementation.
///
/// This is the storage API equivalent of Tower's boxed service helpers: it keeps
/// the normal static layer stack available by default, while allowing callers to
/// explicitly erase the final concrete service type when compile-time type
/// pressure matters more than avoiding dynamic dispatch.
pub trait BoxedServiceTarget {
    type Boxed;

    fn into_boxed_service(self) -> Self::Boxed;
}

impl<T> ServiceBuilder<Identity, T> {
    /// Create a new [`ServiceBuilder`].
    pub fn new() -> Self {
        Self {
            layer: Identity::new(),
            _phantom: PhantomData,
        }
    }
}

impl<T> Default for ServiceBuilder<Identity, T> {
    /// Creates a default `ServiceBuilder` instance.
    ///
    /// This is equivalent to calling [`ServiceBuilder::new()`].
    fn default() -> Self {
        Self::new()
    }
}

impl<L, T> ServiceBuilder<L, T> {
    /// Add a new layer to the [`ServiceBuilder`].
    ///
    /// The provided layer must implement the [`Layer`] trait. See the [type-level
    /// documentation](Self#order) for the order in which layers are called.
    pub fn layer<U>(self, layer: U) -> ServiceBuilder<Stack<U, L>, T>
    where
        U: Layer<()>,
    {
        ServiceBuilder {
            layer: Stack::new(layer, self.layer),
            _phantom: PhantomData,
        }
    }

    /// Optionally add a new layer to the [`ServiceBuilder`].
    ///
    /// If `Some`, the layer is added. If `None`, an [`Identity`] layer is used in its place.
    pub fn option_layer<U>(self, layer: Option<U>) -> ServiceBuilder<Stack<Either<U, Identity>, L>, T>
    where
        U: Layer<()>,
    {
        self.layer(tower::util::option_layer(layer))
    }

    /// Add a [`Layer`] built from a function.
    pub fn layer_fn<F>(self, f: F) -> ServiceBuilder<Stack<LayerFn<F>, L>, T>
    where
        F: Fn(()) + Send + Sync + 'static,
    {
        self.layer(tower::layer::layer_fn(f))
    }

    /// Returns the underlying [`Layer`] implementation.
    pub fn into_inner(self) -> L {
        self.layer
    }

    /// Wrap the backend with the middleware provided by this [`ServiceBuilder`]'s
    /// layers, returning the final service of type `T`.
    pub fn backend<B>(self, backend: B) -> T
    where
        L: Layer<B>,
        T: From<L::Service>,
    {
        let layered_backend = self.layer.layer(backend);
        T::from(layered_backend)
    }

    /// Wrap the backend with this builder's layers, then erase the resulting
    /// service implementation behind a trait object.
    ///
    /// This preserves the same layer ordering as [`backend`](Self::backend):
    /// the first layer added is the first layer to observe a request. The tradeoff
    /// is the usual boxed-service tradeoff: lower compile-time type pressure and a
    /// nameable output type in exchange for runtime dynamic dispatch.
    pub fn boxed<B>(self, backend: B) -> T::Boxed
    where
        L: Layer<B>,
        T: From<L::Service> + BoxedServiceTarget,
    {
        self.backend(backend).into_boxed_service()
    }
}

/// Macro to generate service wrapper modules with the standard pattern.
///
/// This macro generates:
/// - A type alias for the builder
/// - A wrapper struct with Arc-wrapped service
/// - Standard implementations (new, From, Clone, ServiceInfoProvider)
///
/// # Usage
///
/// ```rust,ignore
/// service_wrapper_module! {
///     pub mod v1alpha {
///         service_name: "fileobject",
///         version: "v1alpha",
///         struct_name: FileObject,
///         field_name: file_object,
///         trait_path: crate::traits::fileobject::v1alpha::FileObject as FileObjectTrait,
///     }
/// }
/// ```
#[macro_export]
macro_rules! service_wrapper_module {
    (
        service_name: $service_name:literal,
        version: $version:literal,
        struct_name: $struct_name:ident,
        field_name: $field_name:ident,
        trait_path: $trait_path:path as $trait_alias:ident,
    ) => {
        use std::sync::Arc;
        use $crate::api_services::ServiceBuilder;
        use $crate::layer::Layer;
        use $crate::service_registry::{ServiceInfo, ServiceInfoProvider};
        use $trait_path as $trait_alias;

        /// A wrapper around a `$struct_name` service.
        ///
        /// This struct provides a shared reference to a `$struct_name` service.
        ///
        /// # Fields
        ///
        /// * `$field_name` - An [`Arc`]-wrapped `$struct_name` service
        pub struct $struct_name<T: $trait_alias> {
            pub $field_name: Arc<T>,
        }

        impl<T: $trait_alias> $struct_name<T> {
            pub fn new($field_name: T) -> Self {
                Self {
                    $field_name: Arc::new($field_name),
                }
            }
        }

        // Implement From to work with the generic BackendBuilder
        impl<T: $trait_alias> From<T> for $struct_name<T> {
            fn from($field_name: T) -> Self {
                Self::new($field_name)
            }
        }

        impl<T: $trait_alias> Clone for $struct_name<T> {
            fn clone(&self) -> Self {
                Self {
                    $field_name: self.$field_name.clone(),
                }
            }
        }

        impl<T: $trait_alias> $trait_alias for $struct_name<T> {
            fn inner(&self) -> Option<&dyn $trait_alias> {
                Some(&*self.$field_name)
            }

            fn backend(&self) -> &'static str {
                self.$field_name.backend()
            }
        }

        impl<T: $trait_alias> ServiceInfoProvider for $struct_name<T> {
            fn service_info(&self) -> Option<ServiceInfo> {
                Some(ServiceInfo {
                    name: $service_name.to_string(),
                    version: $version.to_string(),
                })
            }
        }

        paste::paste! {
            /// A builder for creating `$struct_name` services.
            ///
            /// This type alias allows for easy construction of `$struct_name` services
            /// with a backend layer and a service implementation.
            pub type [<$struct_name Builder>]<L, B> = ServiceBuilder<L, $struct_name<<L as Layer<B>>::Service>>;

            /// A `$struct_name` service with its backend erased to a trait object.
            ///
            /// This is the output of [`ServiceBuilder::boxed`]. It keeps service
            /// registration type names small by storing the implementation behind
            /// `Arc<dyn Trait>`.
            pub type [<Box $struct_name>] = $struct_name<Arc<dyn $trait_alias>>;

            impl<T: $trait_alias> $crate::api_services::BoxedServiceTarget for $struct_name<T> {
                type Boxed = [<Box $struct_name>];

                fn into_boxed_service(self) -> Self::Boxed {
                    let inner: Arc<dyn $trait_alias> = self.$field_name;
                    $struct_name::new(inner)
                }
            }

            trait [<$struct_name ErasedLayer>]: Send + Sync {
                fn layer(&self, inner: Arc<dyn $trait_alias>) -> Arc<dyn $trait_alias>;
            }

            struct [<$struct_name ErasedLayerImpl>]<L> {
                layer: L,
            }

            impl<L> [<$struct_name ErasedLayer>] for [<$struct_name ErasedLayerImpl>]<L>
            where
                L: Layer<[<Box $struct_name Builder>]> + Send + Sync + 'static,
                L::Service: $trait_alias,
            {
                fn layer(&self, inner: Arc<dyn $trait_alias>) -> Arc<dyn $trait_alias> {
                    Arc::new(self.layer.layer([<Box $struct_name Builder>]::from_arc(inner)))
                }
            }

            /// A builder that eagerly erases the service after every layer.
            ///
            /// This avoids carrying deeply nested `Stack<...>` service types through
            /// every generated API method body. Layers are still called in the order
            /// they are added, matching [`ServiceBuilder`]; each intermediate service
            /// is boxed behind `Arc<dyn Trait>`, so this path trades runtime dynamic
            /// dispatch for lower compile-time memory pressure.
            #[derive(Clone)]
            pub struct [<Box $struct_name Builder>] {
                inner: Arc<dyn $trait_alias>,
                layers: Vec<Arc<dyn [<$struct_name ErasedLayer>]>>,
            }

            impl [<Box $struct_name Builder>] {
                pub fn new<T>(inner: T) -> Self
                where
                    T: $trait_alias,
                {
                    Self {
                        inner: Arc::new(inner),
                        layers: Vec::new(),
                    }
                }

                pub fn from_arc(inner: Arc<dyn $trait_alias>) -> Self {
                    Self {
                        inner,
                        layers: Vec::new(),
                    }
                }

                pub fn layer<L>(mut self, layer: L) -> Self
                where
                    L: Layer<Self> + Send + Sync + 'static,
                    L::Service: $trait_alias,
                {
                    self.layers.push(Arc::new([<$struct_name ErasedLayerImpl>] { layer }));
                    self
                }

                pub fn option_layer<L>(self, layer: Option<L>) -> Self
                where
                    L: Layer<Self> + Send + Sync + 'static,
                    L::Service: $trait_alias,
                {
                    match layer {
                        Some(layer) => self.layer(layer),
                        None => self,
                    }
                }

                pub fn build(self) -> [<Box $struct_name>] {
                    let mut inner = self.inner;
                    for layer in self.layers.into_iter().rev() {
                        inner = layer.layer(inner);
                    }
                    $struct_name::new(inner)
                }
            }

            impl $trait_alias for [<Box $struct_name Builder>] {
                fn inner(&self) -> Option<&dyn $trait_alias> {
                    Some(&*self.inner)
                }

                fn backend(&self) -> &'static str {
                    self.inner.backend()
                }
            }
        }
    };
}

/// Generate a per-layer boxed builder for a service that must preserve two
/// service traits across erasure.
///
/// Most services can use the single-trait `Box*Builder` generated by
/// [`service_wrapper_module!`]. Layers such as `ListCacheLayer` need an erased
/// value that still implements both `FileObject` and `FileFolder`; this macro
/// centralizes that pattern so future cross-trait combinations do not require
/// bespoke handwritten builders.
#[macro_export]
macro_rules! composite_service_builder {
    (
        composite_trait: $composite_trait:ident,
        builder: $builder:ident,
        alias: $alias:ident,
        erased_layer: $erased_layer:ident,
        erased_layer_impl: $erased_layer_impl:ident,
        parts: $parts:ident,
        output: $output:ident,
        boxed_output: $boxed_output:ident,
        primary_trait: $primary_trait:ident,
        secondary_trait: $secondary_trait_path:path as $secondary_trait:ident,
    ) => {
        use $secondary_trait_path as $secondary_trait;

        /// Composite capability used when a boxed service must implement both
        /// service traits after type erasure.
        pub trait $composite_trait: $primary_trait + $secondary_trait {}

        impl<T> $composite_trait for T where T: $primary_trait + $secondary_trait + ?Sized {}

        /// A per-layer-erased builder for layers that require this composite
        /// service capability.
        #[derive(Clone)]
        pub struct $builder {
            inner: std::sync::Arc<dyn $composite_trait>,
            layers: Vec<std::sync::Arc<dyn $erased_layer>>,
        }

        /// Backwards-friendly short name for the composite boxed builder.
        pub type $alias = $builder;

        trait $erased_layer: Send + Sync {
            fn layer(&self, inner: std::sync::Arc<dyn $composite_trait>) -> std::sync::Arc<dyn $composite_trait>;
        }

        struct $erased_layer_impl<L> {
            layer: L,
        }

        impl<L> $erased_layer for $erased_layer_impl<L>
        where
            L: $crate::layer::Layer<$builder> + Send + Sync + 'static,
            L::Service: $primary_trait + $secondary_trait,
        {
            fn layer(&self, inner: std::sync::Arc<dyn $composite_trait>) -> std::sync::Arc<dyn $composite_trait> {
                std::sync::Arc::new(self.layer.layer($builder::from_arc(inner)))
            }
        }

        struct $parts {
            primary: std::sync::Arc<dyn $primary_trait>,
            secondary: std::sync::Arc<dyn $secondary_trait>,
        }

        impl $primary_trait for $parts {
            fn inner(&self) -> Option<&dyn $primary_trait> {
                Some(&*self.primary)
            }

            fn backend(&self) -> &'static str {
                self.primary.backend()
            }
        }

        impl $secondary_trait for $parts {
            fn inner(&self) -> Option<&dyn $secondary_trait> {
                Some(&*self.secondary)
            }

            fn backend(&self) -> &'static str {
                self.secondary.backend()
            }
        }

        impl $builder {
            /// Create a composite boxed builder from a service that implements
            /// both required traits.
            pub fn new<T>(service: T) -> Self
            where
                T: $primary_trait + $secondary_trait,
            {
                Self {
                    inner: std::sync::Arc::new(service),
                    layers: Vec::new(),
                }
            }

            /// Create a composite boxed builder from an existing composite trait
            /// object.
            pub fn from_arc(inner: std::sync::Arc<dyn $composite_trait>) -> Self {
                Self {
                    inner,
                    layers: Vec::new(),
                }
            }

            /// Create a composite boxed builder from separate erased trait
            /// objects. Prefer [`new`](Self::new) or [`from_arc`](Self::from_arc)
            /// when both traits come from the same service.
            pub fn from_arcs(
                primary: std::sync::Arc<dyn $primary_trait>,
                secondary: std::sync::Arc<dyn $secondary_trait>,
            ) -> Self {
                Self {
                    inner: std::sync::Arc::new($parts { primary, secondary }),
                    layers: Vec::new(),
                }
            }

            /// Add a layer while preserving source-order request flow.
            pub fn layer<L>(mut self, layer: L) -> Self
            where
                L: $crate::layer::Layer<Self> + Send + Sync + 'static,
                L::Service: $primary_trait + $secondary_trait,
            {
                self.layers
                    .push(std::sync::Arc::new($erased_layer_impl { layer }));
                self
            }

            /// Optionally add a layer while preserving source-order request flow.
            pub fn option_layer<L>(self, layer: Option<L>) -> Self
            where
                L: $crate::layer::Layer<Self> + Send + Sync + 'static,
                L::Service: $primary_trait + $secondary_trait,
            {
                match layer {
                    Some(layer) => self.layer(layer),
                    None => self,
                }
            }

            /// Build the primary boxed service.
            ///
            /// Layers are applied in reverse construction order so request flow
            /// matches `ServiceBuilder`: first written, first called.
            pub fn build(self) -> $boxed_output {
                let mut inner = self.inner;
                for layer in self.layers.into_iter().rev() {
                    inner = layer.layer(inner);
                }

                let primary: std::sync::Arc<dyn $primary_trait> = inner;
                $output::new(primary)
            }
        }

        impl $primary_trait for $builder {
            fn inner(&self) -> Option<&dyn $primary_trait> {
                Some(&*self.inner)
            }

            fn backend(&self) -> &'static str {
                let inner: &dyn $primary_trait = &*self.inner;
                inner.backend()
            }
        }

        impl $secondary_trait for $builder {
            fn inner(&self) -> Option<&dyn $secondary_trait> {
                Some(&*self.inner)
            }

            fn backend(&self) -> &'static str {
                let inner: &dyn $secondary_trait = &*self.inner;
                inner.backend()
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Service interface used to verify layer ordering: each implementer pushes
    /// its tag onto the supplied vector when called.
    trait Recorder {
        fn record(&self, into: &mut Vec<&'static str>);
    }

    struct Backend;
    impl Recorder for Backend {
        fn record(&self, into: &mut Vec<&'static str>) {
            into.push("backend");
        }
    }

    struct TaggedLayer {
        tag: &'static str,
    }

    impl<S> Layer<S> for TaggedLayer {
        type Service = TaggedService<S>;

        fn layer(&self, inner: S) -> Self::Service {
            TaggedService { tag: self.tag, inner }
        }
    }

    struct TaggedService<S> {
        tag: &'static str,
        inner: S,
    }

    impl<S: Recorder> Recorder for TaggedService<S> {
        fn record(&self, into: &mut Vec<&'static str>) {
            into.push(self.tag);
            self.inner.record(into);
        }
    }

    // `option_layer(Some(_))` produces an `Either::Left`; `option_layer(None)`
    // produces an `Either::Right` (an `Identity`-wrapped inner). Forward `record`
    // through whichever variant is active.
    impl<A: Recorder, B: Recorder> Recorder for Either<A, B> {
        fn record(&self, into: &mut Vec<&'static str>) {
            match self {
                Either::Left(a) => a.record(into),
                Either::Right(b) => b.record(into),
            }
        }
    }

    struct TestWrapper<S>(S);
    impl<S> From<S> for TestWrapper<S> {
        fn from(s: S) -> Self {
            TestWrapper(s)
        }
    }

    #[test]
    fn layers_are_called_in_the_order_they_were_added() {
        let service: TestWrapper<_> = ServiceBuilder::new()
            .layer(TaggedLayer { tag: "first" })
            .layer(TaggedLayer { tag: "second" })
            .layer(TaggedLayer { tag: "third" })
            .backend(Backend);

        let mut order = Vec::new();
        service.0.record(&mut order);

        assert_eq!(order, vec!["first", "second", "third", "backend"]);
    }

    #[test]
    fn option_layer_some_is_included_in_order() {
        let service: TestWrapper<_> = ServiceBuilder::new()
            .layer(TaggedLayer { tag: "outer" })
            .option_layer(Some(TaggedLayer { tag: "middle" }))
            .layer(TaggedLayer { tag: "inner" })
            .backend(Backend);

        let mut order = Vec::new();
        service.0.record(&mut order);

        assert_eq!(order, vec!["outer", "middle", "inner", "backend"]);
    }

    #[test]
    fn option_layer_none_is_skipped_in_order() {
        let service: TestWrapper<_> = ServiceBuilder::new()
            .layer(TaggedLayer { tag: "outer" })
            .option_layer(Option::<TaggedLayer>::None)
            .layer(TaggedLayer { tag: "inner" })
            .backend(Backend);

        let mut order = Vec::new();
        service.0.record(&mut order);

        assert_eq!(order, vec!["outer", "inner", "backend"]);
    }

    #[allow(dead_code)]
    mod boxed_builder_order {
        pub trait TestRecord
        where
            Self: Send + Sync + 'static,
        {
            fn inner(&self) -> Option<&dyn TestRecord>;

            fn backend(&self) -> &'static str {
                "unknown"
            }

            fn record(&self, into: &mut Vec<&'static str>) {
                if let Some(inner) = self.inner() {
                    inner.record(into);
                }
            }
        }

        pub trait TestFolder
        where
            Self: Send + Sync + 'static,
        {
            fn inner(&self) -> Option<&dyn TestFolder>;

            fn backend(&self) -> &'static str {
                "unknown"
            }
        }

        impl<T> TestRecord for std::sync::Arc<T>
        where
            T: TestRecord + ?Sized,
        {
            fn inner(&self) -> Option<&dyn TestRecord> {
                (**self).inner()
            }

            fn backend(&self) -> &'static str {
                (**self).backend()
            }

            fn record(&self, into: &mut Vec<&'static str>) {
                (**self).record(into);
            }
        }

        impl<T> TestFolder for std::sync::Arc<T>
        where
            T: TestFolder + ?Sized,
        {
            fn inner(&self) -> Option<&dyn TestFolder> {
                (**self).inner()
            }

            fn backend(&self) -> &'static str {
                (**self).backend()
            }
        }

        service_wrapper_module! {
            service_name: "test",
            version: "v1",
            struct_name: TestService,
            field_name: test_service,
            trait_path: self::TestRecord as TestRecordTrait,
        }

        crate::composite_service_builder! {
            composite_trait: TestRecordTestFolder,
            builder: BoxTestRecordTestFolderBuilder,
            alias: BoxTestRecordTestFolder,
            erased_layer: BoxTestRecordTestFolderErasedLayer,
            erased_layer_impl: BoxTestRecordTestFolderErasedLayerImpl,
            parts: BoxTestRecordTestFolderParts,
            output: TestService,
            boxed_output: BoxTestService,
            primary_trait: TestRecordTrait,
            secondary_trait: self::TestFolder as TestFolderTrait,
        }

        struct Backend;
        impl TestRecord for Backend {
            fn inner(&self) -> Option<&dyn TestRecord> {
                None
            }

            fn record(&self, into: &mut Vec<&'static str>) {
                into.push("backend");
            }
        }

        impl TestFolder for Backend {
            fn inner(&self) -> Option<&dyn TestFolder> {
                None
            }
        }

        struct TaggedLayer {
            tag: &'static str,
        }

        impl<S> Layer<S> for TaggedLayer {
            type Service = TaggedService<S>;

            fn layer(&self, inner: S) -> Self::Service {
                TaggedService { tag: self.tag, inner }
            }
        }

        struct TaggedService<S> {
            tag: &'static str,
            inner: S,
        }

        impl<S: TestRecord> TestRecord for TaggedService<S> {
            fn inner(&self) -> Option<&dyn TestRecord> {
                Some(&self.inner)
            }

            fn record(&self, into: &mut Vec<&'static str>) {
                into.push(self.tag);
                self.inner.record(into);
            }
        }

        impl<S: TestFolder> TestFolder for TaggedService<S> {
            fn inner(&self) -> Option<&dyn TestFolder> {
                Some(&self.inner)
            }
        }

        #[test]
        fn service_builder_boxed_output_type_compiles() {
            let service: BoxTestService = TestServiceBuilder::new()
                .layer(TaggedLayer { tag: "outer" })
                .boxed(Backend);

            let mut order = Vec::new();
            service.record(&mut order);

            assert_eq!(order, vec!["outer", "backend"]);
        }

        #[test]
        fn boxed_layers_are_called_in_the_order_they_were_added() {
            let service = BoxTestServiceBuilder::new(Backend)
                .layer(TaggedLayer { tag: "first" })
                .layer(TaggedLayer { tag: "second" })
                .layer(TaggedLayer { tag: "third" })
                .build();

            let mut order = Vec::new();
            service.record(&mut order);

            assert_eq!(order, vec!["first", "second", "third", "backend"]);
        }

        #[test]
        fn boxed_option_layer_some_is_included_in_order() {
            let service = BoxTestServiceBuilder::new(Backend)
                .layer(TaggedLayer { tag: "outer" })
                .option_layer(Some(TaggedLayer { tag: "middle" }))
                .layer(TaggedLayer { tag: "inner" })
                .build();

            let mut order = Vec::new();
            service.record(&mut order);

            assert_eq!(order, vec!["outer", "middle", "inner", "backend"]);
        }

        #[test]
        fn boxed_option_layer_none_is_skipped_in_order() {
            let service = BoxTestServiceBuilder::new(Backend)
                .layer(TaggedLayer { tag: "outer" })
                .option_layer(Option::<TaggedLayer>::None)
                .layer(TaggedLayer { tag: "inner" })
                .build();

            let mut order = Vec::new();
            service.record(&mut order);

            assert_eq!(order, vec!["outer", "inner", "backend"]);
        }

        #[test]
        fn composite_boxed_layers_are_called_in_the_order_they_were_added() {
            let service = BoxTestRecordTestFolderBuilder::new(Backend)
                .layer(TaggedLayer { tag: "first" })
                .layer(TaggedLayer { tag: "second" })
                .layer(TaggedLayer { tag: "third" })
                .build();

            let mut order = Vec::new();
            service.record(&mut order);

            assert_eq!(order, vec!["first", "second", "third", "backend"]);
        }

        #[test]
        fn composite_boxed_option_layer_some_is_included_in_order() {
            let service = BoxTestRecordTestFolderBuilder::new(Backend)
                .layer(TaggedLayer { tag: "outer" })
                .option_layer(Some(TaggedLayer { tag: "middle" }))
                .layer(TaggedLayer { tag: "inner" })
                .build();

            let mut order = Vec::new();
            service.record(&mut order);

            assert_eq!(order, vec!["outer", "middle", "inner", "backend"]);
        }

        #[test]
        fn composite_boxed_option_layer_none_is_skipped_in_order() {
            let service = BoxTestRecordTestFolderBuilder::new(Backend)
                .layer(TaggedLayer { tag: "outer" })
                .option_layer(Option::<TaggedLayer>::None)
                .layer(TaggedLayer { tag: "inner" })
                .build();

            let mut order = Vec::new();
            service.record(&mut order);

            assert_eq!(order, vec!["outer", "inner", "backend"]);
        }

        #[test]
        fn composite_arc_can_be_used_as_either_supertrait() {
            let composite: std::sync::Arc<dyn TestRecordTestFolder> = std::sync::Arc::new(Backend);
            let record: std::sync::Arc<dyn TestRecordTrait> = composite.clone();
            let folder: std::sync::Arc<dyn TestFolderTrait> = composite;

            assert_eq!(record.backend(), "unknown");
            assert_eq!(folder.backend(), "unknown");
        }
    }
}
