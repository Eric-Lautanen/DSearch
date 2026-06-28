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
