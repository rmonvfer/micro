//! The tool's input: JavaScript, optionally preceded by one options line.
//!
//! ```js
//! // @options: {"max_output_tokens": 2000, "timeout_ms": 30000}
//! const listing = await tools.ls({ path: "." });
//! text(listing);
//! ```

use serde_json::Value;

pub const OPTIONS_PREFIX: &str = "// @options:";

const SUPPORTED_FIELDS: &str = "`max_output_tokens` and `timeout_ms`";
/// The longest deadline a script may ask for, in milliseconds.
const MAX_TIMEOUT_MS: u64 = 2_147_483_647;

/// Lark grammar for providers that constrain tool input to a grammar. It only fixes the shape of
/// the options line; the options and the code are checked by [`parse`].
pub const SOURCE_GRAMMAR: &str = r#"
start: options_source | plain_source
options_source: OPTIONS_LINE NEWLINE SOURCE
plain_source: SOURCE

OPTIONS_LINE: /[ \t]*\/\/ @options:[^\r\n]*/
NEWLINE: /\r?\n/
SOURCE: /[\s\S]+/
"#;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SourceOptions {
    /// Token budget for the script's output.
    pub max_output_tokens: Option<u64>,
    /// Deadline for the whole script, tool calls included, in milliseconds.
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSource {
    /// The script with the options line taken out but its line kept, so line numbers are
    /// unchanged.
    pub code: String,
    pub options: SourceOptions,
}

/// Split an optional first-line `// @options: {...}` from the script.
pub fn parse(input: &str) -> Result<ParsedSource, String> {
    if input.trim().is_empty() {
        return Err("Expected JavaScript source text (non-empty). Provide JS only, optionally \
             with a first line `// @options: {\"max_output_tokens\": 1000}`."
            .to_string());
    }
    let (first_line, rest) = match input.find('\n') {
        Some(newline) => (&input[..newline], &input[newline..]),
        None => (input, ""),
    };
    let first_line = first_line.strip_suffix('\r').unwrap_or(first_line);
    let Some(directive) = first_line.trim_start().strip_prefix(OPTIONS_PREFIX) else {
        return Ok(ParsedSource {
            code: input.to_string(),
            options: SourceOptions::default(),
        });
    };
    if rest.trim().is_empty() {
        return Err(
            "The @options line must be followed by JavaScript source on subsequent lines"
                .to_string(),
        );
    }
    Ok(ParsedSource {
        code: rest.to_string(),
        options: parse_options(directive.trim())?,
    })
}

fn parse_options(directive: &str) -> Result<SourceOptions, String> {
    let shape = || format!("@options must be a JSON object with supported fields {SUPPORTED_FIELDS}");
    if directive.is_empty() {
        return Err(shape());
    }
    let value: Value = serde_json::from_str(directive).map_err(|error| {
        format!("@options must be valid JSON with supported fields {SUPPORTED_FIELDS}: {error}")
    })?;
    let Value::Object(fields) = value else {
        return Err(shape());
    };
    if let Some(unknown) = fields
        .keys()
        .find(|key| !["max_output_tokens", "timeout_ms"].contains(&key.as_str()))
    {
        return Err(format!(
            "@options only supports {SUPPORTED_FIELDS}; got `{unknown}`"
        ));
    }
    let mut options = SourceOptions::default();
    if let Some(value) = fields.get("max_output_tokens") {
        options.max_output_tokens = Some(value.as_u64().ok_or_else(|| {
            "@options field `max_output_tokens` must be a non-negative safe integer".to_string()
        })?);
    }
    if let Some(value) = fields.get("timeout_ms") {
        options.timeout_ms = Some(
            value
                .as_u64()
                .filter(|timeout| (1..=MAX_TIMEOUT_MS).contains(timeout))
                .ok_or_else(|| {
                    format!(
                        "@options field `timeout_ms` must be a positive integer up to \
                         {MAX_TIMEOUT_MS}"
                    )
                })?,
        );
    }
    Ok(options)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_source_has_no_options() {
        let parsed = parse("return 1;").unwrap();
        assert_eq!(parsed.code, "return 1;");
        assert_eq!(parsed.options, SourceOptions::default());
    }

    #[test]
    fn the_options_line_is_read_and_its_line_kept() {
        let parsed =
            parse("// @options: {\"max_output_tokens\": 50, \"timeout_ms\": 1000}\nreturn 1;")
                .unwrap();
        assert_eq!(parsed.code, "\nreturn 1;");
        assert_eq!(parsed.options.max_output_tokens, Some(50));
        assert_eq!(parsed.options.timeout_ms, Some(1000));
    }

    #[test]
    fn bad_options_are_explained() {
        assert!(parse("").unwrap_err().contains("non-empty"));
        assert!(parse("// @options: {}").unwrap_err().contains("followed by"));
        assert!(parse("// @options: nope\nx")
            .unwrap_err()
            .contains("valid JSON"));
        assert!(parse("// @options: {\"other\": 1}\nx")
            .unwrap_err()
            .contains("`other`"));
        assert!(parse("// @options: {\"timeout_ms\": 0}\nx")
            .unwrap_err()
            .contains("positive"));
        assert!(parse("// @options: {\"max_output_tokens\": -1}\nx")
            .unwrap_err()
            .contains("non-negative"));
        assert!(parse("// @options: []\nx")
            .unwrap_err()
            .contains("JSON object"));
    }
}
