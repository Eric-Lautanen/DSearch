use std::collections::HashMap;

/// A parsed search query.
#[derive(Debug, Clone, Default)]
pub struct ParsedQuery {
    /// Free-text terms (AND logic by default).
    pub terms: Vec<String>,
    /// Exact phrase matches.
    pub phrases: Vec<String>,
    /// Excluded terms (NOT logic).
    pub excludes: Vec<String>,
    /// Field-scoped filters: field name → value.
    pub fields: HashMap<String, String>,
    /// Result limit override.
    pub limit: Option<usize>,
    /// `since:` filter as unix timestamp.
    pub since: Option<u64>,
    /// `before:` filter as unix timestamp.
    pub before: Option<u64>,
}

/// Parse a search query string into a structured query.
///
/// Supports:
/// - Free text: `rust async` → AND of terms
/// - Exact phrases: `"rust async"` → phrase match
/// - Negation: `-async` → exclude
/// - Field filters: `title:rust`, `tag:category:networking`, `schema:rust/crate`,
///   `source:crates.io`, `scraped:keyword`, `refresh:interval`
/// - Limit: `limit:20`
/// - Date filters: `since:2024-01-01`, `before:2025-01-01`
/// - OR: `rust OR async` → OR logic (stored as special term prefix)
pub fn parse_query(input: &str) -> ParsedQuery {
    let mut query = ParsedQuery::default();
    let tokens = tokenize(input);
    let mut i = 0;

    while i < tokens.len() {
        let token = &tokens[i];

        if token == "OR" && !query.terms.is_empty() {
            // Mark the previous term and next term as OR-linked
            if let Some(last) = query.terms.last_mut() {
                last.insert_str(0, "OR:");
            }
            i += 1;
            if i < tokens.len() {
                let mut next = tokens[i].clone();
                next.insert_str(0, "OR:");
                query.terms.push(next);
            }
        } else if token.starts_with('-') && token.len() > 1 {
            query.excludes.push(token[1..].to_lowercase());
        } else if token.starts_with('"') && token.ends_with('"') && token.len() > 1 {
            query.phrases.push(token[1..token.len() - 1].to_lowercase());
        } else if let Some((field, value)) = parse_field_filter(token) {
            match field.as_str() {
                "limit" => {
                    if let Ok(n) = value.parse::<usize>() {
                        query.limit = Some(n);
                    }
                }
                "since" => {
                    query.since = parse_date_value(&value);
                }
                "before" => {
                    query.before = parse_date_value(&value);
                }
                _ => {
                    query.fields.insert(field, value.to_lowercase());
                }
            }
        } else {
            query.terms.push(token.to_lowercase());
        }

        i += 1;
    }

    query
}

/// Tokenize the input, respecting quoted phrases.
fn tokenize(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;

    for ch in input.chars() {
        if ch == '"' {
            if in_quotes {
                current.push(ch);
                in_quotes = false;
            } else {
                // Flush current token
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
                current.push(ch);
                in_quotes = true;
            }
        } else if ch.is_whitespace() && !in_quotes {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
        } else {
            current.push(ch);
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

/// Parse a field filter like `title:rust` or `tag:category:networking`.
/// Returns (field, value) where field is the part before the first colon
/// and value is everything after. For `tag:category:networking`,
/// field="tag", value="category:networking".
fn parse_field_filter(token: &str) -> Option<(String, String)> {
    let colon_pos = token.find(':')?;
    let field = &token[..colon_pos];
    let value = &token[colon_pos + 1..];

    match field {
        "title" | "tag" | "schema" | "source" | "scraped" | "refresh" | "limit" | "since"
        | "before" => Some((field.to_string(), value.to_string())),
        _ => None,
    }
}

/// Parse a date value. Accepts `YYYY-MM-DD` or a unix timestamp.
fn parse_date_value(value: &str) -> Option<u64> {
    // Try unix timestamp first
    if let Ok(ts) = value.parse::<u64>() {
        return Some(ts);
    }

    // Try YYYY-MM-DD
    let parts: Vec<&str> = value.split('-').collect();
    if parts.len() == 3 {
        if let (Ok(y), Ok(m), Ok(d)) = (
            parts[0].parse::<i32>(),
            parts[1].parse::<u32>(),
            parts[2].parse::<u32>(),
        ) {
            // Validate month range
            if !(1..=12).contains(&m) {
                return None;
            }
            // Validate day range (simplified — doesn't check per-month max)
            if !(1..=31).contains(&d) {
                return None;
            }
            // Use proper calendar arithmetic via days_from_civil
            let days = days_from_civil(y, m as i32, d as i32);
            let epoch_days = days_from_civil(1970, 1, 1);
            let diff_days = days - epoch_days;
            if diff_days < 0 {
                return None;
            }
            return Some((diff_days as u64) * 86400);
        }
    }

    None
}

/// Convert a civil date (year, month, day) to a day count from an epoch.
/// Uses Howard Hinnant's algorithm — correct for all Gregorian dates.
fn days_from_civil(y: i32, m: i32, d: i32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y / 400 } else { (y - 399) / 400 };
    let yoe = (y as i64) - (era as i64) * 400;
    let doy = (153i64 * (m as i64 + if m > 2 { -3 } else { 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era as i64 * 146097 + doe - 719468
}

/// Check if a record matches a parsed query.
pub fn matches_query(record: &crate::model::ContentRecord, query: &ParsedQuery) -> bool {
    // Check field filters
    for (field, value) in &query.fields {
        match field.as_str() {
            "schema" => {
                if !record.schema.to_lowercase().contains(value) {
                    return false;
                }
            }
            "source" => {
                // Filter by source domain — extract host from source_url
                if let Some(host) = extract_host(&record.source_url) {
                    if !host.to_lowercase().contains(value) {
                        return false;
                    }
                } else {
                    return false;
                }
            }
            "scraped" => {
                if record.scrape_source.as_str() != value {
                    return false;
                }
            }
            "refresh" => {
                if record.refresh_policy.as_str() != value {
                    return false;
                }
            }
            "tag" => {
                if !record.tags.iter().any(|t| t.to_lowercase() == *value) {
                    return false;
                }
            }
            "title" => {
                // "title" maps to the record id, source_url, and first line of body
                let first_line = record.body.lines().next().unwrap_or("");
                let searchable =
                    format!("{} {} {}", record.id, record.source_url, first_line).to_lowercase();
                if !searchable.contains(value) {
                    return false;
                }
            }
            _ => {}
        }
    }

    // Check date filters
    if let Some(since) = query.since {
        if record.created_at < since {
            return false;
        }
    }
    if let Some(before) = query.before {
        if record.created_at >= before {
            return false;
        }
    }

    // Build searchable text from record fields
    let searchable = build_searchable_text(record);

    // Check excludes
    for exc in &query.excludes {
        if searchable.contains(exc) {
            return false;
        }
    }

    // Check phrases
    for phrase in &query.phrases {
        if !searchable.contains(phrase) {
            return false;
        }
    }

    // Check terms (AND logic, with OR support)
    if !query.terms.is_empty() {
        let mut or_groups: Vec<Vec<String>> = Vec::new();
        let mut current_group: Vec<String> = Vec::new();

        for term in &query.terms {
            if let Some(stripped) = term.strip_prefix("OR:") {
                current_group.push(stripped.to_string());
            } else {
                if !current_group.is_empty() {
                    or_groups.push(std::mem::take(&mut current_group));
                }
                current_group.push(term.clone());
            }
        }
        if !current_group.is_empty() {
            or_groups.push(current_group);
        }

        for group in &or_groups {
            let any_match = group.iter().any(|t| searchable.contains(t));
            if !any_match {
                return false;
            }
        }
    }

    true
}

/// Build a lowercase searchable text blob from a record.
fn build_searchable_text(record: &crate::model::ContentRecord) -> String {
    let mut text = String::new();
    text.push_str(&record.id);
    text.push(' ');
    text.push_str(&record.source_url);
    text.push(' ');
    text.push_str(&record.schema);
    text.push(' ');
    for tag in &record.tags {
        text.push_str(tag);
        text.push(' ');
    }
    text.push_str(&record.body);
    text.to_lowercase()
}

/// Extract the host from a URL string without using the `url` crate.
fn extract_host(url: &str) -> Option<String> {
    // Strip scheme
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    // Find end of host (first / or : or end of string)
    let end = rest.find(['/', ':']).unwrap_or(rest.len());
    Some(rest[..end].to_string())
}

/// Score a record for ranking. Higher is better.
pub fn score_record(
    record: &crate::model::ContentRecord,
    query: &ParsedQuery,
    holder_count: u32,
) -> f64 {
    let mut score = 0.0;

    let id_text = record.id.to_lowercase();
    let tag_text = record.tags.join(" ").to_lowercase();
    let body_text = record.body.to_lowercase();
    let schema_text = record.schema.to_lowercase();

    // 1. Field match weight: title (id) > tag > body
    for term in &query.terms {
        let t = term.strip_prefix("OR:").unwrap_or(term);
        if id_text.contains(t) {
            score += 3.0;
        }
        if tag_text.contains(t) {
            score += 2.0;
        }
        if body_text.contains(t) {
            score += 1.0;
        }
        if schema_text.contains(t) {
            score += 1.5;
        }
    }

    // 2. Exact phrase bonus
    for phrase in &query.phrases {
        if body_text.contains(phrase) {
            score += 5.0;
        }
        if id_text.contains(phrase) {
            score += 7.0;
        }
    }

    // 3. Record freshness (recency boost)
    // Max boost for records created in the last day, decaying over 365 days
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let age_secs = now.saturating_sub(record.created_at);
    let age_days = age_secs as f64 / 86400.0;
    let freshness = 1.0 / (1.0 + age_days / 30.0);
    score += freshness * 2.0;

    // 4. Holder count boost (more holders = more stable)
    score += (holder_count as f64).ln().max(0.0) * 0.5;

    score
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ContentRecord, RefreshPolicy, ScrapeSource};

    fn make_record(
        id: &str,
        source_url: &str,
        schema: &str,
        tags: &[&str],
        body: &str,
        created_at: u64,
    ) -> ContentRecord {
        ContentRecord {
            id: id.to_string(),
            source_url: source_url.to_string(),
            source_hash: format!("hash_{}", id),
            schema: schema.to_string(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            body: body.to_string(),
            created_at,
            expires_at: 9999999999,
            scrape_source: ScrapeSource::Url,
            refresh_policy: RefreshPolicy::Once,
            sig: "".to_string(),
        }
    }

    #[test]
    fn parse_simple_terms() {
        let q = parse_query("rust async");
        assert_eq!(q.terms, vec!["rust", "async"]);
        assert!(q.phrases.is_empty());
        assert!(q.excludes.is_empty());
    }

    #[test]
    fn parse_phrase() {
        let q = parse_query("\"rust async\"");
        assert!(q.terms.is_empty());
        assert_eq!(q.phrases, vec!["rust async"]);
    }

    #[test]
    fn parse_exclude() {
        let q = parse_query("rust -async");
        assert_eq!(q.terms, vec!["rust"]);
        assert_eq!(q.excludes, vec!["async"]);
    }

    #[test]
    fn parse_or() {
        let q = parse_query("rust OR async");
        assert_eq!(q.terms.len(), 2);
        assert!(q.terms[0].starts_with("OR:"));
        assert!(q.terms[1].starts_with("OR:"));
    }

    #[test]
    fn parse_field_filters() {
        let q = parse_query("schema:rust/crate tag:category:networking");
        assert_eq!(q.fields.get("schema").unwrap(), "rust/crate");
        assert_eq!(q.fields.get("tag").unwrap(), "category:networking");
    }

    #[test]
    fn parse_limit() {
        let q = parse_query("rust limit:20");
        assert_eq!(q.terms, vec!["rust"]);
        assert_eq!(q.limit, Some(20));
    }

    #[test]
    fn parse_since_before() {
        let q = parse_query("rust since:2024-01-01 before:2025-01-01");
        assert!(q.since.is_some());
        assert!(q.before.is_some());
    }

    #[test]
    fn parse_scraped_refresh() {
        let q = parse_query("scraped:keyword refresh:interval");
        assert_eq!(q.fields.get("scraped").unwrap(), "keyword");
        assert_eq!(q.fields.get("refresh").unwrap(), "interval");
    }

    #[test]
    fn match_simple_terms() {
        let record = make_record(
            "r1",
            "https://example.com",
            "wiki/article",
            &["category:networking"],
            "Rust async runtime benchmarks",
            1700000000,
        );
        let q = parse_query("rust async");
        assert!(matches_query(&record, &q));
    }

    #[test]
    fn match_phrase() {
        let record = make_record(
            "r1",
            "https://example.com",
            "wiki/article",
            &[],
            "Rust async runtime benchmarks",
            1700000000,
        );
        let q = parse_query("\"rust async\"");
        assert!(matches_query(&record, &q));
    }

    #[test]
    fn match_exclude() {
        let record = make_record(
            "r1",
            "https://example.com",
            "wiki/article",
            &[],
            "Rust async runtime benchmarks",
            1700000000,
        );
        let q = parse_query("rust -async");
        assert!(!matches_query(&record, &q));
    }

    #[test]
    fn match_or() {
        let record = make_record(
            "r1",
            "https://example.com",
            "wiki/article",
            &[],
            "Only rust here",
            1700000000,
        );
        let q = parse_query("rust OR python");
        assert!(matches_query(&record, &q));
    }

    #[test]
    fn match_schema_filter() {
        let record = make_record(
            "r1",
            "https://example.com",
            "rust/crate",
            &[],
            "Some content",
            1700000000,
        );
        let q = parse_query("schema:rust/crate");
        assert!(matches_query(&record, &q));

        let q2 = parse_query("schema:wiki/article");
        assert!(!matches_query(&record, &q2));
    }

    #[test]
    fn match_tag_filter() {
        let record = make_record(
            "r1",
            "https://example.com",
            "wiki/article",
            &["category:networking"],
            "Some content",
            1700000000,
        );
        let q = parse_query("tag:category:networking");
        assert!(matches_query(&record, &q));

        let q2 = parse_query("tag:category:science");
        assert!(!matches_query(&record, &q2));
    }

    #[test]
    fn match_since_filter() {
        let record = make_record(
            "r1",
            "https://example.com",
            "wiki/article",
            &[],
            "Some content",
            1700000000,
        );
        let q = parse_query("since:1700000000");
        assert!(matches_query(&record, &q));

        let q2 = parse_query("since:1700000001");
        assert!(!matches_query(&record, &q2));
    }

    #[test]
    fn match_before_filter() {
        let record = make_record(
            "r1",
            "https://example.com",
            "wiki/article",
            &[],
            "Some content",
            1700000000,
        );
        let q = parse_query("before:1700000001");
        assert!(matches_query(&record, &q));

        let q2 = parse_query("before:1700000000");
        assert!(!matches_query(&record, &q2));
    }

    #[test]
    fn match_scraped_filter() {
        let mut record = make_record(
            "r1",
            "https://example.com",
            "wiki/article",
            &[],
            "Some content",
            1700000000,
        );
        record.scrape_source = ScrapeSource::Keyword;
        let q = parse_query("scraped:keyword");
        assert!(matches_query(&record, &q));

        let q2 = parse_query("scraped:url");
        assert!(!matches_query(&record, &q2));
    }

    #[test]
    fn match_refresh_filter() {
        let mut record = make_record(
            "r1",
            "https://example.com",
            "wiki/article",
            &[],
            "Some content",
            1700000000,
        );
        record.refresh_policy = RefreshPolicy::Interval;
        let q = parse_query("refresh:interval");
        assert!(matches_query(&record, &q));

        let q2 = parse_query("refresh:once");
        assert!(!matches_query(&record, &q2));
    }

    #[test]
    fn match_source_domain_filter() {
        let record = make_record(
            "r1",
            "https://crates.io/crates/tokio",
            "rust/crate",
            &[],
            "Tokio runtime",
            1700000000,
        );
        let q = parse_query("source:crates.io");
        assert!(matches_query(&record, &q));

        let q2 = parse_query("source:github.com");
        assert!(!matches_query(&record, &q2));
    }

    #[test]
    fn combined_query() {
        let record = make_record(
            "r1",
            "https://crates.io/crates/tokio",
            "rust/crate",
            &["category:networking"],
            "Tokio async runtime",
            1700000000,
        );
        let q = parse_query("tokio schema:rust/crate");
        assert!(matches_query(&record, &q));
    }

    #[test]
    fn score_ranking() {
        let r1 = make_record(
            "rust-guide",
            "https://example.com",
            "wiki/article",
            &["rust"],
            "A guide to rust programming",
            1700000000,
        );
        let r2 = make_record(
            "other",
            "https://example.com",
            "wiki/article",
            &[],
            "Unrelated content",
            1700000000,
        );

        let q = parse_query("rust");
        let s1 = score_record(&r1, &q, 1);
        let s2 = score_record(&r2, &q, 1);
        assert!(s1 > s2, "record with 'rust' in id should score higher");
    }

    #[test]
    fn score_freshness() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let recent = make_record(
            "r1",
            "https://example.com",
            "wiki/article",
            &[],
            "rust content",
            now - 100,
        );
        let old = make_record(
            "r2",
            "https://example.com",
            "wiki/article",
            &[],
            "rust content",
            now - 86400 * 365,
        );

        let q = parse_query("rust");
        let s_recent = score_record(&recent, &q, 1);
        let s_old = score_record(&old, &q, 1);
        assert!(s_recent > s_old, "recent record should score higher");
    }

    #[test]
    fn score_holder_count() {
        let r = make_record(
            "r1",
            "https://example.com",
            "wiki/article",
            &[],
            "rust content",
            1700000000,
        );
        let q = parse_query("rust");
        let s1 = score_record(&r, &q, 1);
        let s5 = score_record(&r, &q, 5);
        assert!(s5 > s1, "more holders should score higher");
    }

    #[test]
    fn date_parsing_known_date() {
        // 2024-01-01 00:00:00 UTC = 1704067200
        let ts = parse_date_value("2024-01-01");
        assert_eq!(ts, Some(1704067200));
    }

    #[test]
    fn date_parsing_leap_year() {
        // 2024-02-29 (leap day) should be valid
        let ts = parse_date_value("2024-02-29");
        assert!(ts.is_some());
    }

    #[test]
    fn date_parsing_invalid_month() {
        let ts = parse_date_value("2024-13-01");
        assert!(ts.is_none());
    }

    #[test]
    fn date_parsing_invalid_month_zero() {
        let ts = parse_date_value("2024-00-15");
        assert!(ts.is_none());
    }

    #[test]
    fn date_parsing_invalid_day_zero() {
        let ts = parse_date_value("2024-01-00");
        assert!(ts.is_none());
    }

    #[test]
    fn date_parsing_unix_timestamp() {
        let ts = parse_date_value("1704067200");
        assert_eq!(ts, Some(1704067200));
    }
}
