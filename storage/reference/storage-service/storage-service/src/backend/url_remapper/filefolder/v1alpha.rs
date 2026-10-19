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

use std::mem::take;

use futures::{stream, StreamExt};
use storage_core::traits::{filefolder::v1alpha::*, BoxStream, Pagination, Request, Result, StreamingOptions};
use tonic::async_trait;

use crate::{
    backend::{
        metadata::Metadata,
        url_remapper::{
            filefolder::synthetic::{__impl_synthetic_paginable, paginate, parse_synthetic_resume_token, resume_page},
            service::UrlRemapperService,
        },
    },
    common::{
        request::RequestExt,
        url_remapper::{RewriteMode, RewriteResult, RuleTag, UrlRemapper},
    },
    error::InvalidArgumentError,
};

__impl_synthetic_paginable!(
    ListResponseWithContinueToken,
    ListResponse,
    FolderAddress,
    sub_resource_addresses
);
__impl_synthetic_paginable!(
    ListStatResponseWithContinueToken,
    ListStatResponse,
    FolderAddress,
    entries
);

impl<T: Metadata> UrlRemapper<T> {
    fn request_rewrite_folder_address_v1alpha(
        &self,
        address: Option<&FolderAddress>,
    ) -> Result<RewriteResult<FolderAddress>> {
        Ok(self
            .fwd_rewrite_folder_url(
                address
                    .ok_or_else(InvalidArgumentError::folder_address_missing)?
                    .uri
                    .as_ref(),
            )
            .map(|uri| FolderAddress { uri }))
    }

    fn response_rewrite_folder_address_v1alpha(&self, rule_tag: RuleTag, address: FolderAddress) -> FolderAddress {
        self.rev_rewrite_unversioned_url_with_tag(rule_tag, &address.uri, RewriteMode::Fuzzy)
            .map(|uri| FolderAddress { uri })
            .unwrap_or(address)
    }

    fn response_rewrite_list_response_v1alpha(
        &self,
        rule_tag: RuleTag,
        response: ListResponseWithContinueToken,
    ) -> ListResponseWithContinueToken {
        ListResponseWithContinueToken {
            response: ListResponse {
                subfolder_addresses: response
                    .response
                    .subfolder_addresses
                    .into_iter()
                    .map(|address| self.response_rewrite_folder_address_v1alpha(rule_tag, address))
                    .collect(),
                sub_resource_addresses: response
                    .response
                    .sub_resource_addresses
                    .into_iter()
                    .map(|address| {
                        self.rev_rewrite_unversioned_url_with_tag(rule_tag, &address, RewriteMode::Precise)
                            .unwrap_or(address)
                    })
                    .collect(),
            },
            ..response
        }
    }

    async fn response_rewrite_list_stat_response_v1alpha(
        &self,
        rule_tag: RuleTag,
        mut response: ListStatResponseWithContinueToken,
    ) -> Result<ListStatResponseWithContinueToken> {
        let mapped_identities = self
            .rev_rewrite_versioned_url_batch_with_tag(
                rule_tag,
                response.response.entries.iter_mut().map(|list_item| {
                    if let Some(resource_info) = list_item.resource_info.as_mut() {
                        if let Some(resource_identity) = resource_info.resource_identity.as_mut() {
                            return take(&mut resource_identity.encoded_identity);
                        }
                    }

                    String::default()
                }),
            )
            .await?;

        for (list_item, mapped_identity) in response.response.entries.iter_mut().zip(mapped_identities) {
            if let Some(resource_info) = list_item.resource_info.as_mut() {
                if let Some(resource_identity) = resource_info.resource_identity.as_mut() {
                    resource_identity.encoded_identity = mapped_identity;
                }
            }

            if let Some(mapped_address) =
                self.rev_rewrite_unversioned_url_with_tag(rule_tag, &list_item.resource_address, RewriteMode::Precise)
            {
                list_item.resource_address = mapped_address;
            }
        }

        for address in response.response.subfolder_addresses.iter_mut() {
            *address = self.response_rewrite_folder_address_v1alpha(rule_tag, take(address));
        }

        Ok(response)
    }
}

#[async_trait]
impl<T: Metadata, S: FileFolder> FileFolder for UrlRemapperService<T, S> {
    fn inner(&self) -> Option<&dyn FileFolder> {
        Some(&self.inner)
    }

    async fn list(
        &self,
        request: Request<ListRequest>,
        streaming: StreamingOptions,
    ) -> Result<BoxStream<ListResponseWithContinueToken>> {
        let parent_url = request
            .get_ref()
            .folder
            .as_ref()
            .ok_or_else(InvalidArgumentError::folder_address_missing)?
            .uri
            .clone();
        let max_page_size = streaming_max_page_size(&streaming);
        if let Some(skip) = streaming_continuation_handle(&streaming).and_then(parse_synthetic_resume_token) {
            let page = resume_page::<ListResponseWithContinueToken>(
                self.mapper.synthetic_children_of(&parent_url),
                skip,
                max_page_size,
            );
            return Ok(Box::pin(stream::iter([Ok(page)])));
        }
        let rewrite = self
            .mapper
            .request_rewrite_folder_address_v1alpha(request.get_ref().folder.as_ref())?;
        let (rule_tag, request) = match rewrite {
            RewriteResult::Mapped(address, rule_tag) => {
                (Some(rule_tag), request.map(|_| ListRequest { folder: Some(address) }))
            },
            RewriteResult::Unchanged => (None, request),
        };

        let stream = self.inner.list(request, streaming).await?;

        Ok(Box::pin(stream.flat_map({
            let mapper = self.mapper.clone();
            move |item_or_error| {
                let mapper = mapper.clone();
                let parent_url = parent_url.clone();
                stream::iter(match item_or_error {
                    Err(e) => vec![Err(e)],
                    Ok(mut item) => {
                        if let Some(rule_tag) = rule_tag {
                            item = mapper.response_rewrite_list_response_v1alpha(rule_tag, item);
                        }
                        let is_last_page = item.continue_token.as_deref().map(|t| t.is_empty()).unwrap_or(true);
                        if !is_last_page {
                            vec![Ok(item)]
                        } else {
                            paginate(
                                &parent_url,
                                item,
                                mapper.synthetic_children_of(&parent_url),
                                max_page_size,
                            )
                            .into_iter()
                            .map(Ok)
                            .collect()
                        }
                    },
                })
            }
        })))
    }

    async fn list_stat(
        &self,
        request: Request<ListStatRequest>,
        streaming: StreamingOptions,
    ) -> Result<BoxStream<ListStatResponseWithContinueToken>> {
        let parent_url = request
            .get_ref()
            .folder
            .as_ref()
            .ok_or_else(InvalidArgumentError::folder_address_missing)?
            .uri
            .clone();
        let max_page_size = streaming_max_page_size(&streaming);
        if let Some(skip) = streaming_continuation_handle(&streaming).and_then(parse_synthetic_resume_token) {
            let page = resume_page::<ListStatResponseWithContinueToken>(
                self.mapper.synthetic_children_of(&parent_url),
                skip,
                max_page_size,
            );
            return Ok(Box::pin(stream::iter([Ok(page)])));
        }
        let rewrite = self
            .mapper
            .request_rewrite_folder_address_v1alpha(request.get_ref().folder.as_ref())?;
        let (rule_tag, request) = match rewrite {
            RewriteResult::Mapped(address, rule_tag) => (
                Some(rule_tag),
                request.map(|request| ListStatRequest {
                    folder: Some(address),
                    ..request
                }),
            ),
            RewriteResult::Unchanged => (None, request),
        };

        let stream = self.inner.list_stat(request, streaming).await?;

        let processed = stream.then({
            let mapper = self.mapper.clone();
            move |response_or_error| {
                let mapper = mapper.clone();
                async move {
                    let mut item = response_or_error?;
                    if let Some(rule_tag) = rule_tag {
                        item = mapper
                            .response_rewrite_list_stat_response_v1alpha(rule_tag, item)
                            .await?;
                    }
                    Ok::<_, storage_core::traits::StorageError>(item)
                }
            }
        });

        Ok(Box::pin(processed.flat_map({
            let mapper = self.mapper.clone();
            move |item_or_error| {
                let mapper = mapper.clone();
                let parent_url = parent_url.clone();
                stream::iter(match item_or_error {
                    Err(e) => vec![Err(e)],
                    Ok(item) => {
                        let is_last_page = item.continue_token.as_deref().map(|t| t.is_empty()).unwrap_or(true);
                        if !is_last_page {
                            vec![Ok(item)]
                        } else {
                            paginate(
                                &parent_url,
                                item,
                                mapper.synthetic_children_of(&parent_url),
                                max_page_size,
                            )
                            .into_iter()
                            .map(Ok)
                            .collect()
                        }
                    },
                })
            }
        })))
    }

    async fn delete_folder(&self, request: Request<DeleteFolderRequest>) -> Result<DeleteFolderResponse> {
        let folder_uri = request
            .get_ref()
            .folder
            .as_ref()
            .ok_or_else(InvalidArgumentError::folder_address_missing)?
            .uri
            .as_str();
        // Reject deletes targeting a mount-point itself: rewriting would
        // silently delete the mounted target bucket. Children of the
        // mount-point are unaffected and continue to delete normally.
        if self.mapper.is_mount_point(folder_uri) {
            return Err(InvalidArgumentError::new("mount-point folder cannot be deleted").into());
        }

        let RewriteResult::Mapped(address, _) = self
            .mapper
            .request_rewrite_folder_address_v1alpha(request.get_ref().folder.as_ref())?
        else {
            return self.inner.delete_folder(request).await;
        };

        self.inner
            .delete_folder(request.map(|_| DeleteFolderRequest { folder: Some(address) }))
            .await
    }

    async fn create_folder(&self, request: Request<CreateFolderRequest>) -> Result<CreateFolderResponse> {
        let folder_uri = request
            .get_ref()
            .folder
            .as_ref()
            .ok_or_else(InvalidArgumentError::folder_address_missing)?
            .uri
            .as_str();
        // Reject creates targeting a mount-point itself: the mount-point
        // is defined by the rewrite rule and creating a folder there
        // would either no-op or write to the mounted target's root.
        if self.mapper.is_mount_point(folder_uri) {
            return Err(InvalidArgumentError::new("mount-point folder cannot be created").into());
        }

        let RewriteResult::Mapped(address, _) = self
            .mapper
            .request_rewrite_folder_address_v1alpha(request.get_ref().folder.as_ref())?
        else {
            return self.inner.create_folder(request).await;
        };

        self.inner
            .create_folder(request.map(|_| CreateFolderRequest { folder: Some(address) }))
            .await
    }

    async fn get_folder_mode(&self, request: Request<GetFolderModeRequest>) -> Result<GetFolderModeResponse> {
        let RewriteResult::Mapped(address, _) = self
            .mapper
            .request_rewrite_folder_address_v1alpha(request.get_ref().folder.as_ref())?
        else {
            return self.inner.get_folder_mode(request).await;
        };

        self.inner
            .get_folder_mode(request.map(|_| GetFolderModeRequest { folder: Some(address) }))
            .await
    }
}

fn streaming_max_page_size(opts: &StreamingOptions) -> Option<usize> {
    match opts {
        StreamingOptions::Paginated(Pagination { max_page_size, .. }) => max_page_size.map(|n| n as usize),
        StreamingOptions::Streamed => None,
    }
}

fn streaming_continuation_handle(opts: &StreamingOptions) -> Option<&str> {
    match opts {
        StreamingOptions::Paginated(Pagination {
            continuation_handle, ..
        }) => continuation_handle.as_deref(),
        StreamingOptions::Streamed => None,
    }
}

#[cfg(test)]
mod tests {
    use futures::StreamExt;
    use rstest::rstest;
    use storage_core::traits::{filefolder::v1alpha::*, BoxStream, Pagination, Request, Result, StreamingOptions};
    use tonic::async_trait;

    use crate::{
        backend::url_remapper::{filefolder::synthetic::synthetic_resume_token, service::UrlRemapperService},
        common::url_remapper::test_helpers::make_remapper,
    };

    fn streaming(max_page_size: Option<u32>) -> StreamingOptions {
        match max_page_size {
            Some(n) => StreamingOptions::Paginated(Pagination {
                continuation_handle: None,
                max_page_size: Some(n),
            }),
            None => StreamingOptions::Streamed,
        }
    }

    fn folder(uri: &str) -> FolderAddress {
        FolderAddress { uri: uri.to_owned() }
    }

    fn list_page(
        subfolders: Vec<&str>,
        files: Vec<&str>,
        continue_token: Option<&str>,
    ) -> ListResponseWithContinueToken {
        ListResponseWithContinueToken {
            response: ListResponse {
                subfolder_addresses: subfolders.into_iter().map(folder).collect(),
                sub_resource_addresses: files.into_iter().map(String::from).collect(),
            },
            continue_token: continue_token.map(String::from),
        }
    }

    fn list_stat_page(
        subfolders: Vec<&str>,
        entries: Vec<&str>,
        continue_token: Option<&str>,
    ) -> ListStatResponseWithContinueToken {
        ListStatResponseWithContinueToken {
            response: ListStatResponse {
                subfolder_addresses: subfolders.into_iter().map(folder).collect(),
                entries: entries
                    .into_iter()
                    .map(|s| ListItem {
                        resource_address: s.to_string(),
                        resource_info: None,
                    })
                    .collect(),
            },
            continue_token: continue_token.map(String::from),
        }
    }

    struct MockListInner {
        pages: Vec<ListResponseWithContinueToken>,
    }

    impl MockListInner {
        fn with_pages(pages: Vec<ListResponseWithContinueToken>) -> Self {
            Self { pages }
        }
    }

    #[async_trait]
    impl FileFolder for MockListInner {
        fn inner(&self) -> Option<&dyn FileFolder> {
            None
        }

        async fn list(
            &self,
            _request: Request<ListRequest>,
            _streaming: StreamingOptions,
        ) -> Result<BoxStream<ListResponseWithContinueToken>> {
            let pages = self.pages.clone();
            Ok(Box::pin(futures::stream::iter(pages.into_iter().map(Ok))))
        }

        async fn list_stat(
            &self,
            _request: Request<ListStatRequest>,
            _streaming: StreamingOptions,
        ) -> Result<BoxStream<ListStatResponseWithContinueToken>> {
            unreachable!("list_stat is exercised by MockListStatInner")
        }

        async fn delete_folder(&self, _: Request<DeleteFolderRequest>) -> Result<DeleteFolderResponse> {
            unreachable!()
        }

        async fn create_folder(&self, _: Request<CreateFolderRequest>) -> Result<CreateFolderResponse> {
            unreachable!()
        }

        async fn get_folder_mode(&self, _: Request<GetFolderModeRequest>) -> Result<GetFolderModeResponse> {
            unreachable!()
        }
    }

    struct MockListStatInner {
        pages: Vec<ListStatResponseWithContinueToken>,
    }

    impl MockListStatInner {
        fn with_pages(pages: Vec<ListStatResponseWithContinueToken>) -> Self {
            Self { pages }
        }
    }

    #[async_trait]
    impl FileFolder for MockListStatInner {
        fn inner(&self) -> Option<&dyn FileFolder> {
            None
        }

        async fn list(
            &self,
            _request: Request<ListRequest>,
            _streaming: StreamingOptions,
        ) -> Result<BoxStream<ListResponseWithContinueToken>> {
            unreachable!("list is exercised by MockListInner")
        }

        async fn list_stat(
            &self,
            _request: Request<ListStatRequest>,
            _streaming: StreamingOptions,
        ) -> Result<BoxStream<ListStatResponseWithContinueToken>> {
            let pages = self.pages.clone();
            Ok(Box::pin(futures::stream::iter(pages.into_iter().map(Ok))))
        }

        async fn delete_folder(&self, _: Request<DeleteFolderRequest>) -> Result<DeleteFolderResponse> {
            unreachable!()
        }

        async fn create_folder(&self, _: Request<CreateFolderRequest>) -> Result<CreateFolderResponse> {
            unreachable!()
        }

        async fn get_folder_mode(&self, _: Request<GetFolderModeRequest>) -> Result<GetFolderModeResponse> {
            unreachable!()
        }
    }

    fn rules(n: usize) -> Vec<(&'static str, &'static str)> {
        let mut out = vec![("omniverse://host/", "https://backend.example/root/")];
        let children: &[(&str, &str)] = &[
            ("omniverse://host/m1/", "https://backend.example/b1/"),
            ("omniverse://host/m2/", "https://backend.example/b2/"),
            ("omniverse://host/m3/", "https://backend.example/b3/"),
            ("omniverse://host/m4/", "https://backend.example/b4/"),
            ("omniverse://host/m5/", "https://backend.example/b5/"),
            ("omniverse://host/m6/", "https://backend.example/b6/"),
            ("omniverse://host/m7/", "https://backend.example/b7/"),
        ];
        out.extend_from_slice(&children[..n]);
        out
    }

    async fn collect_list_pages(
        service: &UrlRemapperService<crate::common::url_remapper::test_helpers::UnreachableMetadata, MockListInner>,
        max_page_size: Option<u32>,
    ) -> Vec<ListResponseWithContinueToken> {
        let request = Request::new(ListRequest {
            folder: Some(folder("omniverse://host/")),
        });
        let stream = service.list(request, streaming(max_page_size)).await.expect("list ok");
        let pages: Vec<_> = stream.collect().await;
        let pages: Vec<ListResponseWithContinueToken> = pages
            .into_iter()
            .map(|p| p.expect("no errors expected in these tests"))
            .collect();
        if let Some(cap) = max_page_size {
            for (idx, page) in pages.iter().enumerate() {
                let size = page.response.subfolder_addresses.len() + page.response.sub_resource_addresses.len();
                assert!(size <= cap as usize, "page {idx} has {size} entries, exceeds cap {cap}",);
            }
        }
        pages
    }

    async fn collect_list_stat_pages(
        service: &UrlRemapperService<crate::common::url_remapper::test_helpers::UnreachableMetadata, MockListStatInner>,
        max_page_size: Option<u32>,
    ) -> Vec<ListStatResponseWithContinueToken> {
        let request = Request::new(ListStatRequest {
            folder: Some(folder("omniverse://host/")),
            include_native_metadata: false,
        });
        let stream = service
            .list_stat(request, streaming(max_page_size))
            .await
            .expect("list_stat ok");
        let pages: Vec<_> = stream.collect().await;
        let pages: Vec<ListStatResponseWithContinueToken> = pages
            .into_iter()
            .map(|p| p.expect("no errors expected in these tests"))
            .collect();
        if let Some(cap) = max_page_size {
            for (idx, page) in pages.iter().enumerate() {
                let size = page.response.subfolder_addresses.len() + page.response.entries.len();
                assert!(
                    size <= cap as usize,
                    "list_stat page {idx} has {size} entries, exceeds cap {cap}",
                );
            }
        }
        pages
    }

    /// `list` integration: synthetic-child injection respects `max_page_size`.
    /// Each case sets up a different inner-final-page size and synthetic-count
    /// vs. cap so we exercise: fits-in-place, exact-fit, one-overflow,
    /// multi-overflow, and `Streamed` (no cap).
    #[rstest]
    // (real_count, synthetic_rules, max_page_size, expected_page_sizes, expected_continue_tokens)
    //
    // Each non-final overflow page carries a `urlremapper:syn:<skip>`
    // resume token; `skip` is the count of synthetic entries already
    // emitted to the client by that point in the stream.
    //
    // Case A: synthetics fit on the final inner page.
    #[case(2, 1, Some(5), vec![3], vec![None])]
    // Case B: exact fit -- 3 real + 2 synthetic == cap 5.
    #[case(3, 2, Some(5), vec![5], vec![None])]
    // Case C: 1 overflow page (4 real + 3 synthetic with cap 5 -> [5, 2]).
    // Page 1: 4 real + 1 synth, skip=1 marker for the remaining 2 synth.
    #[case(4, 3, Some(5), vec![5, 2], vec![Some(synthetic_resume_token(1)), None])]
    // Case D: 2 overflow pages (1 real + 7 synthetic with cap 3 -> [3, 3, 2]).
    // Page 1: 1 real + 2 synth (skip=2); Page 2: 3 synth (skip=5); Page 3: 2 synth (end).
    #[case(1, 7, Some(3), vec![3, 3, 2],
        vec![Some(synthetic_resume_token(2)),
             Some(synthetic_resume_token(5)),
             None])]
    // Case E: inner page already at cap -- all synthetics overflow.
    // Page 1: 3 real (skip=0); Page 2: 2 synth (end).
    #[case(3, 2, Some(3), vec![3, 2], vec![Some(synthetic_resume_token(0)), None])]
    // Case F: no cap (Streamed) -- everything in a single page.
    #[case(10, 5, None, vec![15], vec![None])]
    // Case G: cap = 1 -- forces overflow even for a single synthetic.
    // Page 1: 1 synth (skip=1); Page 2: 1 synth (skip=2); Page 3: 1 synth (end).
    #[case(0, 3, Some(1), vec![1, 1, 1],
        vec![Some(synthetic_resume_token(1)),
             Some(synthetic_resume_token(2)),
             None])]
    #[tokio::test]
    async fn list_synthetic_pagination_honours_max_page_size(
        #[case] real_count: usize,
        #[case] synth_count: usize,
        #[case] max_page_size: Option<u32>,
        #[case] expected_page_sizes: Vec<usize>,
        #[case] expected_continue_tokens: Vec<Option<String>>,
    ) {
        let real_subfolders: Vec<String> = (0..real_count).map(|i| format!("omniverse://host/real{i}/")).collect();
        let real_refs: Vec<&str> = real_subfolders.iter().map(|s| s.as_str()).collect();
        let mapper = make_remapper(&rules(synth_count), true);
        let inner = MockListInner::with_pages(vec![list_page(real_refs, vec![], None)]);
        let service = UrlRemapperService {
            mapper,
            address_filter: None,
            inner,
        };

        let pages = collect_list_pages(&service, max_page_size).await;

        let actual_sizes: Vec<usize> = pages
            .iter()
            .map(|p| p.response.subfolder_addresses.len() + p.response.sub_resource_addresses.len())
            .collect();
        let actual_tokens: Vec<Option<String>> = pages.iter().map(|p| p.continue_token.clone()).collect();
        assert_eq!(actual_sizes, expected_page_sizes, "page sizes mismatch");
        assert_eq!(actual_tokens, expected_continue_tokens, "continue tokens mismatch");

        // Every original real entry must appear exactly once across all pages.
        let all: Vec<String> = pages
            .iter()
            .flat_map(|p| p.response.subfolder_addresses.iter().map(|a| a.uri.clone()))
            .collect();
        for real in &real_subfolders {
            assert_eq!(all.iter().filter(|u| *u == real).count(), 1, "missing/duplicate {real}");
        }
        // Every synthetic URL must appear exactly once across all pages.
        for i in 1..=synth_count {
            let synth = format!("omniverse://host/m{i}/");
            assert_eq!(
                all.iter().filter(|u| **u == synth).count(),
                1,
                "missing/duplicate synthetic {synth}",
            );
        }
    }

    /// Non-final inner pages pass through verbatim; only the final page
    /// triggers synthetic injection.
    #[tokio::test]
    async fn list_non_final_pages_pass_through() {
        let mapper = make_remapper(&rules(2), true);
        let inner = MockListInner::with_pages(vec![
            list_page(vec!["omniverse://host/a/"], vec![], Some("next")),
            list_page(vec!["omniverse://host/b/"], vec![], None),
        ]);
        let service = UrlRemapperService {
            mapper,
            address_filter: None,
            inner,
        };

        let pages = collect_list_pages(&service, Some(10)).await;

        assert_eq!(pages.len(), 2, "non-final page should not be split");
        assert_eq!(pages[0].continue_token.as_deref(), Some("next"));
        assert_eq!(
            pages[0].response.subfolder_addresses.len(),
            1,
            "non-final page untouched"
        );

        // Final page got the 2 synthetic mounts appended.
        let final_uris: Vec<&str> = pages[1]
            .response
            .subfolder_addresses
            .iter()
            .map(|a| a.uri.as_str())
            .collect();
        assert!(final_uris.contains(&"omniverse://host/b/"));
        assert!(final_uris.contains(&"omniverse://host/m1/"));
        assert!(final_uris.contains(&"omniverse://host/m2/"));
    }

    /// A real subfolder that matches a synthetic URL is dropped (rule wins).
    #[tokio::test]
    async fn list_real_synthetic_collision_drops_real() {
        let mapper = make_remapper(&rules(1), true);
        let inner = MockListInner::with_pages(vec![list_page(
            vec!["omniverse://host/m1/", "omniverse://host/other/"],
            vec![],
            None,
        )]);
        let service = UrlRemapperService {
            mapper,
            address_filter: None,
            inner,
        };

        let pages = collect_list_pages(&service, Some(10)).await;
        assert_eq!(pages.len(), 1);
        let uris: Vec<&str> = pages[0]
            .response
            .subfolder_addresses
            .iter()
            .map(|a| a.uri.as_str())
            .collect();
        let m1_count = uris.iter().filter(|u| **u == "omniverse://host/m1/").count();
        assert_eq!(m1_count, 1, "colliding entry must appear exactly once");
        assert!(uris.contains(&"omniverse://host/other/"));
    }

    /// Listing `parent` and `parent/` (with and without trailing slash)
    /// must produce the same set of subfolders. Guards against any
    /// path-normalisation regression that would treat the two as
    /// distinct trie keys.
    #[tokio::test]
    async fn list_with_and_without_trailing_slash_matches() {
        let mapper = make_remapper(&rules(2), true);
        let with_slash_inner = MockListInner::with_pages(vec![list_page(vec![], vec![], None)]);
        let with_slash_service = UrlRemapperService {
            mapper: mapper.clone(),
            address_filter: None,
            inner: with_slash_inner,
        };
        let request_with = Request::new(ListRequest {
            folder: Some(folder("omniverse://host/")),
        });
        let stream_with = with_slash_service
            .list(request_with, streaming(None))
            .await
            .expect("list ok");
        let pages_with: Vec<_> = stream_with.collect().await;
        let uris_with: Vec<String> = pages_with
            .into_iter()
            .flat_map(|p| p.expect("ok").response.subfolder_addresses.into_iter().map(|a| a.uri))
            .collect();

        let without_slash_inner = MockListInner::with_pages(vec![list_page(vec![], vec![], None)]);
        let without_slash_service = UrlRemapperService {
            mapper,
            address_filter: None,
            inner: without_slash_inner,
        };
        let request_without = Request::new(ListRequest {
            folder: Some(folder("omniverse://host")),
        });
        let stream_without = without_slash_service
            .list(request_without, streaming(None))
            .await
            .expect("list ok");
        let pages_without: Vec<_> = stream_without.collect().await;
        let uris_without: Vec<String> = pages_without
            .into_iter()
            .flat_map(|p| p.expect("ok").response.subfolder_addresses.into_iter().map(|a| a.uri))
            .collect();

        assert_eq!(uris_with, uris_without, "trailing-slash and bare parent must agree");
    }

    /// `list_stat` counterpart of `list_synthetic_pagination_honours_max_page_size`.
    /// Mirrors the same scenarios since the pagination logic is symmetric.
    #[rstest]
    #[case(2, 1, Some(5), vec![3], vec![None])]
    #[case(3, 2, Some(5), vec![5], vec![None])]
    #[case(4, 3, Some(5), vec![5, 2], vec![Some(synthetic_resume_token(1)), None])]
    #[case(1, 7, Some(3), vec![3, 3, 2],
        vec![Some(synthetic_resume_token(2)),
             Some(synthetic_resume_token(5)),
             None])]
    #[case(3, 2, Some(3), vec![3, 2], vec![Some(synthetic_resume_token(0)), None])]
    #[case(10, 5, None, vec![15], vec![None])]
    #[tokio::test]
    async fn list_stat_synthetic_pagination_honours_max_page_size(
        #[case] real_entries: usize,
        #[case] synth_count: usize,
        #[case] max_page_size: Option<u32>,
        #[case] expected_page_sizes: Vec<usize>,
        #[case] expected_continue_tokens: Vec<Option<String>>,
    ) {
        let real_paths: Vec<String> = (0..real_entries).map(|i| format!("omniverse://host/file{i}")).collect();
        let real_refs: Vec<&str> = real_paths.iter().map(|s| s.as_str()).collect();
        let mapper = make_remapper(&rules(synth_count), true);
        let inner = MockListStatInner::with_pages(vec![list_stat_page(vec![], real_refs, None)]);
        let service = UrlRemapperService {
            mapper,
            address_filter: None,
            inner,
        };

        let pages = collect_list_stat_pages(&service, max_page_size).await;

        let actual_sizes: Vec<usize> = pages
            .iter()
            .map(|p| p.response.subfolder_addresses.len() + p.response.entries.len())
            .collect();
        let actual_tokens: Vec<Option<String>> = pages.iter().map(|p| p.continue_token.clone()).collect();
        assert_eq!(actual_sizes, expected_page_sizes, "page sizes mismatch");
        assert_eq!(actual_tokens, expected_continue_tokens, "continue tokens mismatch");

        // Each synthetic URL appears exactly once.
        for i in 1..=synth_count {
            let synth = format!("omniverse://host/m{i}/");
            let count: usize = pages
                .iter()
                .map(|p| p.response.subfolder_addresses.iter().filter(|a| a.uri == synth).count())
                .sum();
            assert_eq!(count, 1, "missing/duplicate synthetic {synth}");
        }
    }
}
