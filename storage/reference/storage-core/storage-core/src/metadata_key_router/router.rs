// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::sync::Arc;

use snafu::prelude::*;
use storage_core_builder::traits::error::{status_with_default_error_info, SourceLocation, StorageErrorExt};
use storage_core_builder::traits::metadata::v1alpha::Metadata;

/// Routing service that dispatches metadata requests to one of N inner services
/// based on the metadata key in the request.
///
/// Has a per-key route table (built via [`MetadataKeyRouterBuilder::route`]) and a
/// mandatory default branch (set via [`MetadataKeyRouterBuilder::default`]). For
/// `GetMetadata` with multiple keys, requests fan out to the relevant branches in
/// parallel and the responses are merged. For `GetMetadata` with no keys specified,
/// every branch is asked for the keys it owns and the default is asked for everything
/// else.
///
/// Constructed via [`MetadataKeyRouter::builder`] — there is no figment integration on
/// the router itself; configuration is the consumer's concern.
pub struct MetadataKeyRouter {
    pub(crate) branches: Vec<Branch>,
    pub(crate) default: Arc<dyn Metadata + Send + Sync + 'static>,
    pub(crate) key_to_branch: HashMap<String, usize>,
}

pub(crate) struct Branch {
    pub(crate) service: Arc<dyn Metadata + Send + Sync + 'static>,
    pub(crate) keys: Vec<String>,
}

/// Where a key should be dispatched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum RouteTarget {
    Branch(usize),
    Default,
}

impl MetadataKeyRouter {
    pub fn builder() -> MetadataKeyRouterBuilder {
        <MetadataKeyRouterBuilder as Default>::default()
    }

    pub(crate) fn route_for_key(&self, key: &str) -> RouteTarget {
        // Routed keys are stored lowercased; lookups normalize the same way so
        // mixed-case input still matches. The backends lowercase metadata keys
        // before storage, so a routing decision on the original case would
        // otherwise diverge from where the data actually lives.
        match self.key_to_branch.get(&key.to_lowercase()) {
            Some(&i) => RouteTarget::Branch(i),
            None => RouteTarget::Default,
        }
    }
}

#[derive(Default)]
pub struct MetadataKeyRouterBuilder {
    branches: Vec<PendingBranch>,
    default: Option<Arc<dyn Metadata + Send + Sync + 'static>>,
}

struct PendingBranch {
    service: Arc<dyn Metadata + Send + Sync + 'static>,
    keys: Vec<String>,
}

impl MetadataKeyRouterBuilder {
    /// Add a branch with the keys it owns. Multiple calls accumulate.
    /// Duplicate keys across calls are caught at [`build`](Self::build) time.
    pub fn route(
        mut self,
        keys: impl IntoIterator<Item = impl Into<String>>,
        service: Arc<dyn Metadata + Send + Sync + 'static>,
    ) -> Self {
        self.branches.push(PendingBranch {
            service,
            keys: keys.into_iter().map(Into::into).collect(),
        });
        self
    }

    /// Set the default branch. Mandatory. Last call wins.
    pub fn default(mut self, service: Arc<dyn Metadata + Send + Sync + 'static>) -> Self {
        self.default = Some(service);
        self
    }

    pub fn build(self) -> std::result::Result<MetadataKeyRouter, BuildError> {
        let default = self.default.ok_or_else(|| MissingDefault.build())?;

        let mut key_to_branch: HashMap<String, usize> = HashMap::new();
        let mut branches: Vec<Branch> = Vec::with_capacity(self.branches.len());
        for (idx, pending) in self.branches.into_iter().enumerate() {
            let normalized: Vec<String> = pending.keys.into_iter().map(|k| k.to_lowercase()).collect();
            for key in &normalized {
                if key_to_branch.contains_key(key) {
                    return Err(DuplicateKey { key: key.clone() }.build());
                }
                key_to_branch.insert(key.clone(), idx);
            }
            branches.push(Branch {
                service: pending.service,
                keys: normalized,
            });
        }

        Ok(MetadataKeyRouter {
            branches,
            default,
            key_to_branch,
        })
    }
}

/// Errors returned by [`MetadataKeyRouterBuilder::build`].
#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)), context(suffix(false)))]
pub enum BuildError {
    #[snafu(display("metadata_key_router: key {key:?} appears in more than one route"))]
    DuplicateKey {
        key: String,
        #[snafu(implicit)]
        loc: snafu::Location,
    },

    #[snafu(display("metadata_key_router: default branch is required but was not set"))]
    MissingDefault {
        #[snafu(implicit)]
        loc: snafu::Location,
    },
}

/// Internal runtime error emitted by the metadata-key router.
///
/// Today only [`Error::Internal`] is produced — for example when merging
/// per-branch `get_metadata` responses encounters a duplicate key (which
/// build-time validation should already prevent). Inner-service errors are
/// propagated as-is.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)), context(suffix(false)))]
pub enum Error {
    #[snafu(display("metadata_key_router: internal error: {message}"))]
    Internal {
        message: String,
        #[snafu(implicit)]
        loc: snafu::Location,
    },
}

impl Error {
    // Currently only invoked from unit-test stubs that need to surface a StorageError.
    #[cfg(test)]
    #[track_caller]
    pub(crate) fn internal(message: impl Into<String>) -> Self {
        Internal {
            message: message.into(),
        }
        .build()
    }
}

impl StorageErrorExt for Error {
    fn tonic_status(&self) -> tonic::Status {
        match self {
            Error::Internal { .. } => {
                status_with_default_error_info(tonic::Code::Internal, self.to_string(), "Internal")
            },
        }
    }

    fn http_status(&self) -> http::StatusCode {
        http::StatusCode::INTERNAL_SERVER_ERROR
    }

    fn get_size(&self) -> usize {
        std::mem::size_of_val(self)
    }

    fn source_location(&self) -> Option<SourceLocation> {
        match self {
            Error::Internal { loc, .. } => Some(loc.into()),
        }
    }
}

#[cfg(test)]
const _: fn() = || {
    fn assert_obj_safe<T: ?Sized>() {}
    assert_obj_safe::<dyn Metadata + Send + Sync + 'static>();
};

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use storage_core_builder::traits::error::Result;
    use storage_core_builder::traits::metadata::v1alpha::{
        DeleteMetadataRequest, DeleteMetadataResponse, GetMetadataRequest, GetMetadataResponse, UpdateMetadataRequest,
        UpdateMetadataResponse,
    };
    use storage_core_builder::traits::Request;

    /// Minimal stub used only by the build-validation tests.
    #[derive(Default)]
    struct Stub;

    #[async_trait]
    impl Metadata for Stub {
        fn inner(&self) -> Option<&dyn Metadata> {
            None
        }

        async fn get_metadata(&self, _request: Request<GetMetadataRequest>) -> Result<GetMetadataResponse> {
            Ok(GetMetadataResponse {
                user_metadata: Default::default(),
            })
        }

        async fn update_metadata(&self, _request: Request<UpdateMetadataRequest>) -> Result<UpdateMetadataResponse> {
            Ok(UpdateMetadataResponse { etag: String::new() })
        }

        async fn delete_metadata(&self, _request: Request<DeleteMetadataRequest>) -> Result<DeleteMetadataResponse> {
            Ok(DeleteMetadataResponse {})
        }
    }

    fn stub() -> Arc<dyn Metadata + Send + Sync + 'static> {
        Arc::new(Stub)
    }

    #[test]
    fn valid_build_succeeds() {
        let router = MetadataKeyRouter::builder()
            .route(["a", "b"], stub())
            .route(["c"], stub())
            .default(stub())
            .build()
            .unwrap();

        assert_eq!(router.branches.len(), 2);
        assert_eq!(router.key_to_branch.get("a"), Some(&0));
        assert_eq!(router.key_to_branch.get("b"), Some(&0));
        assert_eq!(router.key_to_branch.get("c"), Some(&1));
        assert!(!router.key_to_branch.contains_key("unrouted"));
    }

    #[test]
    fn missing_default_is_rejected() {
        let result = MetadataKeyRouter::builder().route(["a"], stub()).build();
        match result {
            Err(BuildError::MissingDefault { .. }) => {},
            Err(other) => panic!("expected MissingDefault, got {other:?}"),
            Ok(_) => panic!("expected MissingDefault, got Ok"),
        }
    }

    #[test]
    fn duplicate_key_across_routes_is_rejected() {
        let result = MetadataKeyRouter::builder()
            .route(["shared"], stub())
            .route(["shared"], stub())
            .default(stub())
            .build();
        match result {
            Err(BuildError::DuplicateKey { key, .. }) => assert_eq!(key, "shared"),
            Err(other) => panic!("expected DuplicateKey, got {other:?}"),
            Ok(_) => panic!("expected DuplicateKey, got Ok"),
        }
    }

    #[test]
    fn duplicate_key_within_single_route_is_rejected() {
        let result = MetadataKeyRouter::builder()
            .route(["dup", "dup"], stub())
            .default(stub())
            .build();
        match result {
            Err(BuildError::DuplicateKey { key, .. }) => assert_eq!(key, "dup"),
            Err(other) => panic!("expected DuplicateKey, got {other:?}"),
            Ok(_) => panic!("expected DuplicateKey, got Ok"),
        }
    }

    #[test]
    fn route_for_key_is_case_insensitive() {
        let router = MetadataKeyRouter::builder()
            .route(["Foo", "BAR"], stub())
            .default(stub())
            .build()
            .unwrap();

        assert_eq!(router.route_for_key("foo"), RouteTarget::Branch(0));
        assert_eq!(router.route_for_key("FOO"), RouteTarget::Branch(0));
        assert_eq!(router.route_for_key("Foo"), RouteTarget::Branch(0));
        assert_eq!(router.route_for_key("bar"), RouteTarget::Branch(0));
        assert_eq!(router.route_for_key("Bar"), RouteTarget::Branch(0));
        assert_eq!(router.route_for_key("baz"), RouteTarget::Default);
    }

    #[test]
    fn case_collision_is_treated_as_duplicate() {
        // Two routes that collide only after lowercasing must still be rejected,
        // otherwise one would silently shadow the other depending on insertion order.
        let result = MetadataKeyRouter::builder()
            .route(["FOO"], stub())
            .route(["foo"], stub())
            .default(stub())
            .build();
        match result {
            Err(BuildError::DuplicateKey { key, .. }) => assert_eq!(key, "foo"),
            Err(other) => panic!("expected DuplicateKey, got {other:?}"),
            Ok(_) => panic!("expected DuplicateKey, got Ok"),
        }
    }

    #[test]
    fn branch_keys_are_stored_lowercased() {
        // The empty-keys fan-out (in metadata/v1alpha.rs) sends Branch.keys to the
        // inner service. Storing them lowercased keeps the wire-level request consistent
        // with how backends store metadata.
        let router = MetadataKeyRouter::builder()
            .route(["MixedCase", "ANOTHER"], stub())
            .default(stub())
            .build()
            .unwrap();

        assert_eq!(router.branches[0].keys, vec!["mixedcase", "another"]);
    }
}
