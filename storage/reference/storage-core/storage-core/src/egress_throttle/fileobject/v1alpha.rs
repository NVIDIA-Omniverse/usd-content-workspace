// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;

use storage_core_builder::traits::{
    error::Result,
    fileobject::v1alpha::{
        AbortMultipartUploadRequest, AbortMultipartUploadResponse, CompleteMultipartUploadRequest,
        CompleteMultipartUploadResponse, CompleteRedirectUploadRequest, CompleteRedirectUploadResponse, CopyRequest,
        CopyResponse, DeleteRequest, DeleteResponse, EnumerateRequest, EnumerateResponseWithContinueToken,
        FetchWriteTypeInfoRequest, FetchWriteTypeInfoResponse, FileObject, GetOptimisticLockingSupportRequest,
        GetOptimisticLockingSupportResponse, IsReadOnlyRequest, IsReadOnlyResponse, MoveRequest, MoveResponse,
        ReadFromAddressRequest, ReadFromAddressResponse, ReadRequest, ReadResponse, StatRequest, StatResponse,
        UploadPartRequest, UploadPartResponse, WriteParameters, WriteResponse,
    },
    BoxFuture, BoxStream, Bytes, Request, StreamingOptions,
};

use crate::egress_throttle::EgressThrottle;

#[async_trait]
impl<S> FileObject for EgressThrottle<S>
where
    S: FileObject,
{
    fn inner(&self) -> Option<&dyn FileObject> {
        Some(&self.inner)
    }

    async fn stat(&self, request: Request<StatRequest>) -> Result<StatResponse> {
        let _permit = self.semaphore.acquire().await;
        self.inner.stat(request).await
    }

    async fn read(&self, request: Request<ReadRequest>) -> Result<BoxStream<ReadResponse>> {
        let _permit = self.semaphore.acquire().await;
        self.inner.read(request).await
    }

    async fn read_from_address(
        &self,
        request: Request<ReadFromAddressRequest>,
    ) -> Result<BoxStream<ReadFromAddressResponse>> {
        let _permit = self.semaphore.acquire().await;
        self.inner.read_from_address(request).await
    }

    async fn write(
        &self,
        request: Request<WriteParameters>,
        data_future: BoxFuture<Bytes>,
    ) -> Result<BoxStream<WriteResponse>> {
        let _permit = self.semaphore.acquire().await;
        self.inner.write(request, data_future).await
    }

    async fn delete(&self, request: Request<DeleteRequest>) -> Result<DeleteResponse> {
        let _permit = self.semaphore.acquire().await;
        self.inner.delete(request).await
    }

    async fn abort_multipart_upload(
        &self,
        request: Request<AbortMultipartUploadRequest>,
    ) -> Result<AbortMultipartUploadResponse> {
        let _permit = self.semaphore.acquire().await;
        self.inner.abort_multipart_upload(request).await
    }

    async fn complete_multipart_upload(
        &self,
        request: Request<CompleteMultipartUploadRequest>,
    ) -> Result<CompleteMultipartUploadResponse> {
        let _permit = self.semaphore.acquire().await;
        self.inner.complete_multipart_upload(request).await
    }

    async fn upload_part(&self, request: Request<UploadPartRequest>) -> Result<UploadPartResponse> {
        let _permit = self.semaphore.acquire().await;
        self.inner.upload_part(request).await
    }

    async fn complete_redirect_upload(
        &self,
        request: Request<CompleteRedirectUploadRequest>,
    ) -> Result<CompleteRedirectUploadResponse> {
        let _permit = self.semaphore.acquire().await;
        self.inner.complete_redirect_upload(request).await
    }

    async fn fetch_write_type_info(
        &self,
        request: Request<FetchWriteTypeInfoRequest>,
    ) -> Result<FetchWriteTypeInfoResponse> {
        let _permit = self.semaphore.acquire().await;
        self.inner.fetch_write_type_info(request).await
    }

    async fn enumerate(
        &self,
        request: Request<EnumerateRequest>,
        streaming: StreamingOptions,
    ) -> Result<BoxStream<EnumerateResponseWithContinueToken>> {
        let _permit = self.semaphore.acquire().await;
        self.inner.enumerate(request, streaming).await
    }

    async fn copy(&self, request: Request<CopyRequest>) -> Result<CopyResponse> {
        let _permit = self.semaphore.acquire().await;
        self.inner.copy(request).await
    }

    async fn move_object(&self, request: Request<MoveRequest>) -> Result<MoveResponse> {
        let _permit = self.semaphore.acquire().await;
        self.inner.move_object(request).await
    }

    async fn get_optimistic_locking_support(
        &self,
        request: Request<GetOptimisticLockingSupportRequest>,
    ) -> Result<GetOptimisticLockingSupportResponse> {
        let _permit = self.semaphore.acquire().await;
        self.inner.get_optimistic_locking_support(request).await
    }

    async fn is_read_only(&self, request: Request<IsReadOnlyRequest>) -> Result<IsReadOnlyResponse> {
        // Same reasoning as `is_versioned` on the versioning layer: the S3/Azure
        // backends answer this from config, but a manifest layer below this throttle
        // may issue a backend read for a cold-cache manifest, so the call must consume
        // an egress permit like every other backend-touching method (otherwise
        // concurrent calls bypass `max_concurrency`).
        let _permit = self.semaphore.acquire().await;
        self.inner.is_read_only(request).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use storage_core_util::semaphore::Semaphore;
    use tokio::sync::Notify;

    use super::*;

    /// Mock backend whose `is_read_only` records how many callers are inside it
    /// (with a high-water mark) and then blocks on `release`, letting the test
    /// assert the egress permit — not merely the shared blocker — gates entry.
    struct CountingBackend {
        release: Arc<Notify>,
        inside: Arc<AtomicUsize>,
        max_inside: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl FileObject for CountingBackend {
        fn inner(&self) -> Option<&dyn FileObject> {
            None
        }

        async fn is_read_only(&self, _request: Request<IsReadOnlyRequest>) -> Result<IsReadOnlyResponse> {
            let now = self.inside.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_inside.fetch_max(now, Ordering::SeqCst);
            self.release.notified().await;
            self.inside.fetch_sub(1, Ordering::SeqCst);
            Ok(IsReadOnlyResponse { is_read_only: true })
        }
    }

    fn request() -> Request<IsReadOnlyRequest> {
        Request::new(IsReadOnlyRequest {
            resource_address: "https://bucket.example/object".to_string(),
        })
    }

    /// With a single egress permit, two concurrent `is_read_only` calls must not both
    /// enter the backend: the manifest layer below may issue a cold-cache `.manifest`
    /// read, so entry has to be gated. Without the override the generated default
    /// forwards straight through and the high-water mark would be 2.
    #[tokio::test]
    async fn is_read_only_gates_backend_entry_on_egress_permit() {
        let release = Arc::new(Notify::new());
        let inside = Arc::new(AtomicUsize::new(0));
        let max_inside = Arc::new(AtomicUsize::new(0));
        let throttle = Arc::new(EgressThrottle::new(
            CountingBackend {
                release: release.clone(),
                inside: inside.clone(),
                max_inside: max_inside.clone(),
            },
            Arc::new(Semaphore::new(1, "egress_is_read_only_test")),
        ));

        let t1 = {
            let throttle = throttle.clone();
            tokio::spawn(async move { throttle.is_read_only(request()).await })
        };
        let t2 = {
            let throttle = throttle.clone();
            tokio::spawn(async move { throttle.is_read_only(request()).await })
        };

        // Let both tasks run as far as they can. Exactly one acquires the permit and
        // enters the backend; the other blocks on the permit. Yield several times so a
        // permit-bypass bug has every chance to let the second one in.
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

        release.notify_one();
        release.notify_one();
        let (r1, r2) = tokio::join!(t1, t2);
        assert!(r1.unwrap().unwrap().is_read_only && r2.unwrap().unwrap().is_read_only);
        assert_eq!(
            max_inside.load(Ordering::SeqCst),
            1,
            "entry stayed serialized through completion"
        );
    }
}
