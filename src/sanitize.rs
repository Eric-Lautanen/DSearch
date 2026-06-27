/// Sanitize a string for safe storage and display.
///
/// Rules:
/// - Valid UTF-8 only
/// - No control characters 0x00–0x1F except 0x0A (newline)
/// - No Unicode Cf (format) or Cc (control) categories
/// - Caps: 1 MB record, 256 B key, 64 KB value

/// Maximum record size in bytes.
pub const MAX_RECORD_SIZE: usize = 1_048_576;
/// Maximum key size in bytes.
pub const MAX_KEY_SIZE: usize = 256;
/// Maximum value size in bytes.
pub const MAX_VALUE_SIZE: usize = 65_536;

/// Sanitize a string: enforce valid UTF-8, remove disallowed control characters.
/// Returns the sanitized string and whether any changes were made.
pub fn sanitize_string(input: &str) -> Result<String, String> {
    let mut result = String::with_capacity(input.len());
    let mut changed = false;

    for ch in input.chars() {
        // Allow printable characters and newline
        if ch == '\n' {
            result.push(ch);
            continue;
        }

        // Block 0x00-0x1F (except 0x0A already handled above)
        if ch.is_control() {
            changed = true;
            continue;
        }

        // Block Unicode format characters (Cf category)
        if is_format_char(ch) {
            changed = true;
            continue;
        }

        result.push(ch);
    }

    let _ = changed;
    Ok(result)
}

/// Check if a character is a Unicode format character (Cf category).
fn is_format_char(ch: char) -> bool {
    matches!(ch,
        '\u{00AD}' |       // SOFT HYPHEN
        '\u{0600}'..='\u{0605}' | // Arabic
        '\u{061C}' |       // ARABIC LETTER MARK
        '\u{06DD}' |       // ARABIC END OF AYAH
        '\u{070F}' |       // SYRIAC ABBREVIATION MARK
        '\u{0890}'..='\u{0891}' | // Arabic
        '\u{08E2}' |       // ARABIC DISPUTED END OF AYAH
        '\u{180E}' |       // MONGOLIAN VOWEL SEPARATOR
        '\u{200B}'..='\u{200F}' | // Zero-width, direction marks
        '\u{2028}'..='\u{202E}' | // Line/paragraph separators, direction
        '\u{2060}'..='\u{2064}' | // Word joiner, etc.
        '\u{2066}'..='\u{206F}' | // Direction isolates
        '\u{FEFF}' |       // BOM/ZWNBSP
        '\u{FFF9}'..='\u{FFFB}' | // Interlinear annotation
        '\u{13430}'..='\u{1343F}' // Egyptian hieroglyph format
    )
}

/// Validate a record body size.
pub fn validate_body_size(body: &str) -> Result<(), String> {
    if body.len() > MAX_RECORD_SIZE {
        Err(format!("record body exceeds 1 MB limit ({} bytes)", body.len()))
    } else {
        Ok(())
    }
}

/// Validate a tag key size.
pub fn validate_key_size(key: &str) -> Result<(), String> {
    if key.len() > MAX_KEY_SIZE {
        Err(format!("tag key exceeds 256 byte limit ({} bytes)", key.len()))
    } else {
        Ok(())
    }
}

/// Validate a tag value size.
pub fn validate_value_size(value: &str) -> Result<(), String> {
    if value.len() > MAX_VALUE_SIZE {
        Err(format!("tag value exceeds 64 KB limit ({} bytes)", value.len()))
    } else {
        Ok(())
    }
}

/// Full sanitization pipeline for a ContentRecord.
/// Returns the sanitized record or an error with the rejection reason.
pub fn sanitize_record(record: &crate::model::ContentRecord) -> Result<crate::model::ContentRecord, String> {
    // Sanitize body
    let body = sanitize_string(&record.body)?;
    validate_body_size(&body)?;

    // Sanitize source_url
    let source_url = sanitize_string(&record.source_url)?;

    // Sanitize schema
    let schema = sanitize_string(&record.schema)?;

    // Sanitize tags
    let mut tags = Vec::new();
    for tag in &record.tags {
        let sanitized_tag = sanitize_string(tag)?;
        if let Some((key, value)) = sanitized_tag.split_once(':') {
            validate_key_size(key)?;
            validate_value_size(value)?;
        }
        tags.push(sanitized_tag);
    }

    // Sanitize id and source_hash
    let id = sanitize_string(&record.id)?;
    let source_hash = sanitize_string(&record.source_hash)?;

    Ok(crate::model::ContentRecord {
        id,
        source_url,
        source_hash,
        schema,
        tags,
        body,
        created_at: record.created_at,
        expires_at: record.expires_at,
        scrape_source: record.scrape_source.clone(),
        refresh_policy: record.refresh_policy.clone(),
        sig: record.sig.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_allows_normal_text() {
        let result = sanitize_string("Hello world").unwrap();
        assert_eq!(result, "Hello world");
    }

    #[test]
    fn sanitize_allows_newline() {
        let result = sanitize_string("Hello\nworld").unwrap();
        assert_eq!(result, "Hello\nworld");
    }

    #[test]
    fn sanitize_strips_null_byte() {
        let input = "Hello\0world";
        let result = sanitize_string(input).unwrap();
        assert_eq!(result, "Helloworld");
    }

    #[test]
    fn sanitize_strips_carriage_return() {
        let input = "Hello\r\nworld";
        let result = sanitize_string(input).unwrap();
        assert_eq!(result, "Hello\nworld");
    }

    #[test]
    fn sanitize_strips_tab() {
        let input = "Hello\tworld";
        let result = sanitize_string(input).unwrap();
        assert_eq!(result, "Helloworld");
    }

    #[test]
    fn sanitize_strips_zero_width_space() {
        let input = "Hello\u{200B}world";
        let result = sanitize_string(input).unwrap();
        assert_eq!(result, "Helloworld");
    }

    #[test]
    fn sanitize_strips_bom() {
        let input = "\u{FEFF}Hello world";
        let result = sanitize_string(input).unwrap();
        assert_eq!(result, "Hello world");
    }

    #[test]
    fn sanitize_strips_direction_mark() {
        let input = "Hello\u{200F}world";
        let result = sanitize_string(input).unwrap();
        assert_eq!(result, "Helloworld");
    }

    #[test]
    fn validate_body_size_ok() {
        assert!(validate_body_size("small").is_ok());
    }

    #[test]
    fn validate_body_size_too_large() {
        let big = "x".repeat(MAX_RECORD_SIZE + 1);
        assert!(validate_body_size(&big).is_err());
    }

    #[test]
    fn validate_key_size_ok() {
        assert!(validate_key_size("category").is_ok());
    }

    #[test]
    fn validate_key_size_too_large() {
        let big = "x".repeat(MAX_KEY_SIZE + 1);
        assert!(validate_key_size(&big).is_err());
    }

    #[test]
    fn validate_value_size_ok() {
        assert!(validate_value_size("networking").is_ok());
    }

    #[test]
    fn validate_value_size_too_large() {
        let big = "x".repeat(MAX_VALUE_SIZE + 1);
        assert!(validate_value_size(&big).is_err());
    }

    #[test]
    fn sanitize_record_full() {
        let record = crate::model::ContentRecord {
            id: "test-id".to_string(),
            source_url: "https://example.com".to_string(),
            source_hash: "hash123".to_string(),
            schema: "wiki/article".to_string(),
            tags: vec!["category:networking".to_string()],
            body: "Hello\u{200B} world\0".to_string(),
            created_at: 1000,
            expires_at: 2000,
            scrape_source: crate::model::ScrapeSource::Url,
            refresh_policy: crate::model::RefreshPolicy::Once,
            sig: "".to_string(),
        };
        let sanitized = sanitize_record(&record).unwrap();
        assert_eq!(sanitized.body, "Hello world");
    }
}
