//! JSON as package.json and tsconfig.json hold it. An exports map matches
//! its conditions in the order the keys are written, which serde_json's
//! Value doesn't keep; tsconfig.json may have comments and trailing commas.

use std::fmt;

use serde::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Number,
    Str(String),
    Array(Vec<Json>),
    /// The keys in the order written.
    Object(Vec<(String, Json)>),
}

impl Json {
    pub fn parse(text: &str) -> serde_json::Result<Json> {
        serde_json::from_str(text)
    }

    /// JSON with comments and trailing commas.
    pub fn parse_jsonc(text: &str) -> serde_json::Result<Json> {
        Json::parse(&strip_jsonc(text.strip_prefix('\u{feff}').unwrap_or(text)))
    }

    /// The value of a key; the last, when it's written twice.
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(entries) => entries.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn entries(&self) -> &[(String, Json)] {
        match self {
            Json::Object(entries) => entries,
            _ => &[],
        }
    }
}

impl<'de> Deserialize<'de> for Json {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Json, D::Error> {
        deserializer.deserialize_any(JsonVisitor)
    }
}

struct JsonVisitor;

impl<'de> Visitor<'de> for JsonVisitor {
    type Value = Json;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("JSON")
    }

    fn visit_unit<E>(self) -> Result<Json, E> {
        Ok(Json::Null)
    }

    fn visit_none<E>(self) -> Result<Json, E> {
        Ok(Json::Null)
    }

    fn visit_bool<E>(self, b: bool) -> Result<Json, E> {
        Ok(Json::Bool(b))
    }

    fn visit_i64<E>(self, _: i64) -> Result<Json, E> {
        Ok(Json::Number)
    }

    fn visit_u64<E>(self, _: u64) -> Result<Json, E> {
        Ok(Json::Number)
    }

    fn visit_f64<E>(self, _: f64) -> Result<Json, E> {
        Ok(Json::Number)
    }

    fn visit_str<E>(self, s: &str) -> Result<Json, E> {
        Ok(Json::Str(s.to_string()))
    }

    fn visit_string<E>(self, s: String) -> Result<Json, E> {
        Ok(Json::Str(s))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Json, A::Error> {
        let mut items = vec![];
        while let Some(item) = seq.next_element()? {
            items.push(item);
        }
        Ok(Json::Array(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Json, A::Error> {
        let mut entries = vec![];
        while let Some(entry) = map.next_entry()? {
            entries.push(entry);
        }
        Ok(Json::Object(entries))
    }
}

/// JSON with the comments and the commas before `}` and `]` taken out,
/// strings left as they are.
fn strip_jsonc(text: &str) -> String {
    let mut plain = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, chars.peek()) {
            ('"', _) => string(c, &mut chars, &mut plain),
            ('/', Some('/')) => {
                chars.find(|&c| c == '\n');
                plain.push('\n');
            }
            ('/', Some('*')) => {
                chars.next();
                let mut last = ' ';
                chars.find(|&c| std::mem::replace(&mut last, c) == '*' && c == '/');
                plain.push(' ');
            }
            _ => plain.push(c),
        }
    }
    let mut out = String::with_capacity(plain.len());
    let mut chars = plain.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => string(c, &mut chars, &mut out),
            ',' if matches!(chars.clone().find(|c| !c.is_whitespace()), Some('}' | ']')) => {}
            _ => out.push(c),
        }
    }
    out
}

/// Copies a string, its opening quote read, through its closing quote.
fn string(quote: char, chars: &mut impl Iterator<Item = char>, out: &mut String) {
    out.push(quote);
    while let Some(c) = chars.next() {
        out.push(c);
        match c {
            '\\' => out.extend(chars.next()),
            '"' => return,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn objects_keep_their_keys_in_the_order_written() {
        let json = Json::parse(r#"{"z":{"node":"./n.js","import":"./i.js"},"a":[1,null,true,"s"],"z":"last"}"#).unwrap();
        assert_eq!(json.get("z"), Some(&Json::Str("last".into())), "the last of a key written twice");
        let Json::Object(entries) = &json else { panic!() };
        let keys: Vec<&str> = entries[0].1.entries().iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["node", "import"]);
        assert_eq!(json.get("a"), Some(&Json::Array(vec![Json::Number, Json::Null, Json::Bool(true), Json::Str("s".into())])));
        assert_eq!(Json::Str("x".into()).get("a"), None);
    }

    #[test]
    fn comments_and_trailing_commas_are_read_strings_are_kept() {
        let text = r#"{
            // a comment, with a comma,
            "compilerOptions": {
                /* "verbatimModuleSyntax": false, */
                "verbatimModuleSyntax": true, // "x": 1,
                "paths": { "a//b": ["./c/*"], "q\"/*": [], },
            },
        }"#;
        let json = Json::parse_jsonc(text).unwrap();
        let options = json.get("compilerOptions").unwrap();
        assert_eq!(options.get("verbatimModuleSyntax").and_then(Json::as_bool), Some(true));
        let paths = options.get("paths").unwrap().entries();
        assert_eq!(paths.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["a//b", "q\"/*"]);
        assert_eq!(Json::parse_jsonc("[1, 2, /* x */ ]").unwrap(), Json::Array(vec![Json::Number, Json::Number]));
        assert!(Json::parse_jsonc(r#"{"a": 1 /* open"#).is_err());
        assert!(Json::parse_jsonc("[1,,]").is_err(), "only one comma goes");
    }
}
