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

/// Parse a raw HTTP/1.1 request string into an HttpRequest.
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

    let query = parse_query_string(query_string.unwrap_or(""));

    // Parse headers
    let mut headers = HashMap::new();
    for line in lines.by_ref() {
        if line.is_empty() {
            break;
        }
        if let Some(colon_pos) = line.find(':') {
            let key = line[..colon_pos].trim().to_lowercase();
            let value = line[colon_pos + 1..].trim().to_string();
            headers.insert(key, value);
        }
    }

    // Remaining content is body (for POST)
    let body = raw.split("\r\n\r\n").nth(1).unwrap_or("").to_string();

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
}
