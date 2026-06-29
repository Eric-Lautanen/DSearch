/// Sandboxed execution environment for scraper transforms.
///
/// Transforms are user-defined functions that modify scraped content
/// before it's stored. Since transforms run arbitrary logic, they
/// need to be sandboxed to prevent malicious code from affecting
/// the host system.
///
/// Phase 1: Simple string-based transforms (no code execution).
/// Future: WASM-based sandbox for arbitrary transforms.

/// Apply a named transform to a string.
///
/// Currently supports a small set of built-in transforms.
/// Returns the transformed string, or an error if the transform
/// is unknown or fails.
pub fn apply_transform(name: &str, input: &str) -> Result<String, String> {
    match name {
        "strip_html" => Ok(strip_html_tags(input)),
        "trim" => Ok(input.trim().to_string()),
        "lowercase" => Ok(input.to_lowercase()),
        "uppercase" => Ok(input.to_uppercase()),
        "strip_whitespace" => Ok(input.chars().filter(|c| !c.is_whitespace()).collect()),
        "normalize_whitespace" => Ok(normalize_whitespace(input)),
        "json_extract" => json_extract(input),
        "first_line" => Ok(input.lines().next().unwrap_or("").to_string()),
        "take_lines" => Ok(input.lines().take(10).collect::<Vec<_>>().join("\n")),
        _ => Err(format!("unknown transform: {}", name)),
    }
}

/// Strip HTML tags from a string, leaving only text content.
fn strip_html_tags(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut in_tag = false;

    for ch in input.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(ch),
            _ => {}
        }
    }

    // Decode common HTML entities (order matters: & must be last)
    result = result.replace("&nbsp;", " ");
    result = result.replace("<", "<");
    result = result.replace(">", ">");
    result = result.replace("\u{201C}", "\"");
    result = result.replace("\u{201D}", "\"");
    result = result.replace("&#39;", "'");
    result = result.replace("&", "&");
    result
}

/// Normalize whitespace: collapse runs of whitespace into single spaces.
fn normalize_whitespace(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut last_was_space = false;

    for ch in input.chars() {
        if ch.is_whitespace() {
            if !last_was_space {
                result.push(' ');
                last_was_space = true;
            }
        } else {
            result.push(ch);
            last_was_space = false;
        }
    }

    result.trim().to_string()
}

/// Try to extract meaningful text from a JSON response.
fn json_extract(input: &str) -> Result<String, String> {
    let value: serde_json::Value = serde_json::from_str(input)
        .map_err(|e| format!("json_extract: not valid JSON: {}", e))?;

    // Try common field names for the main content
    for field in &["body", "content", "text", "description", "summary", "value"] {
        if let Some(text) = value.get(field).and_then(|v| v.as_str()) {
            return Ok(text.to_string());
        }
    }

    // If it's a string at the top level, return it
    if let Some(text) = value.as_str() {
        return Ok(text.to_string());
    }

    // If it's an array, concatenate string elements
    if let Some(arr) = value.as_array() {
        let texts: Vec<String> = arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
        if !texts.is_empty() {
            return Ok(texts.join("\n"));
        }
    }

    // Fallback: pretty-print the JSON
    Ok(serde_json::to_string_pretty(&value).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_html_basic() {
        let input = "<p>Hello <b>world</b></p>";
        assert_eq!(strip_html_tags(input), "Hello world");
    }

    #[test]
    fn strip_html_entities() {
        // HTML entities are decoded after tag stripping
        let input = "a & b <em>bold</em> c";
        assert_eq!(strip_html_tags(input), "a & b bold c");
    }
    #[test]
    fn normalize_whitespace_collapses() {
        let input = "hello   \n\t  world";
        assert_eq!(normalize_whitespace(input), "hello world");
    }

    #[test]
    fn apply_transform_strip_html() {
        let result = apply_transform("strip_html", "<div>test</div>").unwrap();
        assert_eq!(result, "test");
    }

    #[test]
    fn apply_transform_unknown() {
        assert!(apply_transform("nonexistent", "input").is_err());
    }

    #[test]
    fn json_extract_body() {
        let input = r#"{"body": "hello", "other": 123}"#;
        assert_eq!(json_extract(input).unwrap(), "hello");
    }

    #[test]
    fn json_extract_array() {
        let input = r#"["a", "b", "c"]"#;
        assert_eq!(json_extract(input).unwrap(), "a\nb\nc");
    }

    #[test]
    fn apply_transform_lowercase() {
        assert_eq!(apply_transform("lowercase", "HELLO").unwrap(), "hello");
    }

    #[test]
    fn apply_transform_trim() {
        assert_eq!(apply_transform("trim", "  hello  ").unwrap(), "hello");
    }
}
