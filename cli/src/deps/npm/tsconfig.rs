//! tsconfig.json's `verbatimModuleSyntax`, which a TypeScript app's
//! deps.json needs: whether it's set, and the file with it set, written
//! into the text as it is, comments and all.

use anyhow::{Context as _, bail};

use super::json::Json;

/// Whether the tsconfig.json `text` sets `"verbatimModuleSyntax": true`.
pub fn sets_verbatim_module_syntax(text: &str) -> anyhow::Result<bool> {
    let config = Json::parse_jsonc(text).context("tsconfig.json is not JSON")?;
    let set = config.get("compilerOptions").and_then(|o| o.get("verbatimModuleSyntax")).and_then(Json::as_bool);
    Ok(set == Some(true))
}

/// The tsconfig.json `text` with `"verbatimModuleSyntax": true` in its
/// compilerOptions, or None when it's set: a value there is changed, else
/// the option is written first.
pub fn with_verbatim_module_syntax(text: &str) -> anyhow::Result<Option<String>> {
    if sets_verbatim_module_syntax(text)? {
        return Ok(None);
    }
    let tokens = tokens(text)?;
    if tokens.first().map(|t| &t.kind) != Some(&Kind::Open) {
        bail!("tsconfig.json is not an object");
    }
    let edited = match member(&tokens, 0, "compilerOptions") {
        None => insert(text, &tokens, 0, r#""compilerOptions": { "verbatimModuleSyntax": true }"#),
        Some(options) if tokens[options].kind == Kind::Open => match member(&tokens, options, "verbatimModuleSyntax") {
            None => insert(text, &tokens, options, r#""verbatimModuleSyntax": true"#),
            Some(value) if matches!(tokens[value].kind, Kind::Str(_) | Kind::Word) => {
                format!("{}true{}", &text[..tokens[value].start], &text[tokens[value].end..])
            }
            Some(_) => bail!("tsconfig.json's verbatimModuleSyntax is not true or false"),
        },
        Some(_) => bail!("tsconfig.json's compilerOptions is not an object"),
    };
    if !sets_verbatim_module_syntax(&edited)? {
        bail!("could not set verbatimModuleSyntax in tsconfig.json");
    }
    Ok(Some(edited))
}

#[derive(Debug, PartialEq)]
enum Kind {
    Open,
    Close,
    Colon,
    Comma,
    Str(String),
    /// Anything else: a number, true, false, null, an array's brackets.
    Word,
}

struct Token {
    kind: Kind,
    start: usize,
    end: usize,
}

/// The tokens of JSON with comments, by their place in the text.
fn tokens(text: &str) -> anyhow::Result<Vec<Token>> {
    let bytes = text.as_bytes();
    let mut tokens = vec![];
    let mut i = if text.starts_with('\u{feff}') { 3 } else { 0 };
    while i < bytes.len() {
        let start = i;
        let kind = match bytes[i] {
            b' ' | b'\t' | b'\r' | b'\n' => {
                i += 1;
                continue;
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                i = text[i..].find('\n').map_or(bytes.len(), |n| i + n);
                continue;
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i = text[i + 2..].find("*/").map(|n| i + 2 + n + 2).context("tsconfig.json has a comment that isn't closed")?;
                continue;
            }
            b'{' => Kind::Open,
            b'}' => Kind::Close,
            b':' => Kind::Colon,
            b',' => Kind::Comma,
            b'[' | b']' => Kind::Word,
            b'"' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    i += if bytes[i] == b'\\' { 2 } else { 1 };
                }
                i = (i + 1).min(bytes.len());
                Kind::Str(serde_json::from_str(&text[start..i]).context("tsconfig.json has a string that isn't one")?)
            }
            _ => {
                while i < bytes.len() && !b" \t\r\n{}[]:,/\"".contains(&bytes[i]) {
                    i += 1;
                }
                Kind::Word
            }
        };
        if i == start {
            i += 1;
        }
        tokens.push(Token { kind, start, end: i });
    }
    Ok(tokens)
}

/// The token of the value of `key` in the object opening at token `open`;
/// the last, when it's written twice.
fn member(tokens: &[Token], open: usize, key: &str) -> Option<usize> {
    let mut depth = 0;
    let mut found = None;
    for i in open + 1..tokens.len() {
        match &tokens[i].kind {
            Kind::Open => depth += 1,
            Kind::Close if depth == 0 => break,
            Kind::Close => depth -= 1,
            Kind::Str(s) if depth == 0 && s == key && tokens.get(i + 1).is_some_and(|t| t.kind == Kind::Colon) && i + 2 < tokens.len() => {
                found = Some(i + 2);
            }
            _ => {}
        }
    }
    found
}

/// The text with `entry` written as the first member of the object
/// opening at token `open`: on a line of its own, indented as the next
/// member, when that one is on its own line.
fn insert(text: &str, tokens: &[Token], open: usize, entry: &str) -> String {
    let at = tokens[open].end;
    let next = &tokens[open + 1..].first().map_or(text.len(), |t| t.start);
    let between = &text[at..*next];
    let empty = tokens.get(open + 1).is_some_and(|t| t.kind == Kind::Close);
    let written = if empty {
        format!(" {entry} ")
    } else if let Some(line) = text[..*next].rfind('\n') {
        if line >= at {
            let newline = if between.contains("\r\n") { "\r\n" } else { "\n" };
            let indent: String = text[line + 1..].chars().take_while(|c| *c == ' ' || *c == '\t').collect();
            format!("{newline}{indent}{entry},")
        } else {
            format!(" {entry},")
        }
    } else {
        format!(" {entry},")
    };
    format!("{}{written}{}", &text[..at], &text[at..])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(text: &str) -> String {
        with_verbatim_module_syntax(text).unwrap().unwrap()
    }

    #[test]
    fn the_option_is_written_into_the_file_as_it_is() {
        let template = "{\n  \"compilerOptions\": {\n    // ES2022\n    \"target\": \"ES2022\",\n    \"strict\": true,\n  },\n  \"include\": [\"src\"]\n}\n";
        assert_eq!(
            set(template),
            "{\n  \"compilerOptions\": {\n    \"verbatimModuleSyntax\": true,\n    // ES2022\n    \"target\": \"ES2022\",\n    \"strict\": true,\n  },\n  \"include\": [\"src\"]\n}\n"
        );
        assert_eq!(set(r#"{"compilerOptions": {"strict": true}}"#), r#"{"compilerOptions": { "verbatimModuleSyntax": true,"strict": true}}"#);
        assert_eq!(set(r#"{"compilerOptions": {}}"#), r#"{"compilerOptions": { "verbatimModuleSyntax": true }}"#);
        assert_eq!(set("{\r\n\t\"include\": []\r\n}"), "{\r\n\t\"compilerOptions\": { \"verbatimModuleSyntax\": true },\r\n\t\"include\": []\r\n}");
        assert_eq!(set("{}"), r#"{ "compilerOptions": { "verbatimModuleSyntax": true } }"#);
        assert_eq!(set("\u{feff}{}"), "\u{feff}{ \"compilerOptions\": { \"verbatimModuleSyntax\": true } }");
    }

    #[test]
    fn a_value_written_is_changed_and_one_set_is_left() {
        assert_eq!(
            set(r#"{"compilerOptions": {"verbatimModuleSyntax": false /* no */, "strict": true}}"#),
            r#"{"compilerOptions": {"verbatimModuleSyntax": true /* no */, "strict": true}}"#
        );
        assert_eq!(
            set(r#"{"compilerOptions": {"verbatimModuleSyntax": true, "verbatimModuleSyntax": false}}"#),
            r#"{"compilerOptions": {"verbatimModuleSyntax": true, "verbatimModuleSyntax": true}}"#,
            "the last one counts"
        );
        assert_eq!(
            set(r#"{"x": {"compilerOptions": {"verbatimModuleSyntax": true}}, "compilerOptions": {"paths": {"verbatimModuleSyntax": ["a"]}}}"#),
            r#"{"x": {"compilerOptions": {"verbatimModuleSyntax": true}}, "compilerOptions": { "verbatimModuleSyntax": true,"paths": {"verbatimModuleSyntax": ["a"]}}}"#,
            "only the options' own"
        );
        let set_already = "{\n  // \"verbatimModuleSyntax\": false\n  \"compilerOptions\": {\"verbatimModuleSyntax\": true}\n}";
        assert_eq!(with_verbatim_module_syntax(set_already).unwrap(), None);
        assert!(sets_verbatim_module_syntax(set_already).unwrap());
        assert!(!sets_verbatim_module_syntax(r#"{"compilerOptions": {"verbatimModuleSyntax": "true"}}"#).unwrap());
        assert_eq!(set(r#"{"compilerOptions": {"verbatimModuleSyntax": "true"}}"#), r#"{"compilerOptions": {"verbatimModuleSyntax": true}}"#);
    }

    #[test]
    fn what_is_not_a_tsconfig_is_left_alone() {
        for text in ["", "[]", "{", r#"{"compilerOptions": []}"#, r#"{"compilerOptions": {"verbatimModuleSyntax": {}}}"#, "{/* open"] {
            assert!(with_verbatim_module_syntax(text).is_err(), "{text:?}");
        }
    }
}
