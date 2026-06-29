/// Minimal HTTP/1.1 request parser — hand-rolled per roadmap philosophy.
use std::collections::HashMap;

#[derive(Debug)]
pub struct HttpRequest {
    pub method: String,
    pub path: String,
    pub query: HashMap<String, String>,
    pub headers: HashMap<String, String>,
    pub body: String,
}

/// Maximum number of headers allowed in a single request.
const MAX_HEADERS: usize = 64;
/// Maximum length of a single header value (bytes).
const MAX_HEADER_VALUE_LEN: usize = 8192;
/// Maximum body size (bytes) — applies to both local and gateway APIs.
const MAX_BODY_SIZE: usize = 1_048_576; // 1 MB

/// Validate the parsed path for path-traversal attacks.
/// Rejects paths containing `..` segments or embedded null bytes.
fn validate_path(path: &str) -> Result<(), String> {
    if path.contains('\0') {
        return Err("path contains null byte".into());
    }
    // Reject any path segment that is exactly ".."
    for segment in path.split('/') {
        if segment == ".." {
            return Err("path traversal detected: '..' segment not allowed".into());
        }
    }
    Ok(())
}

/// Parse a raw HTTP/1.1 request string into an HttpRequest.
/// Validates path (no traversal), limits header count/size, and caps body size.
pub fn parse_http_request(raw: &str) -> Result<HttpRequest, String> {
    let mut lines = raw.lines();
    let request_line = lines.next().ok_or("empty request")?;

    let parts: Vec<&str> = request_line.splitn(3, ' ').collect();
    if parts.len() < 2 {
        return Err("malformed request line".into());
    }

    let method = parts[0].to_string();
    let full_uri = parts[1];

    // Split path and query string
    let (path, query_string) = match full_uri.find('?') {
        Some(pos) => (&full_uri[..pos], Some(&full_uri[pos + 1..])),
        None => (full_uri, None),
    };

    // Percent-decode the path for validation (attackers may encode traversal/null bytes)
    let decoded_path = percent_decode(path);
    validate_path(&decoded_path)?;

    let query = parse_query_string(query_string.unwrap_or(""));

    // Parse headers with limits
    let mut headers = HashMap::new();
    for line in lines.by_ref() {
        if line.is_empty() {
            break;
        }
        if headers.len() >= MAX_HEADERS {
            return Err(format!("too many headers (max {})", MAX_HEADERS));
        }
        if let Some(colon_pos) = line.find(':') {
            let key = line[..colon_pos].trim().to_lowercase();
            let value = line[colon_pos + 1..].trim().to_string();
            if value.len() > MAX_HEADER_VALUE_LEN {
                return Err(format!(
                    "header value too long (max {} bytes): {}",
                    MAX_HEADER_VALUE_LEN, key
                ));
            }
            headers.insert(key, value);
        }
    }

    // Remaining content is body (for POST)
    let body = raw.split("\r\n\r\n").nth(1).unwrap_or("").to_string();

    // Cap body size
    if body.len() > MAX_BODY_SIZE {
        return Err(format!(
            "body too large (max {} bytes, got {})",
            MAX_BODY_SIZE,
            body.len()
        ));
    }

    // If Content-Length is present, validate it against the body
    if let Some(cl) = headers.get("content-length") {
        let declared: usize = cl.parse().map_err(|_| "invalid Content-Length")?;
        if declared > MAX_BODY_SIZE {
            return Err(format!(
                "Content-Length {} exceeds max body size {}",
                declared, MAX_BODY_SIZE
            ));
        }
    }

    Ok(HttpRequest {
        method,
        path: path.to_string(),
        query,
        headers,
        body,
    })
}

/// Parse a URL query string into a HashMap.
fn parse_query_string(qs: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    if qs.is_empty() {
        return map;
    }
    for pair in qs.split('&') {
        if pair.is_empty() {
            continue;
        }
        match pair.find('=') {
            Some(pos) => {
                let key = percent_decode(&pair[..pos]);
                let value = percent_decode(&pair[pos + 1..]);
                map.insert(key, value);
            }
            None => {
                map.insert(percent_decode(pair), String::new());
            }
        }
    }
    map
}

/// Minimal percent-decode — handles %20, %2F, etc.
fn percent_decode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            let hex: String = chars.by_ref().take(2).collect();
            if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                result.push(byte as char);
            } else {
                result.push('%');
                result.push_str(&hex);
            }
        } else if c == '+' {
            result.push(' ');
        } else {
            result.push(c);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_get_request() {
        let raw = "GET /search?q=rust&limit=10 HTTP/1.1\r\nHost: 127.0.0.1:7743\r\nAccept: application/json\r\n\r\n";
        let req = parse_http_request(raw).unwrap();
        assert_eq!(req.method, "GET");
        assert_eq!(req.path, "/search");
        assert_eq!(req.query.get("q").unwrap(), "rust");
        assert_eq!(req.query.get("limit").unwrap(), "10");
    }

    #[test]
    fn parse_post_request() {
        let raw = "POST /record HTTP/1.1\r\nHost: 127.0.0.1:7743\r\nContent-Type: application/json\r\nContent-Length: 13\r\n\r\n{\"id\":\"abc\"}";
        let req = parse_http_request(raw).unwrap();
        assert_eq!(req.method, "POST");
        assert_eq!(req.path, "/record");
        assert!(req.body.contains("abc"));
    }

    #[test]
    fn parse_path_without_query() {
        let raw = "GET /health HTTP/1.1\r\n\r\n";
        let req = parse_http_request(raw).unwrap();
        assert_eq!(req.path, "/health");
        assert!(req.query.is_empty());
    }

    #[test]
    fn percent_decode_spaces() {
        let raw = "GET /search?q=rust%20async HTTP/1.1\r\n\r\n";
        let req = parse_http_request(raw).unwrap();
        assert_eq!(req.query.get("q").unwrap(), "rust async");
    }

    #[test]
    fn plus_decode_spaces() {
        let raw = "GET /search?q=rust+async HTTP/1.1\r\n\r\n";
        let req = parse_http_request(raw).unwrap();
        assert_eq!(req.query.get("q").unwrap(), "rust async");
    }

    #[test]
    fn reject_path_traversal() {
        let raw = "GET /record/../../identity.key HTTP/1.1\r\n\r\n";
        assert!(parse_http_request(raw).is_err());
    }

    #[test]
    fn reject_null_byte_in_path() {
        let raw = "GET /record/abc%00def HTTP/1.1\r\n\r\n";
        assert!(parse_http_request(raw).is_err());
    }

    #[test]
    fn reject_too_many_headers() {
        let mut raw = "GET /health HTTP/1.1\r\n".to_string();
        for i in 0..65 {
            raw.push_str(&format!("x-hdr-{}: val\r\n", i));
        }
        raw.push_str("\r\n");
        assert!(parse_http_request(&raw).is_err());
    }

    #[test]
    fn reject_oversized_header_value() {
        let long_val = "x".repeat(8193);
        let raw = format!("GET /health HTTP/1.1\r\nx-big: {}\r\n\r\n", long_val);
        assert!(parse_http_request(&raw).is_err());
    }

    #[test]
    fn reject_oversized_body() {
        let big_body = "x".repeat(1_048_577);
        let raw = format!(
            "POST /record HTTP/1.1\r\nContent-Length: {}\r\n\r\n{}",
            big_body.len(),
            big_body
        );
        assert!(parse_http_request(&raw).is_err());
    }

    #[test]
    fn reject_oversized_content_length() {
        let raw = format!(
            "POST /record HTTP/1.1\r\nContent-Length: {}\r\n\r\n{{}}",
            2_000_000
        );
        assert!(parse_http_request(&raw).is_err());
    }

    #[test]
    fn normal_path_with_dots_allowed() {
        let raw = "GET /record/abc.def HTTP/1.1\r\n\r\n";
        let req = parse_http_request(raw).unwrap();
        assert_eq!(req.path, "/record/abc.def");
    }

    #[test]
    fn reject_encoded_path_traversal() {
        let raw = "GET /record/%2e%2e/identity.key HTTP/1.1\r\n\r\n";
        assert!(parse_http_request(raw).is_err());
    }
}
