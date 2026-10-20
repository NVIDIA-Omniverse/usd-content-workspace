// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use std::sync::{Arc, Mutex};

use futures::executor::block_on;
use storage_core::{
    api_services::fileobject::v1alpha::{
        BoxFileObject, BoxFileObjectBuilder, BoxFileObjectFileFolderBuilder, FileObjectBuilder,
    },
    layer::Layer,
    traits::{
        async_trait,
        filefolder::v1alpha::FileFolder,
        fileobject::v1alpha::{FileObject, StatRequest, StatResponse},
        Request, Result,
    },
};

#[derive(Clone, Default)]
struct Recorder {
    order: Arc<Mutex<Vec<&'static str>>>,
}

impl Recorder {
    fn push(&self, tag: &'static str) {
        self.order.lock().expect("recorder poisoned").push(tag);
    }

    fn snapshot(&self) -> Vec<&'static str> {
        self.order.lock().expect("recorder poisoned").clone()
    }
}

#[derive(Clone)]
struct Backend {
    recorder: Recorder,
}

#[async_trait]
impl FileObject for Backend {
    fn inner(&self) -> Option<&dyn FileObject> {
        None
    }

    fn backend(&self) -> &'static str {
        "backend"
    }

    async fn stat(&self, _request: Request<StatRequest>) -> Result<StatResponse> {
        self.recorder.push("backend");
        Ok(StatResponse::default())
    }
}

impl FileFolder for Backend {
    fn inner(&self) -> Option<&dyn FileFolder> {
        None
    }

    fn backend(&self) -> &'static str {
        "backend"
    }
}

#[derive(Clone)]
struct TaggedLayer {
    tag: &'static str,
    recorder: Recorder,
}

impl<S> Layer<S> for TaggedLayer {
    type Service = TaggedService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        TaggedService {
            tag: self.tag,
            recorder: self.recorder.clone(),
            inner,
        }
    }
}

struct TaggedService<S> {
    tag: &'static str,
    recorder: Recorder,
    inner: S,
}

#[async_trait]
impl<S: FileObject> FileObject for TaggedService<S> {
    fn inner(&self) -> Option<&dyn FileObject> {
        Some(&self.inner)
    }

    fn backend(&self) -> &'static str {
        self.tag
    }

    async fn stat(&self, request: Request<StatRequest>) -> Result<StatResponse> {
        self.recorder.push(self.tag);
        self.inner.stat(request).await
    }
}

impl<S: FileFolder> FileFolder for TaggedService<S> {
    fn inner(&self) -> Option<&dyn FileFolder> {
        Some(&self.inner)
    }

    fn backend(&self) -> &'static str {
        self.tag
    }
}

fn layer(tag: &'static str, recorder: &Recorder) -> TaggedLayer {
    TaggedLayer {
        tag,
        recorder: recorder.clone(),
    }
}

fn backend(recorder: &Recorder) -> Backend {
    Backend {
        recorder: recorder.clone(),
    }
}

fn static_service() -> (impl FileObject, Recorder) {
    let recorder = Recorder::default();
    let service = FileObjectBuilder::new()
        .layer(layer("outer", &recorder))
        .layer(layer("inner", &recorder))
        .backend(backend(&recorder));

    (service, recorder)
}

fn boundary_boxed_service() -> (BoxFileObject, Recorder) {
    let recorder = Recorder::default();
    let service = FileObjectBuilder::new()
        .layer(layer("outer", &recorder))
        .layer(layer("inner", &recorder))
        .boxed(backend(&recorder));

    (service, recorder)
}

fn per_layer_boxed_service() -> (BoxFileObject, Recorder) {
    let recorder = Recorder::default();
    let service = BoxFileObjectBuilder::new(backend(&recorder))
        .layer(layer("outer", &recorder))
        .layer(layer("inner", &recorder))
        .build();

    (service, recorder)
}

fn composite_boxed_service() -> (BoxFileObject, Recorder) {
    let recorder = Recorder::default();
    let service = BoxFileObjectFileFolderBuilder::new(backend(&recorder))
        .layer(layer("outer", &recorder))
        .layer(layer("inner", &recorder))
        .build();

    (service, recorder)
}

fn file_object_order(service: &dyn FileObject, recorder: &Recorder) -> Vec<&'static str> {
    block_on(service.stat(Request::new(StatRequest::default()))).expect("mock stat should succeed");
    recorder.snapshot()
}

fn main() {
    let (static_service, static_recorder) = static_service();
    let (boundary_boxed_service, boundary_boxed_recorder) = boundary_boxed_service();
    let (per_layer_boxed_service, per_layer_boxed_recorder) = per_layer_boxed_service();
    let (composite_boxed_service, composite_boxed_recorder) = composite_boxed_service();

    let cases = [
        ("static", file_object_order(&static_service, &static_recorder)),
        (
            "boundary-boxed",
            file_object_order(&boundary_boxed_service, &boundary_boxed_recorder),
        ),
        (
            "per-layer-boxed",
            file_object_order(&per_layer_boxed_service, &per_layer_boxed_recorder),
        ),
        (
            "composite-boxed",
            file_object_order(&composite_boxed_service, &composite_boxed_recorder),
        ),
    ];

    for (name, order) in cases {
        println!("{name}: {}", order.join(" -> "));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_builder_styles_preserve_layer_order() {
        let expected = vec!["outer", "inner", "backend"];
        let (static_service, static_recorder) = static_service();
        let (boundary_boxed_service, boundary_boxed_recorder) = boundary_boxed_service();
        let (per_layer_boxed_service, per_layer_boxed_recorder) = per_layer_boxed_service();
        let (composite_boxed_service, composite_boxed_recorder) = composite_boxed_service();

        assert_eq!(file_object_order(&static_service, &static_recorder), expected);
        assert_eq!(
            file_object_order(&boundary_boxed_service, &boundary_boxed_recorder),
            expected
        );
        assert_eq!(
            file_object_order(&per_layer_boxed_service, &per_layer_boxed_recorder),
            expected
        );
        assert_eq!(
            file_object_order(&composite_boxed_service, &composite_boxed_recorder),
            expected
        );
    }
}
