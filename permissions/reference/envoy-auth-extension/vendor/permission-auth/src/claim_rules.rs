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

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fmt;

pub fn parse_claim_rules(value: &str) -> Result<ClaimRules, String> {
    serde_json::from_str(value)
        .map_err(|err| format!("invalid required claims configuration: {}", err))
}

#[derive(Clone, Debug, Default, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ClaimRules(Vec<ClaimRule>);

impl ClaimRules {
    pub fn evaluate(&self, claims: &HashMap<String, Value>) -> Result<(), ClaimRuleViolation> {
        match self
            .0
            .iter()
            .find(|rule| !rule.is_satisfied_by(claims.get(&rule.claim)))
        {
            Some(rule) => Err(ClaimRuleViolation {
                claim: rule.claim.clone(),
                operator: rule.condition.operator(),
            }),
            None => Ok(()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaimRuleViolation {
    pub claim: String,
    pub operator: ClaimOperator,
}

impl fmt::Display for ClaimRuleViolation {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "claim '{}' does not satisfy the '{}' rule",
            self.claim, self.operator
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClaimOperator {
    Equals,
    NotEquals,
    In,
    NotIn,
    Contains,
    ContainsAny,
    ContainsAll,
    Exists,
}

impl ClaimOperator {
    fn as_str(&self) -> &'static str {
        match self {
            ClaimOperator::Equals => "equals",
            ClaimOperator::NotEquals => "notEquals",
            ClaimOperator::In => "in",
            ClaimOperator::NotIn => "notIn",
            ClaimOperator::Contains => "contains",
            ClaimOperator::ContainsAny => "containsAny",
            ClaimOperator::ContainsAll => "containsAll",
            ClaimOperator::Exists => "exists",
        }
    }
}

impl fmt::Display for ClaimOperator {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawClaimRule", into = "RawClaimRule")]
struct ClaimRule {
    claim: String,
    condition: ClaimCondition,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum ClaimCondition {
    Equals(ClaimValue),
    NotEquals(ClaimValue),
    In(Vec<ClaimValue>),
    NotIn(Vec<ClaimValue>),
    Contains(ClaimValue),
    ContainsAny(Vec<ClaimValue>),
    ContainsAll(Vec<ClaimValue>),
    Exists,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
enum ClaimValue {
    Boolean(bool),
    Integer(i64),
    String(String),
}

impl ClaimRule {
    fn is_satisfied_by(&self, claim: Option<&Value>) -> bool {
        let claim = match claim {
            None | Some(Value::Null) => return false,
            Some(claim) => claim,
        };
        match &self.condition {
            ClaimCondition::Equals(expected) => expected.matches(claim),
            ClaimCondition::NotEquals(expected) => is_scalar(claim) && !expected.matches(claim),
            ClaimCondition::In(allowed) => allowed.iter().any(|expected| expected.matches(claim)),
            ClaimCondition::NotIn(denied) => {
                is_scalar(claim) && !denied.iter().any(|expected| expected.matches(claim))
            }
            ClaimCondition::Contains(expected) => contains(claim, expected),
            ClaimCondition::ContainsAny(expected) => {
                expected.iter().any(|expected| contains(claim, expected))
            }
            ClaimCondition::ContainsAll(expected) => {
                expected.iter().all(|expected| contains(claim, expected))
            }
            ClaimCondition::Exists => true,
        }
    }
}

impl ClaimValue {
    fn matches(&self, claim: &Value) -> bool {
        match (self, claim) {
            (ClaimValue::Boolean(expected), Value::Bool(actual)) => expected == actual,
            (ClaimValue::Integer(expected), Value::Number(actual)) => {
                actual.as_i64() == Some(*expected)
            }
            (ClaimValue::String(expected), Value::String(actual)) => expected == actual,
            _ => false,
        }
    }
}

fn is_scalar(claim: &Value) -> bool {
    matches!(claim, Value::Bool(_) | Value::Number(_) | Value::String(_))
}

fn contains(claim: &Value, expected: &ClaimValue) -> bool {
    match claim {
        Value::Array(elements) => elements.iter().any(|element| expected.matches(element)),
        _ => expected.matches(claim),
    }
}

impl ClaimCondition {
    fn operator(&self) -> ClaimOperator {
        match self {
            ClaimCondition::Equals(_) => ClaimOperator::Equals,
            ClaimCondition::NotEquals(_) => ClaimOperator::NotEquals,
            ClaimCondition::In(_) => ClaimOperator::In,
            ClaimCondition::NotIn(_) => ClaimOperator::NotIn,
            ClaimCondition::Contains(_) => ClaimOperator::Contains,
            ClaimCondition::ContainsAny(_) => ClaimOperator::ContainsAny,
            ClaimCondition::ContainsAll(_) => ClaimOperator::ContainsAll,
            ClaimCondition::Exists => ClaimOperator::Exists,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawClaimRule {
    claim: String,
    operator: ClaimOperator,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<ClaimValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    values: Option<Vec<ClaimValue>>,
}

impl TryFrom<RawClaimRule> for ClaimRule {
    type Error = String;

    fn try_from(raw: RawClaimRule) -> Result<Self, Self::Error> {
        if raw.claim.trim().is_empty() {
            return Err("claim must not be empty".to_string());
        }
        let condition = match (raw.operator, raw.value, raw.values) {
            (ClaimOperator::Equals, Some(value), None) => ClaimCondition::Equals(value),
            (ClaimOperator::NotEquals, Some(value), None) => ClaimCondition::NotEquals(value),
            (ClaimOperator::In, None, Some(values)) if !values.is_empty() => {
                ClaimCondition::In(values)
            }
            (ClaimOperator::NotIn, None, Some(values)) if !values.is_empty() => {
                ClaimCondition::NotIn(values)
            }
            (ClaimOperator::Contains, Some(value), None) => ClaimCondition::Contains(value),
            (ClaimOperator::ContainsAny, None, Some(values)) if !values.is_empty() => {
                ClaimCondition::ContainsAny(values)
            }
            (ClaimOperator::ContainsAll, None, Some(values)) if !values.is_empty() => {
                ClaimCondition::ContainsAll(values)
            }
            (ClaimOperator::Exists, None, None) => ClaimCondition::Exists,
            (operator, _, _) => {
                return Err(format!(
                    "rule for claim '{}': operator '{}' {}",
                    raw.claim,
                    operator,
                    operator.operand_requirement()
                ))
            }
        };
        Ok(ClaimRule {
            claim: raw.claim,
            condition,
        })
    }
}

impl ClaimOperator {
    fn operand_requirement(&self) -> &'static str {
        match self {
            ClaimOperator::Equals | ClaimOperator::NotEquals | ClaimOperator::Contains => {
                "requires 'value' and does not accept 'values'"
            }
            ClaimOperator::In
            | ClaimOperator::NotIn
            | ClaimOperator::ContainsAny
            | ClaimOperator::ContainsAll => {
                "requires a non-empty 'values' list and does not accept 'value'"
            }
            ClaimOperator::Exists => "accepts neither 'value' nor 'values'",
        }
    }
}

impl From<ClaimRule> for RawClaimRule {
    fn from(rule: ClaimRule) -> Self {
        let operator = rule.condition.operator();
        let (value, values) = match rule.condition {
            ClaimCondition::Equals(value)
            | ClaimCondition::NotEquals(value)
            | ClaimCondition::Contains(value) => (Some(value), None),
            ClaimCondition::In(values)
            | ClaimCondition::NotIn(values)
            | ClaimCondition::ContainsAny(values)
            | ClaimCondition::ContainsAll(values) => (None, Some(values)),
            ClaimCondition::Exists => (None, None),
        };
        RawClaimRule {
            claim: rule.claim,
            operator,
            value,
            values,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn equals_matches_string_integer_and_boolean_claims() {
        assert_rule(
            json!({"claim": "appid", "operator": "equals", "value": "web-client"}),
            &[json!("web-client")],
            &[
                json!("cli-client"),
                json!("Web-Client"),
                json!(["web-client"]),
                json!({"appid": "web-client"}),
            ],
        );
        assert_rule(
            json!({"claim": "tenant_level", "operator": "equals", "value": 3}),
            &[json!(3)],
            &[json!("3"), json!(3.0), json!(4), json!([3]), json!(true)],
        );
        assert_rule(
            json!({"claim": "email_verified", "operator": "equals", "value": true}),
            &[json!(true)],
            &[json!("true"), json!(false), json!(1), json!([true])],
        );
    }

    #[test]
    fn not_equals_matches_string_integer_and_boolean_claims() {
        assert_rule(
            json!({"claim": "appid", "operator": "notEquals", "value": "legacy-client"}),
            &[json!("web-client"), json!(1), json!(true)],
            &[
                json!("legacy-client"),
                json!(["web-client"]),
                json!({"appid": "web-client"}),
            ],
        );
        assert_rule(
            json!({"claim": "tenant_level", "operator": "notEquals", "value": 0}),
            &[json!(3), json!("0"), json!(0.5)],
            &[json!(0), json!([3])],
        );
        assert_rule(
            json!({"claim": "email_verified", "operator": "notEquals", "value": false}),
            &[json!(true), json!("false")],
            &[json!(false), json!([true])],
        );
    }

    #[test]
    fn in_matches_string_integer_and_boolean_claims() {
        assert_rule(
            json!({"claim": "appid", "operator": "in", "values": ["web-client", "cli-client"]}),
            &[json!("web-client"), json!("cli-client")],
            &[json!("other"), json!("WEB-CLIENT"), json!(["web-client"])],
        );
        assert_rule(
            json!({"claim": "tenant_level", "operator": "in", "values": [2, 3]}),
            &[json!(2), json!(3)],
            &[json!(1), json!("3"), json!(3.0), json!([3])],
        );
        assert_rule(
            json!({"claim": "email_verified", "operator": "in", "values": [true]}),
            &[json!(true)],
            &[json!(false), json!("true"), json!([true])],
        );
    }

    #[test]
    fn not_in_matches_string_integer_and_boolean_claims() {
        assert_rule(
            json!({
                "claim": "appid",
                "operator": "notIn",
                "values": ["blocked-client", "legacy-client"]
            }),
            &[json!("web-client"), json!(7)],
            &[
                json!("blocked-client"),
                json!("legacy-client"),
                json!(["web-client"]),
                json!({}),
            ],
        );
        assert_rule(
            json!({"claim": "tenant_level", "operator": "notIn", "values": [0, 1]}),
            &[json!(3), json!("1")],
            &[json!(0), json!(1), json!([3])],
        );
        assert_rule(
            json!({"claim": "email_verified", "operator": "notIn", "values": [false]}),
            &[json!(true)],
            &[json!(false), json!([true])],
        );
    }

    #[test]
    fn contains_matches_string_integer_and_boolean_claims() {
        assert_rule(
            json!({"claim": "groups", "operator": "contains", "value": "admins"}),
            &[
                json!(["admins", "editors"]),
                json!("admins"),
                json!(["admins", "admins"]),
                json!([{"x": 1}, null, 1, ["editors"], "admins"]),
            ],
            &[
                json!(["editors"]),
                json!([]),
                json!("editors"),
                json!(["Admins"]),
                json!([["admins"]]),
                json!({"admins": true}),
            ],
        );
        assert_rule(
            json!({"claim": "clearance_levels", "operator": "contains", "value": 2}),
            &[json!([1, 2]), json!(2)],
            &[json!([1]), json!(["2"]), json!([2.0]), json!([])],
        );
        assert_rule(
            json!({"claim": "feature_flags", "operator": "contains", "value": true}),
            &[json!([true, false]), json!(true)],
            &[json!([false]), json!(["true"]), json!([])],
        );
    }

    #[test]
    fn contains_any_matches_string_integer_and_boolean_claims() {
        assert_rule(
            json!({"claim": "groups", "operator": "containsAny", "values": ["admins", "auditors"]}),
            &[
                json!(["admins", "editors"]),
                json!(["auditors"]),
                json!("auditors"),
            ],
            &[json!(["editors"]), json!([]), json!("editors"), json!({})],
        );
        assert_rule(
            json!({"claim": "clearance_levels", "operator": "containsAny", "values": [5, 1]}),
            &[json!([1, 2]), json!([5]), json!(1)],
            &[json!([2, 3]), json!(["1"]), json!([1.0]), json!([])],
        );
        assert_rule(
            json!({"claim": "feature_flags", "operator": "containsAny", "values": [false]}),
            &[json!([true, false]), json!(false)],
            &[json!([true]), json!(["false"]), json!([])],
        );
    }

    #[test]
    fn contains_all_matches_string_integer_and_boolean_claims() {
        assert_rule(
            json!({"claim": "groups", "operator": "containsAll", "values": ["admins", "editors"]}),
            &[
                json!(["admins", "editors"]),
                json!(["editors", "admins", "auditors"]),
                json!(["admins", "editors", "admins"]),
            ],
            &[json!(["admins"]), json!("admins"), json!([])],
        );
        assert_rule(
            json!({"claim": "clearance_levels", "operator": "containsAll", "values": [1, 2]}),
            &[json!([1, 2]), json!([2, 1, 3])],
            &[json!([1, 3]), json!([1, "2"]), json!(1), json!([])],
        );
        assert_rule(
            json!({"claim": "feature_flags", "operator": "containsAll", "values": [true, false]}),
            &[json!([true, false]), json!([false, true])],
            &[json!([true]), json!(true), json!(["true", "false"])],
        );
    }

    #[test]
    fn contains_all_accepts_scalar_claim_as_single_element_array() {
        assert_rule(
            json!({"claim": "groups", "operator": "containsAll", "values": ["admins"]}),
            &[json!("admins"), json!(["admins"])],
            &[json!("editors")],
        );
    }

    #[test]
    fn exists_matches_any_present_non_null_claim() {
        assert_rule(
            json!({"claim": "email_verified", "operator": "exists"}),
            &[
                json!(true),
                json!(false),
                json!("value"),
                json!(0),
                json!(1.5),
                json!([]),
                json!({}),
            ],
            &[],
        );
    }

    #[test]
    fn integers_outside_i64_range_never_match() {
        assert_rule(
            json!({"claim": "tenant_level", "operator": "equals", "value": -1}),
            &[json!(-1)],
            &[json!(u64::MAX)],
        );
        assert_rule(
            json!({"claim": "tenant_level", "operator": "in", "values": [i64::MAX, -1]}),
            &[json!(i64::MAX)],
            &[json!(u64::MAX)],
        );
        assert_rule(
            json!({"claim": "clearance_levels", "operator": "contains", "value": -1}),
            &[json!([-1])],
            &[json!([u64::MAX])],
        );
    }

    #[test]
    fn sample_token_satisfies_every_documented_example() {
        let rules = parse_rules(json!([
            {"claim": "appid", "operator": "equals", "value": "web-client"},
            {"claim": "tenant_level", "operator": "equals", "value": 3},
            {"claim": "email_verified", "operator": "equals", "value": true},
            {"claim": "appid", "operator": "notEquals", "value": "legacy-client"},
            {"claim": "tenant_level", "operator": "notEquals", "value": 0},
            {"claim": "email_verified", "operator": "notEquals", "value": false},
            {"claim": "appid", "operator": "in", "values": ["web-client", "cli-client"]},
            {"claim": "tenant_level", "operator": "in", "values": [2, 3]},
            {"claim": "email_verified", "operator": "in", "values": [true]},
            {"claim": "appid", "operator": "notIn", "values": ["blocked-client", "legacy-client"]},
            {"claim": "tenant_level", "operator": "notIn", "values": [0, 1]},
            {"claim": "email_verified", "operator": "notIn", "values": [false]},
            {"claim": "groups", "operator": "contains", "value": "admins"},
            {"claim": "clearance_levels", "operator": "contains", "value": 2},
            {"claim": "feature_flags", "operator": "contains", "value": true},
            {"claim": "groups", "operator": "containsAny", "values": ["admins", "auditors"]},
            {"claim": "clearance_levels", "operator": "containsAny", "values": [5, 1]},
            {"claim": "feature_flags", "operator": "containsAny", "values": [false]},
            {"claim": "groups", "operator": "containsAll", "values": ["admins", "editors"]},
            {"claim": "clearance_levels", "operator": "containsAll", "values": [1, 2]},
            {"claim": "feature_flags", "operator": "containsAll", "values": [true, false]},
            {"claim": "email_verified", "operator": "exists"},
            {"claim": "aud", "operator": "contains", "value": "api"}
        ]));

        assert_eq!(rules.evaluate(&sample_claims()), Ok(()));
    }

    #[test]
    fn evaluate_requires_every_rule_to_pass() {
        let rules = parse_rules(json!([
            {"claim": "appid", "operator": "equals", "value": "web-client"},
            {"claim": "groups", "operator": "contains", "value": "auditors"},
            {"claim": "tenant_level", "operator": "equals", "value": 4}
        ]));

        assert_eq!(
            rules.evaluate(&sample_claims()),
            Err(ClaimRuleViolation {
                claim: "groups".to_string(),
                operator: ClaimOperator::Contains,
            })
        );
    }

    #[test]
    fn empty_rules_accept_any_claims() {
        let rules = parse_claim_rules("[]").unwrap();

        assert_eq!(rules, ClaimRules::default());
        assert_eq!(rules.evaluate(&HashMap::new()), Ok(()));
        assert_eq!(rules.evaluate(&sample_claims()), Ok(()));
    }

    #[test]
    fn violation_describes_claim_and_operator() {
        let violation = ClaimRuleViolation {
            claim: "appid".to_string(),
            operator: ClaimOperator::ContainsAny,
        };

        assert_eq!(
            violation.to_string(),
            "claim 'appid' does not satisfy the 'containsAny' rule"
        );
    }

    #[test]
    fn parser_rejects_invalid_rules() {
        let invalid_rules = [
            json!({"claim": "appid", "operator": "equals"}),
            json!({"claim": "appid", "operator": "equals", "value": null}),
            json!({"claim": "appid", "operator": "equals", "values": ["a"]}),
            json!({"claim": "appid", "operator": "equals", "value": "a", "values": ["a"]}),
            json!({"claim": "appid", "operator": "in", "values": []}),
            json!({"claim": "appid", "operator": "in", "value": "a"}),
            json!({"claim": "appid", "operator": "in"}),
            json!({"claim": "groups", "operator": "contains", "value": ["a", "b"]}),
            json!({"claim": "groups", "operator": "containsAny", "values": [["a"]]}),
            json!({"claim": "groups", "operator": "containsAll", "values": [{"a": 1}]}),
            json!({"claim": "tenant_level", "operator": "equals", "value": 3.5}),
            json!({"claim": "tenant_level", "operator": "equals", "value": u64::MAX}),
            json!({"claim": "email_verified", "operator": "exists", "value": true}),
            json!({"claim": "email_verified", "operator": "exists", "values": [true]}),
            json!({"claim": "appid", "operator": "startsWith", "value": "web"}),
            json!({"claim": "appid", "operator": "Equals", "value": "web"}),
            json!({"claim": "appid", "value": "web"}),
            json!({"operator": "exists"}),
            json!({"claim": "", "operator": "exists"}),
            json!({"claim": "  ", "operator": "exists"}),
            json!({"claim": "appid", "operator": "equals", "value": "a", "extra": 1}),
        ];

        for rule in invalid_rules {
            let config = json!([rule]).to_string();
            assert!(
                parse_claim_rules(&config).is_err(),
                "expected {} to be rejected",
                config
            );
        }
        assert!(parse_claim_rules("{}").is_err());
        assert!(parse_claim_rules("not json").is_err());
    }

    #[test]
    fn parser_error_explains_operand_requirement() {
        let err = parse_claim_rules(r#"[{"claim": "appid", "operator": "equals"}]"#).unwrap_err();

        assert!(
            err.contains("rule for claim 'appid': operator 'equals' requires 'value'"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn rules_round_trip_through_serialization() {
        let config = json!([
            {"claim": "appid", "operator": "equals", "value": "web-client"},
            {"claim": "groups", "operator": "containsAny", "values": ["admins", 1, true]},
            {"claim": "email_verified", "operator": "exists"}
        ]);
        let rules = parse_rules(config.clone());

        assert_eq!(serde_json::to_value(&rules).unwrap(), config);
    }

    fn assert_rule(rule: Value, passing: &[Value], failing: &[Value]) {
        let claim = rule["claim"].as_str().unwrap().to_string();
        let rules = parse_rules(json!([rule]));

        for value in passing {
            assert_eq!(
                rules.evaluate(&claims_with(&claim, value.clone())),
                Ok(()),
                "{} must pass for {} = {}",
                rule,
                claim,
                value
            );
        }
        for value in failing.iter().chain([&Value::Null]) {
            assert!(
                rules.evaluate(&claims_with(&claim, value.clone())).is_err(),
                "{} must fail for {} = {}",
                rule,
                claim,
                value
            );
        }
        assert!(
            rules.evaluate(&HashMap::new()).is_err(),
            "{} must fail when {} is missing",
            rule,
            claim
        );
    }

    fn parse_rules(config: Value) -> ClaimRules {
        parse_claim_rules(&config.to_string()).unwrap()
    }

    fn sample_claims() -> HashMap<String, Value> {
        serde_json::from_value(json!({
            "sub": "user-1",
            "aud": "api",
            "appid": "web-client",
            "tenant_level": 3,
            "email_verified": true,
            "groups": ["admins", "editors"],
            "clearance_levels": [1, 2],
            "feature_flags": [true, false]
        }))
        .unwrap()
    }

    fn claims_with(claim: &str, value: Value) -> HashMap<String, Value> {
        HashMap::from([(claim.to_string(), value)])
    }
}
