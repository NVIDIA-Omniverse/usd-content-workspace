// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;

use storage_core_builder::traits::{
    error::Result,
    versioning::v1alpha::{
        EnumerateVersionsRequest, EnumerateVersionsResponseWithContinueToken, IsVersionedRequest, IsVersionedResponse,
        Versioning,
    },
    BoxStream, Request, StreamingOptions,
};

use crate::egress_throttle::EgressThrottle;

#[async_trait]
impl<S> Versioning for EgressThrottle<S>
where
    S: Versioning,
{
    fn inner(&self) -> Option<&dyn Versioning> {
        Some(&self.inner)
    }

    async fn enumerate_versions(
        &self,
        request: Request<EnumerateVersionsRequest>,
        streaming: StreamingOptions,
        version_id_token: Option<String>,
    ) -> Result<BoxStream<EnumerateVersionsResponseWithContinueToken>> {
        let _permit = self.semaphore.acquire().await;
        self.inner
            .enumerate_versions(request, streaming, version_id_token)
            .await
    }

    async fn is_versioned(&self, request: Request<IsVersionedRequest>) -> Result<IsVersionedResponse> {
        // The S3/Azure backends answer `is_versioned` from config, but a manifest
        // layer below this throttle answers workspace paths by reading a cold-cache
        // `.manifest` from the backend, so the call consumes an egress permit like
        // every other backend-touching method (otherwise concurrent calls bypass
        // `max_concurrency`). When the manifest layer sits above this throttle, its
        // cold read goes through the manifest resolver instead, and only a throttle
        // that wraps the resolver gates it.
        let _permit = self.semaphore.acquire().await;
        self.inner.is_versioned(request).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use storage_core_util::semaphore::Semaphore;
    use tokio::sync::Notify;

    use super::*;

    /// Mock backend whose `is_versioned` records how many callers are inside it
    /// (with a high-water mark) and then blocks on `release`, letting the test
    /// assert the egress permit — not merely the shared blocker — is what gates
    /// entry.
    struct CountingBackend {
        release: Arc<Notify>,
        inside: Arc<AtomicUsize>,
        max_inside: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl Versioning for CountingBackend {
        fn inner(&self) -> Option<&dyn Versioning> {
            None
        }

        async fn is_versioned(&self, _request: Request<IsVersionedRequest>) -> Result<IsVersionedResponse> {
            let now = self.inside.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_inside.fetch_max(now, Ordering::SeqCst);
            self.release.notified().await;
            self.inside.fetch_sub(1, Ordering::SeqCst);
            Ok(IsVersionedResponse { is_versioned: true })
        }
    }

    fn request() -> Request<IsVersionedRequest> {
        Request::new(IsVersionedRequest {
            resource_address: "https://bucket.example/".to_string(),
        })
    }

    /// With a single egress permit, two concurrent `is_versioned` calls must not
    /// both enter the backend: the permit gates entry, so the high-water mark of
    /// concurrent callers inside the backend is exactly 1. If the permit override
    /// were removed both would enter and the mark would be 2. No assumption is
    /// made about which task wins the permit.
    #[tokio::test]
    async fn is_versioned_gates_backend_entry_on_egress_permit() {
        let release = Arc::new(Notify::new());
        let inside = Arc::new(AtomicUsize::new(0));
        let max_inside = Arc::new(AtomicUsize::new(0));
        let throttle = Arc::new(EgressThrottle::new(
            CountingBackend {
                release: release.clone(),
                inside: inside.clone(),
                max_inside: max_inside.clone(),
            },
            Arc::new(Semaphore::new(1, "egress_versioning_test")),
        ));

        let t1 = {
            let throttle = throttle.clone();
            tokio::spawn(async move { throttle.is_versioned(request()).await })
        };
        let t2 = {
            let throttle = throttle.clone();
            tokio::spawn(async move { throttle.is_versioned(request()).await })
        };

        // Let both tasks run as far as they can. Exactly one acquires the permit
        // and enters the backend; the other blocks on the permit. Yield several
        // times so a permit-bypass bug has every chance to let the second one in.
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        assert_eq!(
            inside.load(Ordering::SeqCst),
            1,
            "exactly one call should be inside the backend"
        );
        assert_eq!(
            max_inside.load(Ordering::SeqCst),
            1,
            "the egress permit must gate entry to at most one"
        );

        // Release the in-backend task; it frees the permit, the other enters and is
        // woken by the pre-stored notification. Order-independent.
        release.notify_one();
        release.notify_one();
        let (r1, r2) = tokio::join!(t1, t2);
        assert!(r1.unwrap().unwrap().is_versioned && r2.unwrap().unwrap().is_versioned);
        assert_eq!(
            max_inside.load(Ordering::SeqCst),
            1,
            "entry stayed serialized through completion"
        );
    }
}
