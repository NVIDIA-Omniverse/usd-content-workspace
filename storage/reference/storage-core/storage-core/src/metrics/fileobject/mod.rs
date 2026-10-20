// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

pub mod v1alpha;
pub mod v1beta;

use crate::metrics::utils::TimedStream;
use crate::traits::Result;
use std::pin::Pin;
use storage_core_util::observability::metrics::{BoundCounter, BoundUIntHistogram};
use storage_core_util::warn_limited;

/// Trait to extract data size, chunk size, and redirect status from a read response
pub trait ReadResponseExt {
    fn extract_data_size(&self) -> Option<u64>;
    fn extract_chunk_size(&self) -> Option<u64>;
    fn is_redirect(&self) -> bool;
}

pub static DOWNLOAD_PREFERENCES: [&str; 2] = ["body", "redirect"];

// ReadStream wraps TimedStream and tracks read-specific metrics
pub struct ReadStream<T: ReadResponseExt> {
    inner: Pin<Box<TimedStream<T>>>,
    redirects: std::sync::Arc<BoundCounter>,
    chunk_size_hist: std::sync::Arc<BoundUIntHistogram>,
    data_size_hist: std::sync::Arc<BoundUIntHistogram>,
    data_size: std::cell::RefCell<Option<u64>>,
}

impl<T: ReadResponseExt> ReadStream<T> {
    pub fn new(
        timed_stream: TimedStream<T>,
        redirects: std::sync::Arc<BoundCounter>,
        chunk_size_hist: std::sync::Arc<BoundUIntHistogram>,
        data_size_hist: std::sync::Arc<BoundUIntHistogram>,
    ) -> Self {
        Self {
            inner: Box::pin(timed_stream),
            redirects,
            chunk_size_hist,
            data_size_hist,
            data_size: std::cell::RefCell::new(None),
        }
    }
}

impl<T: ReadResponseExt> Drop for ReadStream<T> {
    fn drop(&mut self) {
        let total_data_size = *self.data_size.borrow();
        if let Some(total_data_size) = total_data_size {
            self.data_size_hist.record(total_data_size);
        }
    }
}

impl<T: ReadResponseExt> futures::Stream for ReadStream<T> {
    type Item = Result<T>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Option<Self::Item>> {
        match self.inner.as_mut().poll_next(cx) {
            std::task::Poll::Ready(Some(Ok(response))) => {
                if let Some(data_size) = response.extract_data_size() {
                    *self.data_size.borrow_mut() = Some(data_size);
                }
                if let Some(chunk_size) = response.extract_chunk_size() {
                    self.chunk_size_hist.record(chunk_size);
                }
                if response.is_redirect() {
                    self.redirects.add(1);
                }
                std::task::Poll::Ready(Some(Ok(response)))
            },
            other => other,
        }
    }
}

pub static UPLOAD_PREFERENCES: [&str; 4] = ["body", "redirect", "multipart", "unspecified"];

pub trait UploadPreferenceExt {
    fn upload_preference(&self) -> &'static str;
}

// WriteStream wraps TimedStream and tracks write-specific metrics
pub struct WriteStream<T: UploadPreferenceExt> {
    inner: Pin<Box<TimedStream<T>>>,
    upload_method_body: std::sync::Arc<BoundCounter>,
    upload_method_redirect: std::sync::Arc<BoundCounter>,
    upload_method_multipart: std::sync::Arc<BoundCounter>,
    data_size_hist: std::sync::Arc<BoundUIntHistogram>,
    upload_preference: std::cell::RefCell<&'static str>,
    data_size: u64,
}

impl<T: UploadPreferenceExt> WriteStream<T> {
    pub fn new(
        timed_stream: TimedStream<T>,
        upload_method_body: std::sync::Arc<BoundCounter>,
        upload_method_redirect: std::sync::Arc<BoundCounter>,
        upload_method_multipart: std::sync::Arc<BoundCounter>,
        data_size_hist: std::sync::Arc<BoundUIntHistogram>,
        data_size: u64,
    ) -> Self {
        Self {
            inner: Box::pin(timed_stream),
            upload_method_body,
            upload_method_redirect,
            upload_method_multipart,
            data_size_hist,
            upload_preference: std::cell::RefCell::new("unknown"),
            data_size,
        }
    }
}

impl<T: UploadPreferenceExt> Drop for WriteStream<T> {
    fn drop(&mut self) {
        // Record the upload method used
        match *self.upload_preference.borrow() {
            "body" => self.upload_method_body.add(1),
            "redirect" => self.upload_method_redirect.add(1),
            "multipart" => self.upload_method_multipart.add(1),
            _ => {
                warn_limited!("Unknown upload preference: {}", *self.upload_preference.borrow());
            },
        }
        // Record the data size if available
        self.data_size_hist.record(self.data_size);
    }
}

impl<T: UploadPreferenceExt> futures::Stream for WriteStream<T> {
    type Item = Result<T>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Option<Self::Item>> {
        match self.inner.as_mut().poll_next(cx) {
            std::task::Poll::Ready(Some(Ok(response))) => {
                *self.upload_preference.borrow_mut() = response.upload_preference();
                std::task::Poll::Ready(Some(Ok(response)))
            },
            other => other,
        }
    }
}
