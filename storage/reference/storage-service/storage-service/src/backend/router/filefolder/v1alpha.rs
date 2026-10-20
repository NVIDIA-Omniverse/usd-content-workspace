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

use futures::future::BoxFuture;
use storage_core::traits::{filefolder::v1alpha::*, BoxStream, Request, Result, StreamingOptions};
use tonic::async_trait;

use crate::backend::router::{error::Error, Dummy, Match, Service, ServiceRoute};

impl<A, MA: Match, B> Service<ServiceRoute<A, MA>, B> {
    pub(super) fn match_folder_address_v1alpha(&self, address: Option<&FolderAddress>) -> bool {
        if let Some(address) = address {
            self.head.selector.match_folder_address(&address.uri)
        } else {
            false
        }
    }
}

impl<A: FileFolder, MA: Match + 'static, B: FileFolder> FileFolder for Service<ServiceRoute<A, MA>, B> {
    fn inner(&self) -> Option<&dyn FileFolder> {
        None
    }

    fn list<'a: 'r, 'r>(
        &'a self,
        request: Request<ListRequest>,
        streaming: StreamingOptions,
    ) -> BoxFuture<'r, Result<BoxStream<ListResponseWithContinueToken>>> {
        if self.match_folder_address_v1alpha(request.get_ref().folder.as_ref()) {
            self.head.service.list(request, streaming)
        } else {
            self.tail.list(request, streaming)
        }
    }

    fn list_stat<'a: 'r, 'r>(
        &'a self,
        request: Request<ListStatRequest>,
        streaming: StreamingOptions,
    ) -> BoxFuture<'r, Result<BoxStream<ListStatResponseWithContinueToken>>> {
        if self.match_folder_address_v1alpha(request.get_ref().folder.as_ref()) {
            self.head.service.list_stat(request, streaming)
        } else {
            self.tail.list_stat(request, streaming)
        }
    }

    fn delete_folder<'a: 'r, 'r>(
        &'a self,
        request: Request<DeleteFolderRequest>,
    ) -> BoxFuture<'r, Result<DeleteFolderResponse>> {
        if self.match_folder_address_v1alpha(request.get_ref().folder.as_ref()) {
            self.head.service.delete_folder(request)
        } else {
            self.tail.delete_folder(request)
        }
    }

    fn create_folder<'a: 'r, 'r>(
        &'a self,
        request: Request<CreateFolderRequest>,
    ) -> BoxFuture<'r, Result<CreateFolderResponse>> {
        if self.match_folder_address_v1alpha(request.get_ref().folder.as_ref()) {
            self.head.service.create_folder(request)
        } else {
            self.tail.create_folder(request)
        }
    }

    fn get_folder_mode<'a: 'r, 'r>(
        &'a self,
        request: Request<GetFolderModeRequest>,
    ) -> BoxFuture<'r, Result<GetFolderModeResponse>> {
        if self.match_folder_address_v1alpha(request.get_ref().folder.as_ref()) {
            self.head.service.get_folder_mode(request)
        } else {
            self.tail.get_folder_mode(request)
        }
    }
}

#[async_trait]
impl FileFolder for Dummy {
    fn inner(&self) -> Option<&dyn FileFolder> {
        None
    }

    async fn list(
        &self,
        _request: Request<ListRequest>,
        _streaming: StreamingOptions,
    ) -> Result<BoxStream<ListResponseWithContinueToken>> {
        Err(Error::UnsupportedAddress.into())
    }

    async fn list_stat(
        &self,
        _request: Request<ListStatRequest>,
        _streaming: StreamingOptions,
    ) -> Result<BoxStream<ListStatResponseWithContinueToken>> {
        Err(Error::UnsupportedAddress.into())
    }

    async fn delete_folder(&self, _request: Request<DeleteFolderRequest>) -> Result<DeleteFolderResponse> {
        Err(Error::UnsupportedAddress.into())
    }

    async fn create_folder(&self, _request: Request<CreateFolderRequest>) -> Result<CreateFolderResponse> {
        Err(Error::UnsupportedAddress.into())
    }

    async fn get_folder_mode(&self, _request: Request<GetFolderModeRequest>) -> Result<GetFolderModeResponse> {
        Err(Error::UnsupportedAddress.into())
    }
}
