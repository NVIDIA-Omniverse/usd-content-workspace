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

//! Integration tests for the manifest-overlay layer ordering (v1beta).
//!
//! The v1beta counterpart of `manifest_overlay_v1alpha`, covering the separate v1beta manifest
//! forks (`storage_core::manifest::{fileobject, filefolder, versioning}::v1beta`). It exercises the
//! *real* `storage-service` binary spawned by the test harness against Azure (Azurite in CI) +
//! Postgres + the in-harness permission mock, so it covers the actual `main.rs` decorator order
//! rather than a layer in isolation.
//!
//! Topology under test, with a request flowing downward:
//!
//! ```text
//! url_remapper -> url_normalizer -> created_timestamp -> PERMISSIONS -> MANIFEST -> caches -> backend
//!                                                                          |
//!             resolver branch:   url_remapper -> url_normalizer -> PERMISSIONS -> caches -> backend
//! ```
//!
//! `permissions` sits *above* the manifest layer, so the client's own request address is authorized
//! before the manifest layer can answer it out of its manifest cache. The manifest layer's two
//! internal reads (the `.manifest` body, and the read-by-identity of a backing entry) take the
//! separate resolver branch instead, which carries its own remapper and its own permission layer:
//! `omniverse://` entry identities still resolve to physical URLs, and every backing read is still
//! authorized against the reader.
//!
//! The warm-cache tests each make an authorized request before dropping the grants
//! and repeating it. Warming is the point: on a cold cache the `.manifest` read itself
//! would be denied, so the repeat would fail for a reason that says nothing about the request path.
//!
//! The permission mock is principal-agnostic (allow/deny keyed on `(kind, uri, action)`, not on the
//! caller), so "an unauthorized reader" is expressed as "no rule seeded": an unseeded resource
//! evaluates to `Missing`, which the permission layer denies for these paths. Grants live in a
//! `RuleSetGuard` whose rules vanish when it drops.
//!
//! All tests share a single `#[once]` service (matching the other harness-backed test files) so the
//! Postgres metadata schema is initialised exactly once per test binary, and the rule sets are
//! global to that service's permission `Context`; every test therefore tags its addresses with a
//! random suffix so neither its workspaces nor its grants can collide with a sibling running
//! concurrently. The remapper is configured on the shared service; tests that address the backend
//! namespace directly pass through it untouched, while the remap tests use the mapped `omniverse://`
//! namespace and authorize against the remapped backend URLs. Requires the external backends
//! (Azure + Postgres) and runs under `cargo test --features test` in CI.

mod common;

use bytes::Bytes;
use rstest::{fixture, rstest};
use serde_json::json;
use tonic::Code;

use common::clients::filefolder::types::v1beta::{FolderAddress, ListRequest, ListStatRequest};
use common::clients::fileobject::types::v1beta::{EnumerateRequest, StatRequest, UnwrapIdentityExt, WriteParameters};
use common::clients::versioning::types::v1beta::EnumerateVersionsRequest;
use common::random::get_random_alphanumeric_string;
use common::service::Service as StorageService;
use common::types::Interface;
use storage_service_test::mocks::permissions::{grpc::Service as GrpcPermissionService, Context};

/// Mapped (client-facing) `omniverse://` prefix used for the remap test, and the backend prefix it
/// resolves to. Chosen so the container-root backend URLs never fall under the remapper's
/// rules (they pass through untouched).
const MAPPED_PREFIX: &str = "manifest-remap/";
const ORIGINAL_PREFIX: &str = "manifest-remap-original/";

/// A spawned `storage-service` with the permission mock and URL remapper wired in, plus the shared
/// permission context used to seed per-`(resource, action)` decisions.
struct OverlayFixture {
    service: StorageService,
    permission_context: Context,
    // Keep the mock alive for the lifetime of the spawned service.
    _permission_service: GrpcPermissionService,
}

#[fixture]
#[once]
fn overlay_fixture() -> OverlayFixture {
    let permission_context = Context::new();
    let permission_service = GrpcPermissionService::new(permission_context.clone());
    let service = StorageService::builder()
        .enable_az(true)
        .enable_postgres_metadata()
        .enable_permissions("grpc", permission_service.endpoint_url(), false)
        .enable_az_url_remapper(MAPPED_PREFIX, ORIGINAL_PREFIX)
        .start();

    OverlayFixture {
        service,
        permission_context,
        _permission_service: permission_service,
    }
}

/// Build a single-entry `.manifest` body (v1beta format) mapping `relative_path` to
/// `resource_identity`, matching the shape the `ManifestLayer` parses.
fn manifest_json(relative_path: &str, resource_identity: &str) -> Bytes {
    Bytes::from(
        serde_json::to_vec(&json!({
            "format_version": { "version_string": "1.0" },
            "storage_api_version": "v1beta",
            "files": [
                { "relative_path": relative_path, "resource_identity": resource_identity },
            ],
        }))
        .expect("manifest JSON"),
    )
}

/// Write `content` to `address` through the fileobject v1beta client, returning the resulting
/// resource identity. Used both for backing objects and for the `.manifest` body itself.
async fn write_object(
    client: &common::clients::fileobject::v1beta::Client,
    address: impl Into<String>,
    content: Bytes,
) -> String {
    let (resource_info, _) = client
        .write_object(
            WriteParameters {
                destination_resource_address: address.into(),
                data_object_size: content.len() as u64,
                ..Default::default()
            },
            content,
        )
        .await
        .expect("object write failed");

    resource_info.unwrap_identity()
}

/// The continuation handle the manifest layer itself hands out once the backend phase of a workspace
/// listing is exhausted; the serialization matches `WorkspaceListToken` / `WorkspaceEnumerateToken`
/// in storage-core, which key off the `phase` tag and ignore field order. Supplying it as the *first*
/// page is the forgery: it asks the manifest layer for its overlay page directly, so the listing
/// never touches the backend.
fn manifest_phase_continuation_handle() -> String {
    json!({ "phase": "manifest", "index": 0 }).to_string()
}

/// remap fix. A `.manifest` storing an `omniverse://` identity resolves: the manifest
/// layer's read-by-identity takes the resolver branch, whose own remapper turns the identity into
/// the backend URL, and the object is served. Permissions are authorized on the remapped backend
/// URLs so this isolates the remap behaviour (never `PermissionDenied`).
#[rstest]
#[test_log::test(tokio::test)]
async fn overlay_read_remaps_omniverse_identity_and_serves(overlay_fixture: &OverlayFixture) {
    let service = &overlay_fixture.service;
    let context = &overlay_fixture.permission_context;
    let client = service.fileobject_v1beta_client(Interface::Grpc).await;
    let tag = get_random_alphanumeric_string(8).to_lowercase();

    // Client-facing (mapped, omniverse://) addresses, and the backend (original) addresses the
    // remapper resolves them to - the latter is what the permissions layer authorizes against.
    let backing_mapped = service
        .remapper_mapped_url_builder()
        .path(format!("data-{tag}/scene.usda"))
        .to_string();
    let backing_backend = service
        .remapper_original_url_builder()
        .path(format!("data-{tag}/scene.usda"))
        .to_string();
    let workspace = format!("overlay-{tag}.wkspc");
    let manifest_mapped = service
        .remapper_mapped_url_builder()
        .path(format!("{workspace}/.manifest"))
        .to_string();
    let manifest_backend = service
        .remapper_original_url_builder()
        .path(format!("{workspace}/.manifest"))
        .to_string();
    let overlay_mapped = service
        .remapper_mapped_url_builder()
        .path(format!("{workspace}/scene.usda"))
        .to_string();
    let overlay_backend = service
        .remapper_original_url_builder()
        .path(format!("{workspace}/scene.usda"))
        .to_string();

    // Authorize the backend URLs the manifest layer's inner reads/writes resolve to.
    let rules = context.new_rule_set();
    rules.allow_object_write(&backing_backend);
    rules.allow_object_write(&manifest_backend);
    rules.allow_object_read(&backing_backend);
    rules.allow_object_read(&manifest_backend);
    // `permissions` now sits above the manifest layer, so the overlay read is authorized on the
    // client's own address (already resolved to the backend URL by the remapper above it) before
    // the manifest layer sees it. Without this grant the test would stop at `PermissionDenied` and
    // never reach the remap behaviour it exists to isolate.
    rules.allow_object_read(&overlay_backend);

    // Backing object written via the mapped namespace; its write-response identity is rewritten back
    // into the mapped namespace, so it is an omniverse:// identity - the shape that failed to remap
    // before the reorder.
    let content = Bytes::from_static(b"remapped-overlay-backing-bytes");
    let omniverse_identity = write_object(&client, &backing_mapped, content.clone()).await;
    assert!(
        omniverse_identity.starts_with("omniverse://"),
        "expected a mapped omniverse:// identity, got {omniverse_identity}"
    );

    // Overlay workspace `.manifest` referencing the backing object by its omniverse:// identity.
    write_object(
        &client,
        &manifest_mapped,
        manifest_json("scene.usda", &omniverse_identity),
    )
    .await;

    // Read the overlay entry: the inner read-by-identity is remapped down to the backend and serves.
    let (_info, served) = client
        .read_object_from_address(overlay_mapped)
        .await
        .expect("overlay read should serve the remapped backing object");
    assert_eq!(served, content);
}

/// bypass closed. The manifest layer's backing-object read flows through the resolver
/// branch's `permissions`, so an overlay entry whose backing object is unauthorized is forbidden,
/// while an authorized backing read serves. Addresses the backend namespace directly (remapper
/// passes it through), keeping the test focused on authorization.
#[rstest]
#[test_log::test(tokio::test)]
async fn overlay_backing_read_is_authorized_against_the_reader(overlay_fixture: &OverlayFixture) {
    let service = &overlay_fixture.service;
    let context = &overlay_fixture.permission_context;
    let client = service.fileobject_v1beta_client(Interface::Grpc).await;
    let tag = get_random_alphanumeric_string(8).to_lowercase();

    // Backing object at a non-overlay path; the `.manifest` + overlay entry live under `<ws>.wkspc`.
    let backing_address = service
        .az_url_builder()
        .path(format!("data-{tag}/scene.usda"))
        .to_string();
    let workspace = format!("overlay-{tag}.wkspc");
    let manifest_address = service
        .az_url_builder()
        .path(format!("{workspace}/.manifest"))
        .to_string();
    let overlay_address = service
        .az_url_builder()
        .path(format!("{workspace}/scene.usda"))
        .to_string();
    let content = Bytes::from_static(b"secret-overlay-backing-bytes");

    // Setup: authorize only the writes, and record the backing object's backend identity.
    let backing_identity = {
        let setup = context.new_rule_set();
        setup.allow_object_write(&backing_address);
        setup.allow_object_write(&manifest_address);
        let backing_identity = write_object(&client, &backing_address, content.clone()).await;
        write_object(
            &client,
            &manifest_address,
            manifest_json("scene.usda", &backing_identity),
        )
        .await;
        backing_identity
    };
    // The manifest stores a backend-URL identity that parses to `backing_address`.
    assert!(
        backing_identity.starts_with(&backing_address),
        "expected a backend-URL identity for {backing_address}, got {backing_identity}"
    );

    // Bypass CLOSED: allow reading the `.manifest`, but leave the backing object unseeded
    // (mock default = deny). The inner backing read is now authorized -> PermissionDenied.
    {
        let rules = context.new_rule_set();
        rules.allow_object_read(&manifest_address);
        // `permissions` now sits above the manifest layer, so the overlay address is authorized
        // first. Without this grant the request would be denied there and the assertion below would
        // no longer say anything about the backing-object check it exists to isolate.
        rules.allow_object_read(&overlay_address);
        let status = client
            .read_object_from_address(overlay_address.clone())
            .await
            .expect_err("overlay read must be forbidden when the backing object is unauthorized");
        assert_eq!(status.code(), Code::PermissionDenied);
    }

    // Authorized reader succeeds: same overlay, now with the backing read also allowed.
    {
        let rules = context.new_rule_set();
        rules.allow_object_read(&manifest_address);
        rules.allow_object_read(&backing_address);
        // See above: the overlay address is authorized above the manifest layer.
        rules.allow_object_read(&overlay_address);
        let (_info, served) = client
            .read_object_from_address(overlay_address)
            .await
            .expect("overlay read should serve when the backing read is authorized");
        assert_eq!(served, content);
    }
}

/// control. A plain (non-overlay) read is unchanged: it authorizes on the backend-URL
/// namespace, confirming the manifest layer is a correct pass-through for an address that names no
/// workspace.
#[rstest]
#[test_log::test(tokio::test)]
async fn normal_non_overlay_read_still_authorizes_on_backend_namespace(overlay_fixture: &OverlayFixture) {
    let service = &overlay_fixture.service;
    let context = &overlay_fixture.permission_context;
    let client = service.fileobject_v1beta_client(Interface::Grpc).await;
    let tag = get_random_alphanumeric_string(8).to_lowercase();

    let address = service
        .az_url_builder()
        .path(format!("plain-{tag}/scene.usda"))
        .to_string();
    let content = Bytes::from_static(b"plain-object-bytes");

    let identity = {
        let setup = context.new_rule_set();
        setup.allow_object_write(&address);
        write_object(&client, &address, content.clone()).await
    };

    // Without a read allow, the plain read is forbidden (permissions still gate normal reads).
    {
        let status = client
            .read_object(identity.clone())
            .await
            .expect_err("plain read must be forbidden without a read allow");
        assert_eq!(status.code(), Code::PermissionDenied);
    }

    // With a read allow on the backend URL, the plain read serves.
    {
        let rules = context.new_rule_set();
        rules.allow_object_read(&address);
        let (_metadata, served) = client
            .read_object(identity)
            .await
            .expect("plain read should serve when authorized");
        assert_eq!(served, content);
    }
}

/// Regression, `Stat`. A `Stat` on an overlay entry is answered from the manifest alone,
/// so with the manifest layer above `permissions` a manifest-cache hit returned the entry without
/// ever reaching an authorization check. Warm the cache with an authorized `Stat`, drop the grants,
/// and repeat: the second `Stat` must be forbidden instead of served out of the cache.
#[rstest]
#[test_log::test(tokio::test)]
async fn overlay_stat_is_authorized_on_a_warm_manifest_cache(overlay_fixture: &OverlayFixture) {
    let service = &overlay_fixture.service;
    let context = &overlay_fixture.permission_context;
    let client = service.fileobject_v1beta_client(Interface::Grpc).await;
    let tag = get_random_alphanumeric_string(8).to_lowercase();

    let backing_address = service
        .az_url_builder()
        .path(format!("data-{tag}/scene.usda"))
        .to_string();
    let workspace = format!("overlay-{tag}.wkspc");
    let manifest_address = service
        .az_url_builder()
        .path(format!("{workspace}/.manifest"))
        .to_string();
    let overlay_address = service
        .az_url_builder()
        .path(format!("{workspace}/scene.usda"))
        .to_string();

    {
        let setup = context.new_rule_set();
        setup.allow_object_write(&backing_address);
        setup.allow_object_write(&manifest_address);
        let backing_identity =
            write_object(&client, &backing_address, Bytes::from_static(b"warm-cache-stat-bytes")).await;
        write_object(
            &client,
            &manifest_address,
            manifest_json("scene.usda", &backing_identity),
        )
        .await;
    }

    // Authorized stat. Besides asserting the overlay entry is served, this leaves the parsed
    // manifest in the service's manifest cache, so the denial below cannot be explained away as a
    // forbidden cold-cache `.manifest` read.
    {
        let rules = context.new_rule_set();
        // The resolver branch reads the `.manifest` body while the cache is cold.
        rules.allow_object_read(&manifest_address);
        // The client's own address is authorized above the manifest layer.
        rules.allow_object_read(&overlay_address);

        let identity = client
            .stat(StatRequest {
                resource_address: overlay_address.clone(),
            })
            .await
            .expect("authorized overlay stat should serve from the manifest")
            .resource_info
            .expect("stat response is missing the resource info")
            .unwrap_identity();
        assert!(
            identity.starts_with(&backing_address),
            "expected the manifest entry identity for {backing_address}, got {identity}"
        );
    }

    // Same entry, same warm cache, no grants.
    let status = client
        .stat(StatRequest {
            resource_address: overlay_address,
        })
        .await
        .expect_err("overlay stat must be forbidden once the grants are gone");
    assert_eq!(status.code(), Code::PermissionDenied);
}

/// Regression, `EnumerateVersions`. The manifest layer synthesizes a single version from
/// the manifest entry, so this is the second surface a warm manifest cache could answer without
/// authorization. Same shape as test 4.
#[rstest]
#[test_log::test(tokio::test)]
async fn overlay_enumerate_versions_is_authorized_on_a_warm_manifest_cache(overlay_fixture: &OverlayFixture) {
    let service = &overlay_fixture.service;
    let context = &overlay_fixture.permission_context;
    let client = service.fileobject_v1beta_client(Interface::Grpc).await;
    let versioning_client = service.versioning_v1beta_client(Interface::Grpc).await;
    let tag = get_random_alphanumeric_string(8).to_lowercase();

    let backing_address = service
        .az_url_builder()
        .path(format!("data-{tag}/scene.usda"))
        .to_string();
    let workspace = format!("overlay-{tag}.wkspc");
    let manifest_address = service
        .az_url_builder()
        .path(format!("{workspace}/.manifest"))
        .to_string();
    let overlay_address = service
        .az_url_builder()
        .path(format!("{workspace}/scene.usda"))
        .to_string();

    {
        let setup = context.new_rule_set();
        setup.allow_object_write(&backing_address);
        setup.allow_object_write(&manifest_address);
        let backing_identity = write_object(
            &client,
            &backing_address,
            Bytes::from_static(b"warm-cache-versions-bytes"),
        )
        .await;
        write_object(
            &client,
            &manifest_address,
            manifest_json("scene.usda", &backing_identity),
        )
        .await;
    }

    // Authorized enumerate, which also warms the manifest cache.
    {
        let rules = context.new_rule_set();
        // The resolver branch reads the `.manifest` body while the cache is cold.
        rules.allow_object_read(&manifest_address);
        // The client's own address is authorized above the manifest layer.
        rules.allow_object_read(&overlay_address);

        let response = versioning_client
            .enumerate_versions_collect(EnumerateVersionsRequest {
                resource_address: overlay_address.clone(),
            })
            .await
            .expect("authorized overlay enumerate_versions should serve from the manifest");
        assert_eq!(response.items.len(), 1);
    }

    // Same entry, same warm cache, no grants.
    let status = versioning_client
        .enumerate_versions_collect(EnumerateVersionsRequest {
            resource_address: overlay_address,
        })
        .await
        .expect_err("overlay enumerate_versions must be forbidden once the grants are gone");
    assert_eq!(status.code(), Code::PermissionDenied);
}

/// Regression, listing surfaces. A manifest-phase continuation handle makes the manifest
/// layer emit its overlay page directly, skipping the backend phase, so with the manifest layer
/// above `permissions` a client could forge one and read a workspace listing it was never
/// authorized for. Pagination is a REST-only wire concept, hence the REST clients here.
#[rstest]
#[test_log::test(tokio::test)]
async fn forged_manifest_continuation_handle_does_not_bypass_authorization(overlay_fixture: &OverlayFixture) {
    let service = &overlay_fixture.service;
    let context = &overlay_fixture.permission_context;
    let client = service.fileobject_v1beta_client(Interface::Grpc).await;
    let filefolder_client = service.filefolder_v1beta_rest_client();
    let fileobject_client = service.fileobject_v1beta_rest_client();
    let tag = get_random_alphanumeric_string(8).to_lowercase();

    let backing_address = service
        .az_url_builder()
        .path(format!("data-{tag}/scene.usda"))
        .to_string();
    let workspace = format!("overlay-{tag}.wkspc");
    let manifest_address = service
        .az_url_builder()
        .path(format!("{workspace}/.manifest"))
        .to_string();
    let overlay_address = service
        .az_url_builder()
        .path(format!("{workspace}/scene.usda"))
        .to_string();
    let workspace_folder = service.az_url_builder().path(format!("{workspace}/")).to_string();

    {
        let setup = context.new_rule_set();
        setup.allow_object_write(&backing_address);
        setup.allow_object_write(&manifest_address);
        let backing_identity =
            write_object(&client, &backing_address, Bytes::from_static(b"forged-handle-bytes")).await;
        write_object(
            &client,
            &manifest_address,
            manifest_json("scene.usda", &backing_identity),
        )
        .await;
    }

    let list_request = ListRequest {
        folder: Some(FolderAddress {
            uri: workspace_folder.clone(),
        }),
    };
    let list_stat_request = ListStatRequest {
        folder: Some(FolderAddress {
            uri: workspace_folder.clone(),
        }),
    };
    let enumerate_request = EnumerateRequest {
        resource_address: workspace_folder.clone(),
    };
    let handle = manifest_phase_continuation_handle();

    // Authorized run: proves the handle is well formed and the manifest layer honours it, so the
    // denials below cannot be explained by a rejected handle. Also warms the manifest cache.
    {
        // The folder grant is inherited by the `.manifest` read the resolver branch does while the
        // cache is cold. `Enumerate` needs the backing object granted separately: it reports a
        // manifest entry by the address its identity parses to, which lies outside the workspace,
        // and per-item filtering is deny-on-missing.
        let rules = context.new_rule_set();
        rules.allow_folder_read(&workspace_folder);
        rules.allow_object_read(&backing_address);
        let expected_overlay = std::slice::from_ref(&overlay_address);

        let (list, _) = filefolder_client
            .list_once(&list_request, Some(&handle), None)
            .await
            .expect("authorized list should serve the manifest page");
        assert_eq!(list.sub_resource_addresses, expected_overlay);

        let (list_stat, _) = filefolder_client
            .list_stat_once(&list_stat_request, Some(&handle), None)
            .await
            .expect("authorized list_stat should serve the manifest page");
        let entries: Vec<_> = list_stat
            .entries
            .into_iter()
            .map(|entry| entry.resource_address)
            .collect();
        assert_eq!(entries, expected_overlay);

        let (enumerate, _) = fileobject_client
            .enumerate_once(enumerate_request.clone(), Some(&handle))
            .await
            .expect("authorized enumerate should serve the manifest page");
        let addresses: Vec<_> = enumerate.items.into_iter().map(|item| item.resource_address).collect();
        assert_eq!(addresses, [backing_address]);
    }

    // Same warm cache and the same handle, with nothing granted.
    let status = filefolder_client
        .list_once(&list_request, Some(&handle), None)
        .await
        .expect_err("a manifest-phase handle must not list an unauthorized workspace");
    assert_eq!(status.code(), Code::PermissionDenied);

    let status = filefolder_client
        .list_stat_once(&list_stat_request, Some(&handle), None)
        .await
        .expect_err("a manifest-phase handle must not list_stat an unauthorized workspace");
    assert_eq!(status.code(), Code::PermissionDenied);

    let status = fileobject_client
        .enumerate_once(enumerate_request, Some(&handle))
        .await
        .expect_err("a manifest-phase handle must not enumerate an unauthorized workspace");
    assert_eq!(status.code(), Code::PermissionDenied);
}

/// response rewriting. The manifest layer now sits below the remapper, so an overlay
/// response passes through the remapper's reverse rewrite on its way out for the first time. A
/// manifest stores an already client-facing identity, so that rewrite must be a no-op: the identity
/// the client gets back is byte-for-byte the one the `.manifest` declared.
#[rstest]
#[test_log::test(tokio::test)]
async fn overlay_identity_is_not_rewritten_by_the_remapper(overlay_fixture: &OverlayFixture) {
    let service = &overlay_fixture.service;
    let context = &overlay_fixture.permission_context;
    let client = service.fileobject_v1beta_client(Interface::Grpc).await;
    let tag = get_random_alphanumeric_string(8).to_lowercase();

    let backing_mapped = service
        .remapper_mapped_url_builder()
        .path(format!("data-{tag}/scene.usda"))
        .to_string();
    let backing_backend = service
        .remapper_original_url_builder()
        .path(format!("data-{tag}/scene.usda"))
        .to_string();
    let workspace = format!("overlay-{tag}.wkspc");
    let manifest_mapped = service
        .remapper_mapped_url_builder()
        .path(format!("{workspace}/.manifest"))
        .to_string();
    let manifest_backend = service
        .remapper_original_url_builder()
        .path(format!("{workspace}/.manifest"))
        .to_string();
    let overlay_mapped = service
        .remapper_mapped_url_builder()
        .path(format!("{workspace}/scene.usda"))
        .to_string();
    let overlay_backend = service
        .remapper_original_url_builder()
        .path(format!("{workspace}/scene.usda"))
        .to_string();

    // The remapper resolves every client-facing address before `permissions` sees it, so all the
    // grants are on backend URLs. A stat is answered from the manifest entry alone, so the backing
    // object needs no read grant here.
    let rules = context.new_rule_set();
    rules.allow_object_write(&backing_backend);
    rules.allow_object_write(&manifest_backend);
    rules.allow_object_read(&manifest_backend);
    rules.allow_object_read(&overlay_backend);

    let omniverse_identity = write_object(
        &client,
        &backing_mapped,
        Bytes::from_static(b"identity-passthrough-bytes"),
    )
    .await;
    write_object(
        &client,
        &manifest_mapped,
        manifest_json("scene.usda", &omniverse_identity),
    )
    .await;

    let identity = client
        .stat(StatRequest {
            resource_address: overlay_mapped,
        })
        .await
        .expect("overlay stat should serve from the manifest")
        .resource_info
        .expect("stat response is missing the resource info")
        .unwrap_identity();
    assert_eq!(
        identity, omniverse_identity,
        "the remapper must leave an identity the manifest already stores in the mapped namespace alone"
    );
}
