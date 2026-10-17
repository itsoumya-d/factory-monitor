//! Structured-output contract: every cloud answer must satisfy a JSON
//! schema before any business logic sees it (docs/09 §1 "structured-chat
//! with JSON-schema constrained output"). We ship a deliberately small
//! validator covering exactly the subset our prompts emit.

use serde_json::Value;

#[derive(Debug, Clone)]
pub struct JsonSchemaSpec {
    name: String,
    #[allow(dead_code)]
    shape: Value,
}

impl JsonSchemaSpec {
    pub fn new(name: impl Into<String>, shape: Value) -> Self {
        Self {
            name: name.into(),
            shape,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Validate a decoded provider answer against the spec. Returns the
    /// first violation (dotted path) so callers can `warn!` with context.
    pub fn validate(&self, value: &Value) -> Result<(), String> {
        validate(value, &self.shape, "$")
    }
}

fn validate(value: &Value, schema: &Value, path: &str) -> Result<(), String> {
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        return values
            .iter()
            .any(|v| v == value)
            .ok_or_else(|| format!("{path}: value {value} not in enum [{}]", list_of(values)))
            .map(|_| ());
    }
    match schema.get("type").and_then(Value::as_str) {
        Some("object") => {
            let obj = value
                .as_object()
                .ok_or(format!("{path}: expected object"))?;
            for req in schema
                .get("required")
                .and_then(Value::as_array)
                .map(|r| r.iter())
                .unwrap_or_default()
            {
                let key = req.as_str().unwrap_or_default();
                if !obj.contains_key(key) {
                    return Err(format!("{path}.{key}: missing required key"));
                }
            }
            if let Some(props) = schema.get("properties").and_then(Value::as_object) {
                for (key, sub_schema) in props {
                    if let Some(sub_value) = obj.get(key) {
                        validate(sub_value, sub_schema, &format!("{path}.{key}"))?;
                    }
                }
            }
            Ok(())
        }
        Some("array") => {
            let arr = value.as_array().ok_or(format!("{path}: expected array"))?;
            if let Some(items) = schema.get("items") {
                for (i, item) in arr.iter().enumerate() {
                    validate(item, items, &format!("{path}[{i}]"))?;
                }
            }
            Ok(())
        }
        Some("string") => {
            if value.is_string() {
                Ok(())
            } else {
                Err(format!("{path}: expected string"))
            }
        }
        Some("number") => {
            if value.is_number() {
                Ok(())
            } else {
                Err(format!("{path}: expected number"))
            }
        }
        Some("integer") => {
            if value.is_i64() || value.is_u64() {
                Ok(())
            } else {
                Err(format!("{path}: expected integer"))
            }
        }
        Some("boolean") => {
            if value.is_boolean() {
                Ok(())
            } else {
                Err(format!("{path}: expected boolean"))
            }
        }
        Some("null") => {
            if value.is_null() {
                Ok(())
            } else {
                Err(format!("{path}: expected null"))
            }
        }
        _ => Ok(()), // no constraint on this node
    }
}

fn list_of(values: &[Value]) -> String {
    values
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// The morning answer shape from docs/09 §2 (`expected` object).
pub fn layout_diff_schema() -> JsonSchemaSpec {
    JsonSchemaSpec::new(
        "factory_layout_diff",
        serde_json::json!({
            "type": "object",
            "required": ["detect_changes", "walkway_blocked", "new_equipment_needs_roi"],
            "properties": {
                "detect_changes": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "required": ["kind", "object", "zone_id", "blocking_counting_line"],
                        "properties": {
                            "kind": {
                                "enum": ["moved", "removed", "added", "occluded", "irrelevant"]
                            },
                            "object": { "type": "string" },
                            "zone_id": { "type": ["string", "null"] },
                            "blocking_counting_line": { "type": "boolean" },
                            "suggested_roi_fix": {
                                "type": ["object", "null"],
                                "required": ["zone_id", "new_line"],
                                "properties": {
                                    "zone_id": { "type": "string" },
                                    "new_line": {
                                        "type": "array",
                                        "items": {
                                            "type": "array",
                                            "items": { "type": "number" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                },
                "walkway_blocked": { "type": "boolean" },
                "new_equipment_needs_roi": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "required": ["object", "bbox_2d", "proposed_zone_id"],
                        "properties": {
                            "object": { "type": "string" },
                            "bbox_2d": {
                                "type": "array",
                                "items": { "type": "number" }
                            },
                            "proposed_zone_id": { "type": "string" }
                        }
                    }
                }
            }
        }),
    )
}

/// The EOD answer shape from docs/09 §3 step 4.
pub fn root_cause_schema() -> JsonSchemaSpec {
    JsonSchemaSpec::new(
        "root_cause_localization",
        serde_json::json!({
            "type": "object",
            "required": ["culprit_window", "root_cause", "corrected_boxes", "notes"],
            "properties": {
                "culprit_window": {
                    "type": "object",
                    "required": ["ts_start", "ts_end"],
                    "properties": {
                        "ts_start": { "type": "string" },
                        "ts_end": { "type": "string" }
                    }
                },
                "root_cause": { "type": "string" },
                "corrected_boxes": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "required": ["file_name", "bbox_2d", "category"],
                        "properties": {
                            "file_name": { "type": "string" },
                            "bbox_2d": {
                                "type": "array",
                                "items": { "type": "number" }
                            },
                            "category": { "type": "string" }
                        }
                    }
                },
                "notes": { "type": "string" }
            }
        }),
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn object_contract_is_enforced() {
        let spec = JsonSchemaSpec::new(
            "t",
            serde_json::json!({
                "type": "object",
                "required": ["a"],
                "properties": {
                    "a": {"type": "boolean"},
                    "b": {"enum": ["x", "y"]}
                }
            }),
        );
        spec.validate(&serde_json::json!({"a": true, "b": "x"}))
            .unwrap_or_else(|e| panic!("clean: {e}"));
        assert_eq!(
            spec.validate(&serde_json::json!({"a": "truthy"}))
                .unwrap_err(),
            "$.a: expected boolean"
        );
        assert_eq!(
            spec.validate(&serde_json::json!({})).unwrap_err(),
            "$.a: missing required key"
        );
        assert!(
            spec.validate(&serde_json::json!({"a": true, "b": "z"}))
                .unwrap_err()
                .contains("not in enum")
        );
    }

    #[test]
    fn array_items_recursed() {
        let spec = JsonSchemaSpec::new(
            "t",
            serde_json::json!({"type": "array", "items": {"type": "integer"}}),
        );
        spec.validate(&serde_json::json!([1, 2, 3]))
            .unwrap_or_else(|e| panic!("array: {e}"));
        assert_eq!(
            spec.validate(&serde_json::json!([1, "2"])).unwrap_err(),
            "$[1]: expected integer"
        );
        assert_eq!(
            spec.validate(&serde_json::json!("nope")).unwrap_err(),
            "$: expected array"
        );
    }

    #[test]
    fn builtin_layout_schema_accepts_docs_example() {
        let spec = layout_diff_schema();
        let example = serde_json::json!({
            "detect_changes": [
                {"kind": "moved", "object": "pallet", "zone_id": null,
                 "blocking_counting_line": true,
                 "suggested_roi_fix": null},
                {"kind": "removed", "object": "forklift", "zone_id": "line_2",
                 "blocking_counting_line": false,
                 "suggested_roi_fix": {"zone_id": "line_2", "new_line": [[10.0, 5.0], [50.0, 5.0]]}}
            ],
            "walkway_blocked": false,
            "new_equipment_needs_roi": [
                {"object": "compressor", "bbox_2d": [1.0, 2.0, 3.0, 4.0], "proposed_zone_id": "line_3"}
            ]
        });
        spec.validate(&example)
            .unwrap_or_else(|e| panic!("docs example must validate: {e}"));
    }

    #[test]
    fn builtin_root_cause_schema_accepts_docs_example() {
        let spec = root_cause_schema();
        let example = serde_json::json!({
            "culprit_window": {"ts_start": "2026-04-07T14:01:00Z", "ts_end": "2026-04-07T14:03:00Z"},
            "root_cause": "pallets misclassified as boxes",
            "corrected_boxes": [{"file_name": "f0.jpg", "bbox_2d": [1.0, 2.0, 3.0, 4.0], "category": "pallet"}],
            "notes": "see frame 3"
        });
        spec.validate(&example)
            .unwrap_or_else(|e| panic!("docs example must validate: {e}"));
    }
}
