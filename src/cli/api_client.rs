/// Simple HTTP client for CLI → API communication.
/// Hand-rolled per roadmap philosophy — no extra deps.
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;

/// Read the API port from {data_dir}/api.port.
pub fn read_api_port(data_dir: &Path) -> Option<u16> {
    let port_path = data_dir.join("api.port");
    let contents = std::fs::read_to_string(port_path).ok()?;
    contents.trim().parse().ok()
}

/// Make a GET request to the local API using the data_dir to find the port.
/// Returns None if the API is not reachable.
pub fn api_get_from_dir(data_dir: &Path, path: &str) -> Option<String> {
    let port = read_api_port(data_dir)?;
    api_get(port, path).ok()
}

/// Check if the local API is reachable.
pub fn api_is_reachable(data_dir: &Path) -> Option<u16> {
    let port = read_api_port(data_dir)?;
    let addr = format!("127.0.0.1:{}", port);
    if TcpStream::connect_timeout(&addr.parse().ok()?, std::time::Duration::from_secs(2)).is_ok() {
        Some(port)
    } else {
        None
    }
}

/// Make a GET request to the local API and return the response body.
pub fn api_get(port: u16, path: &str) -> Result<String, String> {
    let addr = format!("127.0.0.1:{}", port);
    let mut stream = TcpStream::connect_timeout(
        &addr.parse().map_err(|e| format!("invalid addr: {}", e))?,
        std::time::Duration::from_secs(5),
    )
    .map_err(|e| format!("connect to API: {}", e))?;
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(10)))
        .ok();

    let request = format!("GET {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAccept: application/json\r\nConnection: close\r\n\r\n", path, port);
    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("write: {}", e))?;
    stream.flush().map_err(|e| format!("flush: {}", e))?;

    let mut response = Vec::new();
    let mut buf = [0u8; 65536];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => response.extend_from_slice(&buf[..n]),
            Err(_) => break,
        }
    }

    let response_str = String::from_utf8_lossy(&response);
    // Split headers from body
    if let Some(pos) = response_str.find("\r\n\r\n") {
        let headers = &response_str[..pos];
        let body = &response_str[pos + 4..];
        // Check status code
        let status_line = headers.lines().next().unwrap_or("");
        if status_line.contains("200") {
            Ok(body.to_string())
        } else if status_line.contains("404") {
            Err(format!("not found: {}", body))
        } else if status_line.contains("400") {
            Err(format!("bad request: {}", body))
        } else {
            Err(format!("API error: {}", status_line))
        }
    } else {
        Err("malformed HTTP response".into())
    }
}

/// Make a POST request to the local API with a JSON body.
pub fn api_post(port: u16, path: &str, body: &str) -> Result<String, String> {
    let addr = format!("127.0.0.1:{}", port);
    let mut stream = TcpStream::connect_timeout(
        &addr.parse().map_err(|e| format!("invalid addr: {}", e))?,
        std::time::Duration::from_secs(5),
    )
    .map_err(|e| format!("connect to API: {}", e))?;
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(10)))
        .ok();

    let request = format!(
        "POST {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nAccept: application/json\r\nConnection: close\r\n\r\n{}",
        path, port, body.len(), body
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("write: {}", e))?;
    stream.flush().map_err(|e| format!("flush: {}", e))?;

    let mut response = Vec::new();
    let mut buf = [0u8; 65536];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => response.extend_from_slice(&buf[..n]),
            Err(_) => break,
        }
    }

    let response_str = String::from_utf8_lossy(&response);
    if let Some(pos) = response_str.find("\r\n\r\n") {
        let headers = &response_str[..pos];
        let resp_body = &response_str[pos + 4..];
        let status_line = headers.lines().next().unwrap_or("");
        if status_line.contains("200") || status_line.contains("201") {
            Ok(resp_body.to_string())
        } else {
            Err(format!("API error: {} — {}", status_line, resp_body))
        }
    } else {
        Err("malformed HTTP response".into())
    }
}
