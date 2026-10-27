//! JSON with Python's key order and `json.dumps` formatting, so prompts and
//! hashed fingerprints match the Python service byte for byte.

use serde::ser::{SerializeMap, SerializeSeq};
use serde::{Serialize, Serializer};

/// A JSON value whose objects keep insertion order.
#[derive(Clone, Debug, PartialEq)]
pub enum J {
    Null,
    Bool(bool),
    /// The number as Python prints it.
    Num(String),
    Str(String),
    Arr(Vec<J>),
    Obj(Vec<(String, J)>),
}

impl J {
    pub fn str(value: impl Into<String>) -> Self {
        Self::Str(value.into())
    }

    pub fn opt(value: Option<impl Into<String>>) -> Self {
        value.map_or(Self::Null, |v| Self::Str(v.into()))
    }

    pub fn obj<const N: usize>(pairs: [(&str, J); N]) -> Self {
        Self::Obj(pairs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
    }

    pub fn strs<S: AsRef<str>>(values: impl IntoIterator<Item = S>) -> Self {
        Self::Arr(
            values
                .into_iter()
                .map(|v| Self::Str(v.as_ref().to_owned()))
                .collect(),
        )
    }

    /// `json.dumps(value, ensure_ascii=True)` with the default separators
    /// (`", "`, `": "`) and insertion order.
    pub fn dumps(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, false, false);
        out
    }

    /// `json.dumps(value, ensure_ascii=True, sort_keys=True, separators=(",", ":"))`.
    pub fn dumps_sorted_compact(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, true, true);
        out
    }

    fn write(&self, out: &mut String, sort: bool, compact: bool) {
        let (item, key) = if compact { (",", ":") } else { (", ", ": ") };
        match self {
            Self::Null => out.push_str("null"),
            Self::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Self::Num(n) => out.push_str(n),
            Self::Str(s) => ascii_string(s, out),
            Self::Arr(items) => {
                out.push('[');
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        out.push_str(item);
                    }
                    v.write(out, sort, compact);
                }
                out.push(']');
            }
            Self::Obj(pairs) => {
                let mut pairs: Vec<&(String, J)> = pairs.iter().collect();
                if sort {
                    pairs.sort_by(|a, b| a.0.cmp(&b.0));
                }
                out.push('{');
                for (i, (k, v)) in pairs.into_iter().enumerate() {
                    if i > 0 {
                        out.push_str(item);
                    }
                    ascii_string(k, out);
                    out.push_str(key);
                    v.write(out, sort, compact);
                }
                out.push('}');
            }
        }
    }
}

/// A JSON string literal with every non-ASCII character as `\uXXXX`.
fn ascii_string(text: &str, out: &mut String) {
    let encoded = serde_json::to_string(text).unwrap_or_default();
    for c in encoded.chars() {
        if c.is_ascii() {
            out.push(c);
        } else {
            let mut units = [0_u16; 2];
            for unit in c.encode_utf16(&mut units) {
                out.push_str(&format!("\\u{unit:04x}"));
            }
        }
    }
}

/// Request bodies keep the same order (TypeSafe sees the options as listed).
impl Serialize for J {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Null => serializer.serialize_unit(),
            Self::Bool(b) => serializer.serialize_bool(*b),
            Self::Num(n) => match (n.parse::<i64>(), n.parse::<f64>()) {
                (Ok(i), _) => serializer.serialize_i64(i),
                (_, Ok(f)) => serializer.serialize_f64(f),
                _ => serializer.serialize_str(n),
            },
            Self::Str(s) => serializer.serialize_str(s),
            Self::Arr(items) => {
                let mut seq = serializer.serialize_seq(Some(items.len()))?;
                for v in items {
                    seq.serialize_element(v)?;
                }
                seq.end()
            }
            Self::Obj(pairs) => {
                let mut map = serializer.serialize_map(Some(pairs.len()))?;
                for (k, v) in pairs {
                    map.serialize_entry(k, v)?;
                }
                map.end()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_like_python_json_dumps() {
        let value = J::obj([
            ("b", J::str("café")),
            ("a", J::Arr(vec![J::Null, J::Num("1".into())])),
        ]);
        assert_eq!(value.dumps(), r#"{"b": "caf\u00e9", "a": [null, 1]}"#);
        assert_eq!(
            value.dumps_sorted_compact(),
            r#"{"a":[null,1],"b":"caf\u00e9"}"#
        );
        assert_eq!(
            serde_json::to_string(&value).unwrap(),
            r#"{"b":"café","a":[null,1]}"#
        );
    }
}
