/// Minimal HTTP/1.1 response builder — hand-rolled per roadmap philosophy.
use std::collections::HashMap;

#[derive(Debug)]
pub struct HttpResponse {
    pub status: u16,
    pub status_text: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn ok(body: impl Into<Vec<u8>>) -> Self {
        Self::new(200, "OK", body)
    }

    pub fn json(body: &str) -> Self {
        let mut resp = Self::ok(body);
        resp.headers.insert(
            "content-type".into(),
            "application/json; charset=utf-8".into(),
        );
        resp
    }

    pub fn not_found(error: &str) -> Self {
        let body = serde_json::json!({"error": error, "code": 404}).to_string();
        let mut resp = Self::new(404, "Not Found", body);
        resp.headers.insert(
            "content-type".into(),
            "application/json; charset=utf-8".into(),
        );
        resp
    }

    pub fn bad_request(error: &str) -> Self {
        let body = serde_json::json!({"error": error, "code": 400}).to_string();
        let mut resp = Self::new(400, "Bad Request", body);
        resp.headers.insert(
            "content-type".into(),
            "application/json; charset=utf-8".into(),
        );
        resp
    }

    pub fn internal_error(error: &str) -> Self {
        let body = serde_json::json!({"error": error, "code": 500}).to_string();
        let mut resp = Self::new(500, "Internal Server Error", body);
        resp.headers.insert(
            "content-type".into(),
            "application/json; charset=utf-8".into(),
        );
        resp
    }

    pub fn method_not_allowed(error: &str) -> Self {
        let body = serde_json::json!({"error": error, "code": 405}).to_string();
        let mut resp = Self::new(405, "Method Not Allowed", body);
        resp.headers.insert(
            "content-type".into(),
            "application/json; charset=utf-8".into(),
        );
        resp
    }

    pub fn new(status: u16, status_text: &str, body: impl Into<Vec<u8>>) -> Self {
        let mut headers = HashMap::new();
        headers.insert("connection".into(), "close".into());
        headers.insert("server".into(), "dsearch".into());
        Self {
            status,
            status_text: status_text.into(),
            headers,
            body: body.into(),
        }
    }

    /// Add standard response headers per roadmap spec.
    pub fn with_node_headers(mut self, node_id: &str) -> Self {
        self.headers.insert("x-node-id".into(), node_id.into());
        self.headers.insert("x-protocol-version".into(), "1".into());
        self
    }

    /// Add X-Record-Count header.
    pub fn with_record_count(mut self, count: usize) -> Self {
        self.headers
            .insert("x-record-count".into(), count.to_string());
        self
    }

    /// Serialize to bytes for writing to a TCP stream.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();

        // Status line
        out.extend_from_slice(
            format!("HTTP/1.1 {} {}\r\n", self.status, self.status_text).as_bytes(),
        );

        // Content-Length
        out.extend_from_slice(format!("content-length: {}\r\n", self.body.len()).as_bytes());

        // Headers
        for (key, value) in &self.headers {
            if key.to_lowercase() != "content-length" {
                out.extend_from_slice(format!("{}: {}\r\n", key, value).as_bytes());
            }
        }

        // Blank line
        out.extend_from_slice(b"\r\n");

        // Body
        out.extend_from_slice(&self.body);

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_response_ok() {
        let resp = HttpResponse::ok(b"hello".to_vec());
        assert_eq!(resp.status, 200);
        assert_eq!(resp.status_text, "OK");
    }

    #[test]
    fn http_response_json() {
        let resp = HttpResponse::json(r#"{"key":"value"}"#);
        assert_eq!(resp.status, 200);
        assert_eq!(resp.headers.get("content-type").unwrap(), "application/json; charset=utf-8");
    }

    #[test]
    fn http_response_not_found() {
        let resp = HttpResponse::not_found("test error");
        assert_eq!(resp.status, 404);
        let body = String::from_utf8_lossy(&resp.body);
        assert!(body.contains("test error"));
    }

    #[test]
    fn http_response_bad_request() {
        let resp = HttpResponse::bad_request("bad input");
        assert_eq!(resp.status, 400);
        let body = String::from_utf8_lossy(&resp.body);
        assert!(body.contains("bad input"));
    }

    #[test]
    fn http_response_internal_error() {
        let resp = HttpResponse::internal_error("oops");
        assert_eq!(resp.status, 500);
        let body = String::from_utf8_lossy(&resp.body);
        assert!(body.contains("oops"));
    }

    #[test]
    fn http_response_method_not_allowed() {
        let resp = HttpResponse::method_not_allowed("only GET");
        assert_eq!(resp.status, 405);
        let body = String::from_utf8_lossy(&resp.body);
        assert!(body.contains("only GET"));
    }

    #[test]
    fn http_response_with_node_headers() {
        let resp = HttpResponse::ok(b"{}".to_vec()).with_node_headers("node-abc");
        assert_eq!(resp.headers.get("x-node-id").unwrap(), "node-abc");
        assert_eq!(resp.headers.get("x-protocol-version").unwrap(), "1");
    }

    #[test]
    fn http_response_with_record_count() {
        let resp = HttpResponse::ok(b"{}".to_vec()).with_record_count(42);
        assert_eq!(resp.headers.get("x-record-count").unwrap(), "42");
    }

    #[test]
    fn http_response_to_bytes_format() {
        let resp = HttpResponse::ok(b"hello".to_vec());
        let bytes = resp.to_bytes();
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(text.contains("content-length: 5"));
        assert!(text.contains("connection: close"));
        assert!(text.contains("server: dsearch"));
        assert!(text.ends_with("hello"));
    }

    #[test]
    fn http_response_to_bytes_no_duplicate_content_length() {
        let mut resp = HttpResponse::ok(b"test".to_vec());
        resp.headers.insert("content-length".into(), "999".into());
        let bytes = resp.to_bytes();
        let text = String::from_utf8_lossy(&bytes);
        // Should only have one content-length (the computed one)
        let count = text.matches("content-length").count();
        assert_eq!(count, 1);
    }
}
