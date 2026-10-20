// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

use async_trait::async_trait;
use figment::Figment;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use storage_core::layer::Layer;
use storage_core::traits::error::Result;
use storage_core::traits::filefolder::v1alpha::{
    FileFolder as FileFolderV1Alpha, ListRequest as ListRequestV1Alpha,
    ListResponseWithContinueToken as ListResponseWithContinueTokenV1Alpha, ListStatRequest as ListStatRequestV1Alpha,
    ListStatResponseWithContinueToken as ListStatResponseWithContinueTokenV1Alpha,
};
use storage_core::traits::filefolder::v1beta::{
    FileFolder as FileFolderV1Beta, ListRequest as ListRequestV1Beta,
    ListResponseWithContinueToken as ListResponseWithContinueTokenV1Beta, ListStatRequest as ListStatRequestV1Beta,
    ListStatResponseWithContinueToken as ListStatResponseWithContinueTokenV1Beta,
};
use storage_core::traits::fileobject::v1alpha::{
    FileObject as FileObjectV1Alpha, StatRequest as StatRequestV1Alpha, StatResponse as StatResponseV1Alpha,
};
use storage_core::traits::fileobject::v1beta::{
    FileObject as FileObjectV1Beta, StatRequest as StatRequestV1Beta, StatResponse as StatResponseV1Beta,
};
use storage_core::traits::{BoxStream, Request, StreamingOptions};
use storage_core_util::{debug_limited, warn_limited};
use tracing::instrument;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Enable the remapper layer.
    /// Default is true
    #[serde(default = "default_enabled")]
    enabled: bool,

    /// Map of strings to remap. Keys are the original strings, values are the replacement strings.
    /// Default is an empty map
    #[serde(default = "default_remap_map")]
    remap_map: HashMap<String, String>,
}

fn default_enabled() -> bool {
    true
}

fn default_remap_map() -> HashMap<String, String> {
    HashMap::new()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            remap_map: default_remap_map(),
        }
    }
}

impl Config {
    pub const CONFIG_KEY: &str = "remapper";
}

impl TryFrom<&Figment> for Config {
    type Error = figment::Error;

    fn try_from(figment: &Figment) -> std::result::Result<Self, Self::Error> {
        figment.focus(Self::CONFIG_KEY).extract()
    }
}

pub struct RemapperLayer {
    remap_map: HashMap<String, String>,
}

impl RemapperLayer {
    pub fn new(config: Config) -> Option<Self> {
        if config.enabled {
            Some(Self {
                remap_map: config.remap_map,
            })
        } else {
            None
        }
    }

    pub fn from_figment(figment: &Figment) -> std::result::Result<Option<Self>, figment::Error> {
        let config = Config::try_from(figment)?;
        Ok(Self::new(config))
    }
}

impl<S> Layer<S> for RemapperLayer {
    type Service = Remapper<S>;
    fn layer(&self, inner: S) -> Self::Service {
        Remapper::new(inner, self.remap_map.clone())
    }
}

pub struct Remapper<S> {
    inner: S,
    remap_map: HashMap<String, String>,
}

impl<S> Remapper<S> {
    pub fn new(inner: S, remap_map: HashMap<String, String>) -> Self {
        Self { inner, remap_map }
    }

    fn remap(&self, original: &str) -> String {
        Self::remap_inner(original, &self.remap_map)
    }

    fn remap_inner(original: &str, remap_map: &HashMap<String, String>) -> String {
        let mut remapped = original.to_string();
        for (from, to) in remap_map {
            remapped = remapped.replace(from, to);
        }
        remapped
    }
}

#[async_trait]
impl<S> FileObjectV1Alpha for Remapper<S>
where
    S: FileObjectV1Alpha,
{
    fn inner(&self) -> Option<&dyn FileObjectV1Alpha> {
        Some(&self.inner)
    }

    #[instrument(skip_all)]
    async fn stat(&self, mut request: Request<StatRequestV1Alpha>) -> Result<StatResponseV1Alpha> {
        request.get_mut().resource_address = self.remap(&request.get_ref().resource_address);
        let mut result = self.inner.stat(request).await?;

        if let Some(ref mut resource_info) = result.resource_info {
            if let Some(ref mut resource_identity) = resource_info.resource_identity {
                resource_identity.encoded_identity = self.remap(&resource_identity.encoded_identity);
            }
        }

        Ok(result)
    }
}

#[async_trait]
impl<S> FileFolderV1Alpha for Remapper<S>
where
    S: FileFolderV1Alpha,
{
    fn inner(&self) -> Option<&dyn FileFolderV1Alpha> {
        Some(&self.inner)
    }

    #[instrument(skip(self, request, streaming))]
    async fn list(
        &self,
        mut request: Request<ListRequestV1Alpha>,
        streaming: StreamingOptions,
    ) -> Result<BoxStream<ListResponseWithContinueTokenV1Alpha>> {
        if let Some(ref mut folder) = request.get_mut().folder {
            folder.uri = self.remap(&folder.uri);
        }
        let stream = self.inner.list(request, streaming).await?;

        // Transform the stream to remove any results that contain remapped strings
        let remap_map = self.remap_map.clone();
        let transformed_stream = stream.map(move |response| match response {
            Ok(mut list_response) => {
                list_response
                    .response
                    .subfolder_addresses
                    .iter_mut()
                    .for_each(|subfolder| {
                        subfolder.uri = Self::remap_inner(&subfolder.uri, &remap_map);
                    });
                list_response
                    .response
                    .sub_resource_addresses
                    .iter_mut()
                    .for_each(|resource| {
                        *resource = Self::remap_inner(resource, &remap_map);
                    });
                Ok(list_response)
            },
            Err(e) => {
                warn_limited!("RemapperLayer: Error in list stream: {:?}", e);
                Err(e)
            },
        });

        debug_limited!("RemapperLayer: List request stream created");
        Ok(Box::pin(transformed_stream))
    }

    #[instrument(skip(self, request, streaming))]
    async fn list_stat(
        &self,
        mut request: Request<ListStatRequestV1Alpha>,
        streaming: StreamingOptions,
    ) -> Result<BoxStream<ListStatResponseWithContinueTokenV1Alpha>> {
        if let Some(ref mut folder) = request.get_mut().folder {
            folder.uri = self.remap(&folder.uri);
        }
        let stream = self.inner.list_stat(request, streaming).await?;

        // Transform the stream to remove any results that contain remapped strings
        let remap_map = self.remap_map.clone();
        let transformed_stream = stream.map(move |response| match response {
            Ok(mut list_response) => {
                list_response
                    .response
                    .subfolder_addresses
                    .iter_mut()
                    .for_each(|subfolder| {
                        subfolder.uri = Self::remap_inner(&subfolder.uri, &remap_map);
                    });
                list_response.response.entries.iter_mut().for_each(|entry| {
                    entry.resource_address = Self::remap_inner(&entry.resource_address, &remap_map);
                });
                Ok(list_response)
            },
            Err(e) => {
                warn_limited!("RemapperLayer: Error in list_stat stream: {:?}", e);
                Err(e)
            },
        });

        debug_limited!("RemapperLayer: List_stat request stream created");
        Ok(Box::pin(transformed_stream))
    }
}

#[async_trait]
impl<S> FileObjectV1Beta for Remapper<S>
where
    S: FileObjectV1Beta,
{
    fn inner(&self) -> Option<&dyn FileObjectV1Beta> {
        Some(&self.inner)
    }

    #[instrument(skip_all)]
    async fn stat(&self, mut request: Request<StatRequestV1Beta>) -> Result<StatResponseV1Beta> {
        request.get_mut().resource_address = self.remap(&request.get_ref().resource_address);
        let mut result = self.inner.stat(request).await?;

        if let Some(ref mut resource_info) = result.resource_info {
            if let Some(ref mut resource_identity) = resource_info.resource_identity {
                resource_identity.encoded_identity = self.remap(&resource_identity.encoded_identity);
            }
        }

        Ok(result)
    }
}

#[async_trait]
impl<S> FileFolderV1Beta for Remapper<S>
where
    S: FileFolderV1Beta,
{
    fn inner(&self) -> Option<&dyn FileFolderV1Beta> {
        Some(&self.inner)
    }

    #[instrument(skip(self, request, streaming))]
    async fn list(
        &self,
        mut request: Request<ListRequestV1Beta>,
        streaming: StreamingOptions,
    ) -> Result<BoxStream<ListResponseWithContinueTokenV1Beta>> {
        if let Some(ref mut folder) = request.get_mut().folder {
            folder.uri = self.remap(&folder.uri);
        }
        let stream = self.inner.list(request, streaming).await?;

        // Transform the stream to remove any results that contain remapped strings
        let remap_map = self.remap_map.clone();
        let transformed_stream = stream.map(move |response| match response {
            Ok(mut list_response) => {
                list_response
                    .response
                    .subfolder_addresses
                    .iter_mut()
                    .for_each(|subfolder| {
                        subfolder.uri = Self::remap_inner(&subfolder.uri, &remap_map);
                    });
                list_response
                    .response
                    .sub_resource_addresses
                    .iter_mut()
                    .for_each(|resource| {
                        *resource = Self::remap_inner(resource, &remap_map);
                    });
                Ok(list_response)
            },
            Err(e) => {
                warn_limited!("RemapperLayer: Error in list stream: {:?}", e);
                Err(e)
            },
        });

        debug_limited!("RemapperLayer: List request stream created");
        Ok(Box::pin(transformed_stream))
    }

    #[instrument(skip(self, request, streaming))]
    async fn list_stat(
        &self,
        mut request: Request<ListStatRequestV1Beta>,
        streaming: StreamingOptions,
    ) -> Result<BoxStream<ListStatResponseWithContinueTokenV1Beta>> {
        if let Some(ref mut folder) = request.get_mut().folder {
            folder.uri = self.remap(&folder.uri);
        }
        let stream = self.inner.list_stat(request, streaming).await?;

        // Transform the stream to remove any results that contain remapped strings
        let remap_map = self.remap_map.clone();
        let transformed_stream = stream.map(move |response| match response {
            Ok(mut list_response) => {
                list_response
                    .response
                    .subfolder_addresses
                    .iter_mut()
                    .for_each(|subfolder| {
                        subfolder.uri = Self::remap_inner(&subfolder.uri, &remap_map);
                    });
                list_response.response.entries.iter_mut().for_each(|entry| {
                    entry.resource_address = Self::remap_inner(&entry.resource_address, &remap_map);
                });
                Ok(list_response)
            },
            Err(e) => {
                warn_limited!("RemapperLayer: Error in list_stat stream: {:?}", e);
                Err(e)
            },
        });

        debug_limited!("RemapperLayer: List_stat request stream created");
        Ok(Box::pin(transformed_stream))
    }
}
