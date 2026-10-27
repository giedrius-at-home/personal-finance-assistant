//! Text exactly as the Python service wrote it: `str(Decimal)`,
//! `json.dumps(..., sort_keys=True, separators=(",", ":"))` and
//! `datetime.isoformat()` of a UTC instant. Hashes and stored rows depend on
//! these byte for byte.

use std::fmt::Write as _;

use jiff::Timestamp;
use rust_decimal::Decimal;
use serde_json::Value;

/// Python's `str(Decimal)`: plain notation unless the exponent is positive
/// or the value is smaller than 1E-6, then scientific (`1E-7`, `0E-8`).
pub fn decimal_str(d: Decimal) -> String {
    let digits = d.mantissa().unsigned_abs().to_string();
    let exponent = -i64::from(d.scale());
    let length = digits.len() as i64;
    let adjusted = exponent + length - 1;
    let sign = if d.is_sign_negative() && !d.is_zero() {
        "-"
    } else {
        ""
    };
    if exponent <= 0 && adjusted >= -6 {
        if exponent == 0 {
            return format!("{sign}{digits}");
        }
        let point = length + exponent;
        if point > 0 {
            let (whole, fraction) = digits.split_at(point as usize);
            return format!("{sign}{whole}.{fraction}");
        }
        let zeros = "0".repeat((-point) as usize);
        return format!("{sign}0.{zeros}{digits}");
    }
    let (first, rest) = digits.split_at(1);
    let fraction = if rest.is_empty() {
        String::new()
    } else {
        format!(".{rest}")
    };
    let exp_sign = if adjusted >= 0 { "+" } else { "-" };
    format!("{sign}{first}{fraction}E{exp_sign}{}", adjusted.abs())
}

/// `json.dumps(value, sort_keys=True, separators=(",", ":"))` with the
/// default `ensure_ascii=True`.
pub fn json_canonical(value: &Value) -> String {
    let mut out = String::new();
    write_json(value, &mut out);
    out
}

fn write_json(value: &Value, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&n.to_string()),
        Value::String(s) => write_str(s, out),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json(item, out);
            }
            out.push(']');
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (i, key) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_str(key, out);
                out.push(':');
                write_json(&map[key], out);
            }
            out.push('}');
        }
    }
}

fn write_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 || (c as u32) > 0x7e => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    let _ = write!(out, "\\u{unit:04x}");
                }
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// `t` cut to whole microseconds, as Python's `datetime` holds it.
pub fn micros(t: Timestamp) -> Timestamp {
    let nanos = t.as_nanosecond();
    Timestamp::from_nanosecond(nanos - nanos.rem_euclid(1000)).unwrap_or(t)
}

/// `datetime.isoformat()` of a UTC-aware instant: microseconds only when
/// there are any, and a `+00:00` offset.
pub fn isoformat_utc(t: Timestamp) -> String {
    let t = micros(t);
    if t.subsec_nanosecond() == 0 {
        t.strftime("%Y-%m-%dT%H:%M:%S+00:00").to_string()
    } else {
        t.strftime("%Y-%m-%dT%H:%M:%S%.6f+00:00").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    #[test]
    fn decimal_text_matches_python() {
        assert_eq!(decimal_str(d("340.00")), "340.00");
        assert_eq!(
            decimal_str(d("-919.5402298850574712643678162")),
            "-919.5402298850574712643678162"
        );
        assert_eq!(decimal_str(d("0")), "0");
        assert_eq!(decimal_str(d("0.000001")), "0.000001");
        assert_eq!(decimal_str(d("0.0000001")), "1E-7");
        assert_eq!(decimal_str(d("0.00000000")), "0E-8");
        assert_eq!(decimal_str(d("0.00000012")), "1.2E-7");
        assert_eq!(decimal_str(d("12")), "12");
    }

    #[test]
    fn canonical_json_sorts_and_escapes() {
        let v = json!({"b": [1, "é"], "a": {"z": null, "y": true}, "c": "\"x\"\n"});
        assert_eq!(
            json_canonical(&v),
            "{\"a\":{\"y\":true,\"z\":null},\"b\":[1,\"\\u00e9\"],\"c\":\"\\\"x\\\"\\n\"}"
        );
        assert_eq!(json_canonical(&json!("\u{1f440}")), "\"\\ud83d\\udc40\"");
    }

    #[test]
    fn isoformat_drops_zero_microseconds() {
        let t: Timestamp = "2026-12-03T08:00:00Z".parse().unwrap();
        assert_eq!(isoformat_utc(t), "2026-12-03T08:00:00+00:00");
        let t: Timestamp = "2026-10-27T10:00:00.1234567Z".parse().unwrap();
        assert_eq!(isoformat_utc(t), "2026-10-27T10:00:00.123456+00:00");
    }
}
