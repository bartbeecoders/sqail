//! Helpers for encoding cell values as JSON (see `sqail_proto::LogicalType`).

use serde_json::Value;

pub fn float(v: f64) -> Value {
    serde_json::Number::from_f64(v)
        .map(Value::Number)
        .unwrap_or_else(|| Value::String(v.to_string()))
}

pub fn bytes(b: &[u8]) -> Value {
    Value::String(hex::encode(b))
}

pub fn text(s: impl Into<String>) -> Value {
    Value::String(s.into())
}

pub fn int(v: i64) -> Value {
    Value::Number(v.into())
}
