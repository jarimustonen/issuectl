//! Versioned JSON envelopes for the public CLI surface.

use serde::Serialize;
use serde_json::Value;

/// The version of issuectl's JSON output envelope and its contained data.
///
/// Bump this only for breaking changes to the JSON API. Additive fields do not
/// require a bump. This is independent from the on-disk issue schema version.
pub const CLI_SCHEMA_VERSION: u32 = 1;

/// Wrap a successful CLI result, moving any legacy top-level `warnings` field
/// into the canonical envelope.
pub fn success<T: Serialize>(data: &T) -> serde_json::Result<Value> {
    let mut data = serde_json::to_value(data)?;
    let warnings = match &mut data {
        Value::Object(object) => object
            .remove("warnings")
            .filter(Value::is_array)
            .unwrap_or_else(|| Value::Array(Vec::new())),
        _ => Value::Array(Vec::new()),
    };
    Ok(serde_json::json!({
        "schema_version": CLI_SCHEMA_VERSION,
        "data": data,
        "warnings": warnings,
    }))
}

/// Wrap a JSON error payload for stderr.
pub fn error(code: &str, message: &str, extra: Value) -> Value {
    let mut error = serde_json::Map::new();
    error.insert("code".into(), Value::String(code.into()));
    error.insert("message".into(), Value::String(message.into()));
    if let Value::Object(extra) = extra {
        error.extend(extra);
    }
    serde_json::json!({"schema_version": CLI_SCHEMA_VERSION, "error": error})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_wraps_data_and_lifts_warnings() {
        let value = success(&serde_json::json!({"value": 1, "warnings": ["old"]})).unwrap();
        assert_eq!(value["schema_version"], CLI_SCHEMA_VERSION);
        assert_eq!(value["data"], serde_json::json!({"value": 1}));
        assert_eq!(value["warnings"], serde_json::json!(["old"]));
    }

    #[test]
    fn error_preserves_extra_fields() {
        let value = error("not-found", "missing", serde_json::json!({"slug": "x"}));
        assert_eq!(value["error"]["code"], "not-found");
        assert_eq!(value["error"]["slug"], "x");
    }
}
