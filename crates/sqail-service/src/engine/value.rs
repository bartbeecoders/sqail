//! Helpers for encoding cell values as JSON (see `sqail_proto::LogicalType`).

use serde_json::Value;

/// A finite float as a JSON number; NaN and the infinities, which JSON
/// lacks, as the strings `"NaN"`, `"Infinity"` and `"-Infinity"` (the server
/// spelling).
pub fn float(v: f64) -> Value {
    serde_json::Number::from_f64(v)
        .map(Value::Number)
        .unwrap_or_else(|| {
            text(if v.is_nan() {
                "NaN"
            } else if v > 0.0 {
                "Infinity"
            } else {
                "-Infinity"
            })
        })
}

/// A 4-byte float with its own shortest digits (`0.1`), not those of the
/// f64 it widens to (`0.10000000149011612`).
pub fn float32(v: f32) -> Value {
    float(v.to_string().parse().unwrap_or(v.into()))
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn floats() {
        assert_eq!(float(1.5), json!(1.5));
        assert_eq!(float(f64::NAN), json!("NaN"));
        assert_eq!(float(f64::INFINITY), json!("Infinity"));
        assert_eq!(float(f64::NEG_INFINITY), json!("-Infinity"));
        assert_eq!(float32(0.1), json!(0.1));
        assert_eq!(float32(f32::MAX), json!(3.4028235e38));
        assert_eq!(float32(1e-45), json!(1e-45));
        assert_eq!(float32(f32::NEG_INFINITY), json!("-Infinity"));
    }
}
