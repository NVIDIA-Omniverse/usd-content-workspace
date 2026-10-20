// SPDX-FileCopyrightText: Copyright 2026 NVIDIA CORPORATION & AFFILIATES
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

use super::*;
use serde_json::json;
use std::sync::Arc;

const POLICY: &str = r#"
    permit(
        principal == Principal::"test-user",
        action == Action::"storage:read",
        resource == File::"/Projects/Astronaut/test.data"
    );
"#;

fn new_action() -> Action {
    Action {
        name: "read".to_string(),
        service: "storage".to_string(),
    }
}

fn new_principal() -> Principal {
    Principal {
        id: "test-user".to_string(),
        info: None,
    }
}

fn new_resource() -> Resource {
    Resource {
        id: "/Projects/Astronaut/test.data".to_string(),
        kind: "File".to_string(),
        data: Some(json!({
            "path": "/Projects/Astronaut/test.data"
        })),
    }
}

#[test]
fn test_evaluate_policy() {
    let action = new_action();
    let principal = new_principal();
    let resource = new_resource();

    validate_policy(
        POLICY,
        Some(&action),
        Some(&principal.id),
        None,
        Some(&resource),
    )
    .unwrap();

    let authorizer = Authorizer::new();
    let response = evaluate_policy(
        POLICY,
        &authorizer,
        &action,
        &principal,
        Some(&resource),
        None,
        "groups",
    )
    .unwrap();
    assert_eq!(response.decision(), Decision::Allow);
}

#[test]
fn test_evaluate_policy_condition() {
    let policy = r#"
        permit(
            principal,
            action == Action::"storage:read",
            resource
        ) when {
            resource is File &&
            resource.path like "/Projects/*"
        };
    "#;

    let action = new_action();
    let principal = new_principal();
    let resource = new_resource();

    validate_policy(
        policy,
        Some(&action),
        Some(&principal.id),
        None,
        Some(&resource),
    )
    .unwrap();

    let authorizer = Authorizer::new();
    let response = evaluate_policy(
        policy,
        &authorizer,
        &action,
        &principal,
        Some(&resource),
        None,
        "groups",
    )
    .unwrap();
    assert_eq!(response.decision(), Decision::Allow);
}

#[test]
fn test_evaluate_policy_resource_condition_with_system_attribute() {
    let policy = r#"
        permit(
            principal,
            action == Action::"storage:read",
            resource
        ) when {
            resource.id like "/Projects/*"
        };
    "#;

    let action = new_action();
    let principal = new_principal();
    let resource = new_resource();

    validate_policy(
        policy,
        Some(&action),
        Some(&principal.id),
        None,
        Some(&resource),
    )
    .unwrap();

    let authorizer = Authorizer::new();
    let response = evaluate_policy(
        policy,
        &authorizer,
        &action,
        &principal,
        Some(&resource),
        None,
        "groups",
    )
    .unwrap();
    assert_eq!(response.decision(), Decision::Allow);
}

#[test]
fn test_evaluate_policy_principal_condition_with_system_attribute() {
    let policy = r#"
        permit(
            principal,
            action == Action::"storage:read",
            resource
        ) when {
            principal.sub like "test*"
        };
    "#;

    let action = new_action();
    let principal = new_principal();
    let resource = new_resource();

    validate_policy(
        policy,
        Some(&action),
        Some(&principal.id),
        None,
        Some(&resource),
    )
    .unwrap();

    let authorizer = Authorizer::new();
    let response = evaluate_policy(
        policy,
        &authorizer,
        &action,
        &principal,
        Some(&resource),
        None,
        "groups",
    )
    .unwrap();
    assert_eq!(response.decision(), Decision::Allow);
}

#[test]
fn test_evaluate_policy_forbid_over_permit() {
    let policy = r#"
        permit(
            principal,
            action == Action::"storage:read",
            resource
        ) when {
            resource.path like "/Projects/*"
        };
        forbid(
            principal,
            action == Action::"storage:read",
            resource == File::"/Projects/Astronaut/test.data"
        );
    "#;

    let action = new_action();
    let principal = new_principal();
    let resource = new_resource();

    validate_policy(
        policy,
        Some(&action),
        Some(&principal.id),
        None,
        Some(&resource),
    )
    .unwrap();

    let authorizer = Authorizer::new();
    let response = evaluate_policy(
        policy,
        &authorizer,
        &action,
        &principal,
        Some(&resource),
        None,
        "groups",
    )
    .unwrap();
    assert_eq!(response.decision(), Decision::Deny);
}

#[test]
fn test_evaluate_policy_principal_array_attrs() {
    let policy = r#"
        permit(
            principal,
            action == Action::"storage:read",
            resource
        ) when {
            principal.groups.contains("admin")
        };
    "#;

    let action = new_action();
    let mut principal = new_principal();
    principal.info = Some(json!({
        "groups": ["test", "admin"]
    }));
    let resource = new_resource();

    validate_policy(
        policy,
        Some(&action),
        Some(&principal.id),
        None,
        Some(&resource),
    )
    .unwrap();

    let authorizer = Authorizer::new();
    let response = evaluate_policy(
        policy,
        &authorizer,
        &action,
        &principal,
        Some(&resource),
        None,
        "groups",
    )
    .unwrap();
    assert_eq!(response.decision(), Decision::Allow);
}

#[test]
fn test_evaluate_policy_principal_record_attrs() {
    let policy = r#"
        permit(
            principal,
            action == Action::"storage:read",
            resource
        ) when {
            principal.department.name == "qa"
        };
    "#;

    let action = new_action();
    let mut principal = new_principal();
    principal.info = Some(json!({
        "department": {
            "name": "qa",
        }
    }));
    let resource = new_resource();

    validate_policy(
        policy,
        Some(&action),
        Some(&principal.id),
        None,
        Some(&resource),
    )
    .unwrap();

    let authorizer = Authorizer::new();
    let response = evaluate_policy(
        policy,
        &authorizer,
        &action,
        &principal,
        Some(&resource),
        None,
        "groups",
    )
    .unwrap();
    assert_eq!(response.decision(), Decision::Allow);
}

#[test]
fn test_validate_policy_principal() {
    let action = new_action();
    let mut principal = new_principal();
    principal.id = "invalid-user".to_string();

    let resource = new_resource();

    let result = validate_policy(
        POLICY,
        Some(&action),
        Some(&principal.id),
        None,
        Some(&resource),
    );
    match result {
        Err(PolicyErr {
            kind: PolicyErrKind::InvalidPrincipal,
            ..
        }) => {}
        result => panic!("{:?}", result),
    }
}

#[test]
fn test_validate_empty_principal() {
    let action = new_action();
    let mut principal = new_principal();
    principal.id = String::new();

    let resource = new_resource();

    let result = validate_policy(
        POLICY,
        Some(&action),
        Some(&principal.id),
        None,
        Some(&resource),
    );
    match result {
        Err(PolicyErr {
            kind: PolicyErrKind::InvalidPrincipal,
            ..
        }) => {}
        result => panic!("{:?}", result),
    }
}

#[test]
fn test_validate_empty_policy() {
    let principal = new_principal();
    let resource = new_resource();
    let action = new_action();

    let result = validate_policy(
        "",
        Some(&action),
        Some(&principal.id),
        None,
        Some(&resource),
    );
    match result {
        Err(PolicyErr {
            kind: PolicyErrKind::InvalidPolicy,
            msg,
        }) => {
            assert_eq!(msg, "`policy` cannot be empty.".to_string());
        }
        result => panic!("{:?}", result),
    }
}

#[test]
fn test_validate_policy_action() {
    let mut action = new_action();
    action.name = "write".to_string();

    let principal = new_principal();
    let resource = new_resource();

    let result = validate_policy(
        POLICY,
        Some(&action),
        Some(&principal.id),
        None,
        Some(&resource),
    );
    match result {
        Err(PolicyErr {
            kind: PolicyErrKind::InvalidAction,
            ..
        }) => {}
        result => panic!("{:?}", result),
    }
}

#[test]
fn test_validate_empty_action_name() {
    let mut action = new_action();
    action.name = String::new();

    let principal = new_principal();
    let resource = new_resource();

    let result = validate_policy(
        POLICY,
        Some(&action),
        Some(&principal.id),
        None,
        Some(&resource),
    );
    match result {
        Err(PolicyErr {
            kind: PolicyErrKind::InvalidAction,
            msg,
        }) => {
            assert_eq!(msg, "`action.name` cannot be empty.".to_string());
        }
        result => panic!("{:?}", result),
    }
}

#[test]
fn test_validate_empty_action_service() {
    let mut action = new_action();
    action.service = String::new();

    let principal = new_principal();
    let resource = new_resource();

    let result = validate_policy(
        POLICY,
        Some(&action),
        Some(&principal.id),
        None,
        Some(&resource),
    );
    match result {
        Err(PolicyErr {
            kind: PolicyErrKind::InvalidAction,
            msg,
        }) => {
            assert_eq!(msg, "`action.service` cannot be empty.".to_string());
        }
        result => panic!("{:?}", result),
    }
}

#[test]
fn test_validate_policy_action_range() {
    let mut action = new_action();
    action.name = "admin".to_string();

    let principal = new_principal();
    let resource = new_resource();

    let result = validate_policy(
        POLICY,
        Some(&action),
        Some(&principal.id),
        None,
        Some(&resource),
    );
    match result {
        Err(PolicyErr {
            kind: PolicyErrKind::InvalidAction,
            ..
        }) => {}
        result => panic!("{:?}", result),
    }
}

#[test]
fn test_validate_policy_resource() {
    let action = new_action();
    let principal = new_principal();
    let mut resource = new_resource();
    resource.id = "/Projects/Astronaut/Astronaut.usd".to_string();

    let result = validate_policy(
        POLICY,
        Some(&action),
        Some(&principal.id),
        None,
        Some(&resource),
    );
    match result {
        Err(PolicyErr {
            kind: PolicyErrKind::InvalidResource,
            ..
        }) => {}
        result => panic!("{:?}", result),
    }
}

#[test]
fn test_validate_empty_resource_id() {
    let action = new_action();
    let principal = new_principal();
    let mut resource = new_resource();
    resource.id = String::new();

    let result = validate_policy(
        POLICY,
        Some(&action),
        Some(&principal.id),
        None,
        Some(&resource),
    );
    match result {
        Err(PolicyErr {
            kind: PolicyErrKind::InvalidResource,
            msg,
        }) => {
            assert_eq!(msg, "`resource.id` cannot be empty.".to_string());
        }
        result => panic!("{:?}", result),
    }
}

#[test]
fn test_validate_empty_resource_type() {
    let action = new_action();
    let principal = new_principal();
    let mut resource = new_resource();
    resource.kind = String::new();

    let result = validate_policy(
        POLICY,
        Some(&action),
        Some(&principal.id),
        None,
        Some(&resource),
    );
    match result {
        Err(PolicyErr {
            kind: PolicyErrKind::InvalidResource,
            msg,
        }) => {
            assert_eq!(msg, "`resource.type` cannot be empty.".to_string());
        }
        result => panic!("{:?}", result),
    }
}

#[test]
fn test_validate_policy_with_url_resource_encoding() {
    let policy = r#"
        permit(
            principal == Principal::"test-user",
            action == Action::"storage:read",
            resource == File::"https://example.com/user@domain/resource"
        );
    "#;

    let action = new_action();
    let principal = new_principal();
    let mut resource = new_resource();
    resource.id = "https://example.com/user%40domain/resource".to_string();

    let result = validate_policy(
        policy,
        Some(&action),
        Some(&principal.id),
        None,
        Some(&resource),
    );
    assert!(
        result.is_ok(),
        "Policy validation should succeed after URL encoding"
    );

    let expected_policy = r#"permit(principal == Principal::"test-user", action == Action::"storage:read", resource == File::"https://example.com/user%40domain/resource");"#;
    let policy_string = result.unwrap();
    assert_eq!(
        policy_string.trim(),
        expected_policy,
        "Returned policy should have encoded resource URL"
    );
}

#[test]
fn test_validate_policy_with_multiple_at_characters() {
    let policy = r#"
        permit(
            principal == Principal::"test-user",
            action == Action::"storage:read",
            resource == File::"https://example.com/user@domain.test/resource@file"
        );
    "#;

    let action = new_action();
    let principal = new_principal();
    let mut resource = new_resource();
    resource.id = "https://example.com/user%40domain.test/resource%40file".to_string();

    let result = validate_policy(
        policy,
        Some(&action),
        Some(&principal.id),
        None,
        Some(&resource),
    );
    assert!(
        result.is_ok(),
        "Policy validation should succeed with multiple @ characters encoded"
    );

    let expected_policy = r#"permit(principal == Principal::"test-user", action == Action::"storage:read", resource == File::"https://example.com/user%40domain.test/resource%40file");"#;
    let policy_string = result.unwrap();
    assert_eq!(
        policy_string.trim(),
        expected_policy,
        "Returned policy should have encoded resource URL with multiple @ characters"
    );
}

#[test]
fn test_validate_policy_with_spaces_in_url() {
    let policy = r#"
        permit(
            principal == Principal::"test-user",
            action == Action::"storage:read",
            resource == File::"https://example.com/path with spaces/resource"
        );
    "#;

    let action = new_action();
    let principal = new_principal();
    let mut resource = new_resource();
    resource.id = "https://example.com/path%20with%20spaces/resource".to_string();

    let result = validate_policy(
        policy,
        Some(&action),
        Some(&principal.id),
        None,
        Some(&resource),
    );
    assert!(
        result.is_ok(),
        "Policy validation should succeed with spaces encoded"
    );

    let expected_policy = r#"permit(principal == Principal::"test-user", action == Action::"storage:read", resource == File::"https://example.com/path%20with%20spaces/resource");"#;
    let policy_string = result.unwrap();
    assert_eq!(
        policy_string.trim(),
        expected_policy,
        "Returned policy should have encoded resource URL with spaces"
    );
}

#[test]
fn test_validate_policy_with_non_url_resource() {
    let policy = r#"
        permit(
            principal == Principal::"test-user",
            action == Action::"storage:read",
            resource == File::"not-a-url-resource@test"
        );
    "#;

    let action = new_action();
    let principal = new_principal();
    let mut resource = new_resource();
    resource.id = "not-a-url-resource@test".to_string();

    let result = validate_policy(
        policy,
        Some(&action),
        Some(&principal.id),
        None,
        Some(&resource),
    );
    assert!(
        result.is_ok(),
        "Policy validation should succeed for non-URL resources"
    );

    let expected_policy = r#"permit(principal == Principal::"test-user", action == Action::"storage:read", resource == File::"not-a-url-resource@test");"#;
    let policy_string = result.unwrap();
    assert_eq!(
        policy_string.trim(),
        expected_policy,
        "Returned policy should have unchanged non-URL resource"
    );
}

#[test]
fn test_validate_policy_with_already_encoded_url() {
    let policy = r#"
        permit(
            principal == Principal::"test-user",
            action == Action::"storage:read",
            resource == File::"https://example.com/user%40domain/resource%20file"
        );
    "#;

    let action = new_action();
    let principal = new_principal();
    let mut resource = new_resource();
    resource.id = "https://example.com/user%40domain/resource%20file".to_string();

    let result = validate_policy(
        policy,
        Some(&action),
        Some(&principal.id),
        None,
        Some(&resource),
    );
    assert!(
        result.is_ok(),
        "Policy validation should succeed with already encoded URLs"
    );

    let expected_policy = r#"permit(principal == Principal::"test-user", action == Action::"storage:read", resource == File::"https://example.com/user%40domain/resource%20file");"#;
    let policy_string = result.unwrap();
    assert_eq!(
        policy_string.trim(),
        expected_policy,
        "Returned policy should have the already encoded URL unchanged"
    );
}

#[test]
fn test_validate_policy_with_complex_url_encoding() {
    let policy = r#"
        permit(
            principal == Principal::"test-user",
            action == Action::"storage:read",
            resource == File::"https://api.example.com/users@domain/profile"
        );
    "#;

    let action = new_action();
    let principal = new_principal();
    let mut resource = new_resource();
    resource.id = "https://api.example.com/users%40domain/profile".to_string();

    let result = validate_policy(
        policy,
        Some(&action),
        Some(&principal.id),
        None,
        Some(&resource),
    );
    assert!(
        result.is_ok(),
        "Policy validation should succeed with complex URL encoding"
    );

    let expected_policy = r#"permit(principal == Principal::"test-user", action == Action::"storage:read", resource == File::"https://api.example.com/users%40domain/profile");"#;
    let policy_string = result.unwrap();
    assert_eq!(
        policy_string.trim(),
        expected_policy,
        "Returned policy should have encoded resource URL"
    );
}

#[test]
fn test_evaluate_policy_with_encoded_url_resource() {
    let policy = r#"
        permit(
            principal == Principal::"test-user",
            action == Action::"storage:read",
            resource == File::"https://example.com/user@domain/test.data"
        );
    "#;

    let action = new_action();
    let principal = new_principal();
    let mut resource = new_resource();
    resource.id = "https://example.com/user%40domain/test.data".to_string();
    resource.data = Some(json!({
        "path": "https://example.com/user%40domain/test.data"
    }));

    let policy = validate_policy(
        policy,
        Some(&action),
        Some(&principal.id),
        None,
        Some(&resource),
    )
    .unwrap();

    let authorizer = Authorizer::new();
    let response = evaluate_policy(
        &policy,
        &authorizer,
        &action,
        &principal,
        Some(&resource),
        None,
        "groups",
    )
    .unwrap();
    assert_eq!(response.decision(), Decision::Allow);
}

#[test]
fn test_validate_policy_encoding_preserves_userinfo_and_query() {
    let policy = r#"
        permit(
            principal == Principal::"test-user",
            action == Action::"storage:read",
            resource == File::"https://user:password@example.com/path?param=value@test"
        );
    "#;

    let action = new_action();
    let principal = new_principal();
    let mut resource = new_resource();
    resource.id = "https://user:password@example.com/path?param=value@test".to_string();

    let result = validate_policy(
        policy,
        Some(&action),
        Some(&principal.id),
        None,
        Some(&resource),
    );
    assert!(
        result.is_ok(),
        "Policy validation should succeed preserving userinfo and query"
    );

    let expected_policy = r#"permit(principal == Principal::"test-user", action == Action::"storage:read", resource == File::"https://user:password@example.com/path?param=value@test");"#;

    let policy_string = result.unwrap();
    assert_eq!(
        policy_string.trim(),
        expected_policy,
        "Returned policy should preserve userinfo and query parameters without encoding"
    );
}

#[test]
fn test_validate_policy_with_special_characters_encoding() {
    let policy = r#"
        permit(
            principal == Principal::"test-user",
            action == Action::"storage:read",
            resource == File::"https://example.com/path/with spaces & symbols@test!$%"
        );
    "#;

    let action = new_action();
    let principal = new_principal();
    let mut resource = new_resource();
    resource.id =
        "https://example.com/path/with%20spaces%20%26%20symbols%40test%21%24%25".to_string();

    let result = validate_policy(
        policy,
        Some(&action),
        Some(&principal.id),
        None,
        Some(&resource),
    );
    assert!(
        result.is_ok(),
        "Policy validation should succeed with special characters encoded"
    );

    let expected_policy = r#"permit(principal == Principal::"test-user", action == Action::"storage:read", resource == File::"https://example.com/path/with%20spaces%20%26%20symbols%40test%21%24%25");"#;
    let policy_string = result.unwrap();
    assert_eq!(
        policy_string.trim(),
        expected_policy,
        "Returned policy should have encoded special characters"
    );
}

#[test]
fn test_infer_policy_scopes_eq_constraints() {
    let policy = r#"
        permit(
            principal == Principal::"test-user",
            action == Action::"storage:read",
            resource == File::"/Projects/Astronaut/test.data"
        );
    "#;

    let scopes = infer_policy_scopes(policy).unwrap();
    assert_eq!(
        scopes.principal,
        Some(Principal {
            id: "test-user".to_string(),
            info: None
        })
    );
    assert_eq!(
        scopes.action,
        Some(Action {
            service: "storage".to_string(),
            name: "read".to_string()
        })
    );
    assert_eq!(
        scopes.resource,
        Some(Resource {
            id: "/Projects/Astronaut/test.data".to_string(),
            kind: "File".to_string(),
            data: None
        })
    );
}

#[test]
fn test_infer_policy_scopes_group_constraint() {
    let policy = r#"
        permit(
            principal in Group::"admins",
            action in [Action::"storage:read", Action::"storage:write"],
            resource is File
        );
    "#;

    let scopes = infer_policy_scopes(policy).unwrap();
    assert_eq!(scopes.principal, None);
    assert_eq!(
        scopes.group,
        Some(Group {
            id: "admins".to_string()
        })
    );
    assert_eq!(scopes.action, None);
    assert_eq!(scopes.resource, None);
}

#[test]
fn test_infer_policy_scopes_non_representable_constraints() {
    let policy = r#"
        permit(
            principal in Team::"admins",
            action in [Action::"storage:read", Action::"storage:write"],
            resource is File
        );
    "#;

    let scopes = infer_policy_scopes(policy).unwrap();
    assert_eq!(scopes.principal, None);
    assert_eq!(scopes.group, None);
    assert_eq!(scopes.action, None);
    assert_eq!(scopes.resource, None);
}

#[test]
fn test_evaluate_group_scope_allows_member_and_denies_non_member() {
    let policy = r#"permit(principal in Group::"admins", action, resource);"#;
    let action = new_action();
    let authorizer = Authorizer::new();

    let member = Principal {
        id: "test-user".to_string(),
        info: Some(json!({ "groups": ["admins", "viewers"] })),
    };
    let response =
        evaluate_policy(policy, &authorizer, &action, &member, None, None, "groups").unwrap();
    assert_eq!(response.decision(), Decision::Allow);

    let non_member = Principal {
        id: "test-user".to_string(),
        info: Some(json!({ "groups": ["viewers"] })),
    };
    let response = evaluate_policy(
        policy,
        &authorizer,
        &action,
        &non_member,
        None,
        None,
        "groups",
    )
    .unwrap();
    assert_eq!(response.decision(), Decision::Deny);
}

#[test]
fn test_evaluate_group_scope_matches_contains_condition() {
    let group_scope = r#"permit(principal in Group::"admins", action, resource);"#;
    let condition =
        r#"permit(principal, action, resource) when { principal.groups.contains("admins") };"#;
    let action = new_action();
    let authorizer = Authorizer::new();

    for groups in [json!(["admins"]), json!(["viewers"]), json!([])] {
        let principal = Principal {
            id: "test-user".to_string(),
            info: Some(json!({ "groups": groups })),
        };
        let scoped = evaluate_policy(
            group_scope,
            &authorizer,
            &action,
            &principal,
            None,
            None,
            "groups",
        )
        .unwrap();
        let conditioned = evaluate_policy(
            condition,
            &authorizer,
            &action,
            &principal,
            None,
            None,
            "groups",
        )
        .unwrap();
        assert_eq!(scoped.decision(), conditioned.decision());
    }
}

#[test]
fn test_evaluate_group_scope_honors_configured_claim() {
    let policy = r#"permit(principal in Group::"admins", action, resource);"#;
    let action = new_action();
    let authorizer = Authorizer::new();

    let principal = Principal {
        id: "test-user".to_string(),
        info: Some(json!({ "roles": ["admins"] })),
    };

    let denied = evaluate_policy(
        policy,
        &authorizer,
        &action,
        &principal,
        None,
        None,
        "groups",
    )
    .unwrap();
    assert_eq!(denied.decision(), Decision::Deny);

    let allowed = evaluate_policy(
        policy,
        &authorizer,
        &action,
        &principal,
        None,
        None,
        "roles",
    )
    .unwrap();
    assert_eq!(allowed.decision(), Decision::Allow);
}

#[test]
fn test_infer_policy_scopes_multiple_policies_error() {
    let policy = r#"
        permit(
            principal == Principal::"test-user",
            action == Action::"storage:read",
            resource
        );
        permit(
            principal == Principal::"other-user",
            action == Action::"storage:read",
            resource
        );
    "#;

    let result = infer_policy_scopes(policy);
    match result {
        Err(PolicyErr {
            kind: PolicyErrKind::InvalidPolicy,
            msg,
        }) => assert_eq!(
            msg,
            "Expected exactly one Cedar policy, but multiple policies were provided."
        ),
        result => panic!("{:?}", result),
    }
}

#[test]
fn test_infer_policy_scopes_encodes_resource_id() {
    let policy = r#"
        permit(
            principal == Principal::"test-user",
            action == Action::"storage:read",
            resource == File::"https://example.com/user@domain/test.data"
        );
    "#;

    let scopes = infer_policy_scopes(policy).unwrap();
    assert_eq!(
        scopes.resource,
        Some(Resource {
            id: "https://example.com/user%40domain/test.data".to_string(),
            kind: "File".to_string(),
            data: None
        })
    );
}

#[test]
fn test_infer_policy_scopes_empty_policy() {
    let result = infer_policy_scopes("");
    match result {
        Err(PolicyErr {
            kind: PolicyErrKind::InvalidPolicy,
            msg,
        }) => assert_eq!(msg, "`policy` cannot be empty."),
        result => panic!("{:?}", result),
    }
}

#[test]
fn test_process_policy_matches_legacy_helpers() {
    // The batch handler now calls `process_policy` instead of the trio of
    // `infer_policy_scopes` + `validate_policy` + `canonicalize_policy`. Lock
    // in that the single-parse variant produces the same text, scopes, and
    // canonical hash as the legacy path so we don't silently drift.
    let policy = r#"
        permit(
            principal == Principal::"test-user",
            action == Action::"storage:read",
            resource == File::"https://example.com/path with space/data.usd"
        );
    "#;

    let scopes_legacy = infer_policy_scopes(policy).unwrap();
    let text_legacy = validate_policy(
        policy,
        scopes_legacy.action.as_ref(),
        scopes_legacy.principal.as_ref().map(|p| p.id.as_str()),
        scopes_legacy.group.as_ref().map(|g| g.id.as_str()),
        scopes_legacy.resource.as_ref(),
    )
    .unwrap();
    let canonical_legacy = canonicalize_policy(&text_legacy).unwrap();
    let hash_legacy = md5_hex(&canonical_legacy);

    let processed = process_policy(policy, true).unwrap();
    assert_eq!(processed.text, text_legacy);
    assert_eq!(processed.scopes.principal, scopes_legacy.principal);
    assert_eq!(processed.scopes.action, scopes_legacy.action);
    assert_eq!(processed.scopes.resource, scopes_legacy.resource);
    assert_eq!(processed.canonical_hash, hash_legacy);
    assert_eq!(processed.canonical_hash.len(), 32);
}

#[test]
fn test_process_policy_without_validation_still_returns_hash() {
    let policy = r#"permit(principal, action, resource);"#;
    let processed = process_policy(policy, false).unwrap();
    assert!(processed.scopes.principal.is_none());
    assert_eq!(processed.canonical_hash.len(), 32);
    // Identical input must produce identical hash (deterministic dedup key).
    let again = process_policy(policy, false).unwrap();
    assert_eq!(processed.canonical_hash, again.canonical_hash);
}

#[test]
fn test_md5_hex_known_value() {
    // RFC 1321 sample.
    assert_eq!(md5_hex(""), "d41d8cd98f00b204e9800998ecf8427e");
    assert_eq!(md5_hex("abc"), "900150983cd24fb0d6963f7d28e17f72");
}

#[test]
fn test_canonicalize_policy_normalizes_whitespace_and_comments() {
    let base = "permit(principal, action, resource);";
    let with_comments =
        "// header comment\npermit(principal, action, resource);\n// trailing comment\n";
    let with_whitespace = "permit(\n    principal,\n\taction,\n    resource\n);\n";
    let with_extra_spaces = "  permit(  principal  ,  action  ,  resource  )  ;  ";

    let canonical = canonicalize_policy(base).unwrap();
    assert_eq!(canonicalize_policy(with_comments).unwrap(), canonical);
    assert_eq!(canonicalize_policy(with_whitespace).unwrap(), canonical);
    assert_eq!(canonicalize_policy(with_extra_spaces).unwrap(), canonical);
}

#[test]
fn test_canonicalize_policy_preserves_string_literal_whitespace() {
    let a = r#"permit(principal, action, resource == File::"/path with spaces");"#;
    let b = r#"permit(
        principal,
        action,
        resource == File::"/path with spaces"
    );"#;
    let c = r#"permit(principal, action, resource == File::"/path without spaces");"#;

    let ca = canonicalize_policy(a).unwrap();
    let cb = canonicalize_policy(b).unwrap();
    let cc = canonicalize_policy(c).unwrap();
    assert_eq!(ca, cb);
    assert_ne!(ca, cc);
}

#[test]
fn test_infer_policy_scopes_in_hierarchy_url() {
    let policy = r#"
        permit(
            principal == Principal::"test-user",
            action == Action::"storage:read",
            resource in Folder::"https://main/docs/"
        );
    "#;

    let scopes = infer_policy_scopes(policy).unwrap();
    assert_eq!(scopes.resource, None);
    assert_eq!(
        scopes.resource_prefix,
        Some(Resource {
            id: "https://main/docs/".to_string(),
            kind: "Folder".to_string(),
            data: None
        })
    );
}

#[test]
fn test_infer_policy_scopes_in_hierarchy_normalizes_trailing_slash() {
    let policy = r#"
        permit(
            principal,
            action,
            resource in Folder::"https://main/docs"
        );
    "#;

    let scopes = infer_policy_scopes(policy).unwrap();
    assert_eq!(
        scopes.resource_prefix,
        Some(Resource {
            id: "https://main/docs/".to_string(),
            kind: "Folder".to_string(),
            data: None
        })
    );
}

#[test]
fn test_infer_policy_scopes_in_hierarchy_rejects_non_url() {
    let policy = r#"
        permit(
            principal,
            action,
            resource in Folder::"not-a-url"
        );
    "#;

    match infer_policy_scopes(policy) {
        Err(PolicyErr {
            kind: PolicyErrKind::InvalidResource,
            ..
        }) => {}
        result => panic!("expected InvalidResource, got {:?}", result),
    }
}

#[test]
fn test_resource_hierarchy_ancestors_includes_intermediate_folders() {
    let ancestors = resource_hierarchy_ancestors("Folder", "https://main/docs/reports/q1.pdf");
    let ids: Vec<&str> = ancestors.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(
        ids,
        vec![
            "https://main/",
            "https://main/docs/",
            "https://main/docs/reports/"
        ]
    );
    assert!(ancestors.iter().all(|a| a.kind == "Folder"));
}

#[test]
fn test_resource_hierarchy_ancestors_folder_form_includes_self() {
    let ancestors = resource_hierarchy_ancestors("Folder", "https://main/docs/");
    let ids: Vec<&str> = ancestors.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(ids, vec!["https://main/", "https://main/docs/"]);
}

#[test]
fn test_resource_hierarchy_ancestors_rejects_non_url() {
    assert!(resource_hierarchy_ancestors("Folder", "not-a-url").is_empty());
}

#[test]
fn test_evaluate_in_scope_matches_descendant_and_rejects_sibling() {
    let policy = r#"
        permit(
            principal == Principal::"test-user",
            action == Action::"storage:read",
            resource in Folder::"https://main/docs/"
        );
    "#;
    let action = Action {
        name: "read".to_string(),
        service: "storage".to_string(),
    };
    let authorizer = Authorizer::new();
    let principal = Principal {
        id: "test-user".to_string(),
        info: None,
    };
    let prepared = prepare_principal(&principal, "groups").unwrap();

    let descendant = Resource {
        id: "https://main/docs/reports/q1.pdf".to_string(),
        kind: "object".to_string(),
        data: None,
    };
    let ancestors = resource_hierarchy_ancestors("Folder", &descendant.id);
    let parents = build_resource_parents(&ancestors).unwrap();
    let response = evaluate_policy_with_principal(
        policy,
        &authorizer,
        &prepared,
        &action,
        Some(&descendant),
        &parents,
        None,
    )
    .unwrap();
    assert_eq!(response.decision(), Decision::Allow);

    let sibling = Resource {
        id: "https://main/other/q1.pdf".to_string(),
        kind: "object".to_string(),
        data: None,
    };
    let sibling_ancestors = resource_hierarchy_ancestors("Folder", &sibling.id);
    let sibling_parents = build_resource_parents(&sibling_ancestors).unwrap();
    let response = evaluate_policy_with_principal(
        policy,
        &authorizer,
        &prepared,
        &action,
        Some(&sibling),
        &sibling_parents,
        None,
    )
    .unwrap();
    assert_eq!(response.decision(), Decision::Deny);
}

#[test]
fn test_evaluate_in_scope_matches_exact_folder_node() {
    let policy = r#"
        permit(
            principal == Principal::"test-user",
            action == Action::"storage:read",
            resource in Folder::"https://main/docs/"
        );
    "#;
    let action = Action {
        name: "read".to_string(),
        service: "storage".to_string(),
    };
    let authorizer = Authorizer::new();
    let principal = Principal {
        id: "test-user".to_string(),
        info: None,
    };
    let prepared = prepare_principal(&principal, "groups").unwrap();

    let exact = Resource {
        id: "https://main/docs/".to_string(),
        kind: "object".to_string(),
        data: None,
    };
    let ancestors = resource_hierarchy_ancestors("Folder", &exact.id);
    let parents = build_resource_parents(&ancestors).unwrap();
    let response = evaluate_policy_with_principal(
        policy,
        &authorizer,
        &prepared,
        &action,
        Some(&exact),
        &parents,
        None,
    )
    .unwrap();
    assert_eq!(response.decision(), Decision::Allow);
}

#[test]
fn test_evaluate_in_scope_same_type_exact_node_has_no_self_cycle() {
    let policy = r#"
        permit(
            principal,
            action,
            resource in Folder::"omniverse://main/abc/"
        );
    "#;
    let action = Action {
        name: "read".to_string(),
        service: "storage".to_string(),
    };
    let authorizer = Authorizer::new();
    let principal = Principal {
        id: "test-user".to_string(),
        info: None,
    };
    let prepared = prepare_principal(&principal, "groups").unwrap();

    let exact = Resource {
        id: "omniverse://main/abc/".to_string(),
        kind: "Folder".to_string(),
        data: None,
    };
    let ancestors = resource_hierarchy_ancestors("Folder", &exact.id);
    assert!(
        ancestors
            .iter()
            .any(|a| a.id == exact.id && a.kind == exact.kind),
        "the exact folder node is expected among the ancestors"
    );

    let parents = build_resource_parents(&ancestors).unwrap();
    let response = evaluate_policy_with_principal(
        policy,
        &authorizer,
        &prepared,
        &action,
        Some(&exact),
        &parents,
        None,
    )
    .expect("self-referential parent must not raise a transitive closure error");
    assert_eq!(response.decision(), Decision::Allow);
}

#[test]
fn test_evaluate_in_scope_dot_dot_segment_resolves_to_parent_folder() {
    let policy = r#"
        permit(
            principal == Principal::"test-user",
            action == Action::"storage:read",
            resource in Folder::"https://main/container/"
        );
    "#;
    let action = Action {
        name: "read".to_string(),
        service: "storage".to_string(),
    };
    let authorizer = Authorizer::new();
    let principal = Principal {
        id: "test-user".to_string(),
        info: None,
    };
    let prepared = prepare_principal(&principal, "groups").unwrap();

    let escaped = Resource {
        id: encode_resource_url("https://main/container/../abc/").unwrap(),
        kind: "object".to_string(),
        data: None,
    };
    assert_eq!(escaped.id, "https://main/abc/");

    let ancestors = resource_hierarchy_ancestors("Folder", &escaped.id);
    let ids: Vec<&str> = ancestors.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(ids, vec!["https://main/", "https://main/abc/"]);

    let parents = build_resource_parents(&ancestors).unwrap();
    let response = evaluate_policy_with_principal(
        policy,
        &authorizer,
        &prepared,
        &action,
        Some(&escaped),
        &parents,
        None,
    )
    .unwrap();
    assert_eq!(response.decision(), Decision::Deny);
}

#[test]
fn same_hash_and_text_reuses_arc() {
    let cache = PolicySetCache::new(100, std::time::Duration::from_secs(60));
    let hash = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let first = cache.get_or_parse(hash, POLICY).unwrap().0;
    let second = cache.get_or_parse(hash, POLICY).unwrap().0;
    assert!(Arc::ptr_eq(&first, &second));
    assert!(cache.get_or_parse(hash, POLICY).unwrap().1);
}

#[test]
fn same_hash_different_text_does_not_collide() {
    let cache = PolicySetCache::new(100, std::time::Duration::from_secs(60));
    let permit = r#"permit(principal, action, resource);"#;
    let forbid = r#"forbid(principal, action, resource);"#;
    let fingerprint = 42u64;

    let permit_key = PolicyTextKey::with_fingerprint(permit, fingerprint);
    let forbid_key = PolicyTextKey::with_fingerprint(forbid, fingerprint);
    assert_ne!(permit_key, forbid_key);
    assert_eq!(permit_key.fingerprint(), forbid_key.fingerprint());

    let permit_set = Arc::new(parse_policy_set(permit).unwrap());
    cache.insert(permit_key, permit_set.clone());

    assert!(
        cache.get(&forbid_key).is_none(),
        "identical fingerprint with different text must not return another PolicySet"
    );
    assert!(Arc::ptr_eq(
        &cache
            .get(&PolicyTextKey::with_fingerprint(permit, fingerprint))
            .unwrap(),
        &permit_set
    ));
}

#[test]
fn different_policies_evaluate_independently() {
    let cache = PolicySetCache::new(100, std::time::Duration::from_secs(60));
    let permit = r#"
        permit(
            principal == Principal::"test-user",
            action == Action::"storage:read",
            resource
        );
    "#;
    let forbid = r#"
        forbid(
            principal == Principal::"test-user",
            action == Action::"storage:read",
            resource
        );
    "#;
    let authorizer = Authorizer::new();
    let action = new_action();
    let principal = new_principal();
    let prepared = prepare_principal(&principal, "groups").unwrap();

    let permit_set = cache
        .get_or_parse("permit-hash-000000000000000000000000", permit)
        .unwrap()
        .0;
    let forbid_set = cache
        .get_or_parse("forbid-hash-000000000000000000000000", forbid)
        .unwrap()
        .0;

    let permit_response = evaluate_policy_set_with_principal(
        permit_set.as_ref(),
        &authorizer,
        &prepared,
        &action,
        None,
        &[],
        None,
    )
    .unwrap();
    let forbid_response = evaluate_policy_set_with_principal(
        forbid_set.as_ref(),
        &authorizer,
        &prepared,
        &action,
        None,
        &[],
        None,
    )
    .unwrap();

    assert_eq!(permit_response.decision(), Decision::Allow);
    assert_eq!(forbid_response.decision(), Decision::Deny);
    assert!(!Arc::ptr_eq(&permit_set, &forbid_set));
}

#[test]
fn invalid_text_not_cached() {
    let cache = PolicySetCache::new(100, std::time::Duration::from_secs(60));
    let hash = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let invalid = "not a cedar policy";

    assert!(cache.get_or_parse(hash, invalid).is_err());
    assert_eq!(cache.entry_count(), 0);
    assert!(cache.get_or_parse(hash, invalid).is_err());

    let (valid, hit) = cache.get_or_parse(hash, POLICY).unwrap();
    assert!(!hit);
    assert_eq!(valid.policies().count(), 1);
}

#[test]
fn cached_eval_matches_fresh_parse() {
    let cache = PolicySetCache::new(100, std::time::Duration::from_secs(60));
    let hash = "cccccccccccccccccccccccccccccccc";
    let authorizer = Authorizer::new();
    let action = new_action();
    let principal = new_principal();
    let resource = new_resource();
    let prepared = prepare_principal(&principal, "groups").unwrap();

    let cached = cache.get_or_parse(hash, POLICY).unwrap().0;
    let cached_response = evaluate_policy_set_with_principal(
        cached.as_ref(),
        &authorizer,
        &prepared,
        &action,
        Some(&resource),
        &[],
        None,
    )
    .unwrap();
    let fresh_response = evaluate_policy_with_principal(
        POLICY,
        &authorizer,
        &prepared,
        &action,
        Some(&resource),
        &[],
        None,
    )
    .unwrap();

    assert_eq!(cached_response.decision(), fresh_response.decision());
    assert_eq!(cached_response.decision(), Decision::Allow);
}
