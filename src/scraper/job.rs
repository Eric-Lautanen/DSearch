use crate::model::{schema, ContentRecord, RefreshPolicy, ScrapeSource};
use crate::sanitize::sanitize_record;
use crate::storage::Store;
use crate::trust::sign::{compute_record_id, compute_source_hash};

/// Run a single URL-source scrape job: fetch the URL, create a ContentRecord, store it.
pub async fn run_url_job(
    store: &Store,
    name: &str,
    target: &str,
    lifecycle: &str,
    ttl_secs: u64,
) -> Result<ScrapeResult, String> {
    let body = fetch_url(target).await?;

    let source_hash = compute_source_hash(target.as_bytes());

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let created_at_str = now.to_string();
    let id = compute_record_id(
        target.as_bytes(),
        source_hash.as_bytes(),
        schema::GENERIC_KV.as_bytes(),
        b"",
        body.as_bytes(),
        created_at_str.as_bytes(),
    );

    let expires_at = match lifecycle {
        "pinned" => 0,
        _ => now + ttl_secs,
    };

    let mut record = ContentRecord {
        id,
        source_url: target.to_string(),
        source_hash,
        schema: schema::GENERIC_KV.to_string(),
        tags: vec![format!("scraper:{}", name)],
        body,
        created_at: now,
        expires_at,
        scrape_source: ScrapeSource::Url,
        refresh_policy: RefreshPolicy::Once,
        sig: "".to_string(),
    };

    record = sanitize_record(&record)?;
    let result = store.insert_record(&mut record)?;

    Ok(ScrapeResult {
        job_name: name.to_string(),
        record_id: record.id.clone(),
        inserted: matches!(result, crate::storage::records::InsertResult::Inserted),
        replaced: matches!(result, crate::storage::records::InsertResult::ReplacedNewer),
    })
}

#[derive(Debug)]
pub struct ScrapeResult {
    pub job_name: String,
    pub record_id: String,
    pub inserted: bool,
    pub replaced: bool,
}

// ---------------------------------------------------------------------------
// HTTP(S) fetch — hand-rolled over std::net + rustls, no extra deps
// ---------------------------------------------------------------------------
// The roadmap requires plain url/feed/api sources use a hand-written HTTP/1.1
// client. For HTTPS we reuse the *existing* rustls crate (already pulled in
// as quinn's TLS backend) instead of adding tokio-rustls / rustls-native-certs
// / rustls-pki-types as separate deps.
//
// We use std::net::TcpStream (blocking) with rustls::ClientConnection because
// rustls's complete_io() is designed for blocking I/O. The fetch runs inside
// tokio::task::spawn_blocking so it doesn't block the async runtime.
//
// PEM cert parsing is hand-rolled (base64 between delimiters — plain format
// logic, not a crypto primitive). base64 decoding is also hand-rolled.
//
// DO NOT replace this with reqwest or tokio-rustls. Those are reserved for
// the keyword-discovery path (scraper/discovery/) which needs cookie jars,
// redirect handling, and UA rotation that a hand-rolled client makes painful.
// ---------------------------------------------------------------------------

/// Fetch a URL. Uses blocking I/O inside spawn_blocking.
async fn fetch_url(url: &str) -> Result<String, String> {
    let url = url.to_string();
    tokio::task::spawn_blocking(move || fetch_url_blocking(&url))
        .await
        .map_err(|e| format!("fetch task: {}", e))?
}

fn fetch_url_blocking(url: &str) -> Result<String, String> {
    let (host, path, port, is_https) = parse_url(url)?;

    let body_bytes = if is_https {
        fetch_https_blocking(&host, port, &path)?
    } else {
        fetch_http_blocking(&host, port, &path)?
    };

    let response =
        String::from_utf8(body_bytes).map_err(|e| format!("response not valid UTF-8: {}", e))?;
    if let Some(idx) = response.find("\r\n\r\n") {
        Ok(response[idx + 4..].to_string())
    } else if let Some(idx) = response.find("\n\n") {
        Ok(response[idx + 2..].to_string())
    } else {
        Ok(response)
    }
}

fn parse_url(url: &str) -> Result<(String, String, u16, bool), String> {
    let (scheme, rest) = if let Some(r) = url.strip_prefix("https://") {
        ("https", r)
    } else if let Some(r) = url.strip_prefix("http://") {
        ("http", r)
    } else {
        return Err(format!("unsupported URL scheme: {}", url));
    };
    let is_https = scheme == "https";
    let (host_port, path) = rest
        .find('/')
        .map(|i| (&rest[..i], &rest[i..]))
        .unwrap_or((rest, "/"));
    let (host, port) = host_port
        .rfind(':')
        .map(|i| {
            let h = host_port[..i].to_string();
            let p: u16 = host_port[i + 1..].parse().map_err(|_| "invalid port")?;
            Ok::<_, String>((h, p))
        })
        .transpose()?
        .unwrap_or_else(|| (host_port.to_string(), if is_https { 443 } else { 80 }));
    Ok((host, path.to_string(), port, is_https))
}

/// Plain HTTP fetch — hand-rolled HTTP/1.1 over std::net::TcpStream.
fn fetch_http_blocking(host: &str, port: u16, path: &str) -> Result<Vec<u8>, String> {
    use std::io::{Read, Write};

    let addr = format!("{}:{}", host, port);
    let mut stream =
        std::net::TcpStream::connect(&addr).map_err(|e| format!("connect to {}: {}", addr, e))?;
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(30)))
        .ok();

    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: dsearch/0.1\r\nAccept: */*\r\nConnection: close\r\n\r\n",
        path, host
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("write request: {}", e))?;

    let mut response = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = stream
            .read(&mut buf)
            .map_err(|e| format!("read response: {}", e))?;
        if n == 0 {
            break;
        }
        response.extend_from_slice(&buf[..n]);
        if response.len() > 2 * 1024 * 1024 {
            return Err("response too large (>2 MB)".to_string());
        }
    }
    Ok(response)
}

/// HTTPS fetch — hand-rolled TLS over std::net::TcpStream using the existing rustls crate.
fn fetch_https_blocking(host: &str, port: u16, path: &str) -> Result<Vec<u8>, String> {
    use std::io::{Read, Write};
    use std::sync::Arc;

    let mut root_store = rustls::RootCertStore::empty();
    load_native_certs(&mut root_store);

    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    let config = Arc::new(config);

    let name = rustls::pki_types::ServerName::try_from(host)
        .map_err(|e| format!("invalid DNS name {}: {:?}", host, e))?
        .to_owned();

    let addr = format!("{}:{}", host, port);
    let mut stream =
        std::net::TcpStream::connect(&addr).map_err(|e| format!("connect to {}: {}", addr, e))?;
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(30)))
        .ok();

    let mut conn = rustls::ClientConnection::new(config, name)
        .map_err(|e| format!("TLS init for {}: {:?}", host, e))?;

    let mut tls = rustls::Stream::new(&mut conn, &mut stream);

    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: dsearch/0.1\r\nAccept: */*\r\nConnection: close\r\n\r\n",
        path, host
    );
    tls.write_all(request.as_bytes())
        .map_err(|e| format!("TLS write: {:?}", e))?;

    let mut response = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = match tls.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(format!("TLS read: {:?}", e)),
        };
        response.extend_from_slice(&buf[..n]);
        if response.len() > 2 * 1024 * 1024 {
            return Err("response too large (>2 MB)".to_string());
        }
    }

    Ok(response)
}

// ---------------------------------------------------------------------------
// Native cert loading — hand-rolled PEM parsing, no extra deps
// ---------------------------------------------------------------------------

fn load_native_certs(root_store: &mut rustls::RootCertStore) {
    #[cfg(target_os = "linux")]
    {
        try_load_pem_file(root_store, "/etc/ssl/certs/ca-certificates.crt");
        try_load_pem_file(root_store, "/etc/pki/tls/certs/ca-bundle.crt");
        try_load_pem_file(root_store, "/etc/ssl/ca-bundle.pem");
    }
    #[cfg(target_os = "macos")]
    {
        try_load_pem_file(root_store, "/etc/ssl/cert.pem");
        try_load_pem_file(root_store, "/etc/ssl/certs/ca-certificates.crt");
    }
    #[cfg(target_os = "windows")]
    {
        // Windows system cert store access: use the certutil command to export
        // the system CA bundle as PEM, then load it. This avoids adding a
        // Windows-specific crate dependency while still supporting HTTPS scraping.
        let temp_dir = std::env::temp_dir();
        let export_path = temp_dir.join("dsearch_ca_bundle.pem");
        // Try exporting the system root CA store via certutil
        let output = std::process::Command::new("certutil")
            .args([
                "-convert",
                "-f",
                "ROOT",
                "PEM",
                &export_path.to_string_lossy(),
            ])
            .output();
        if let Ok(out) = &output {
            if out.status.success() && export_path.exists() {
                try_load_pem_file(root_store, &export_path.to_string_lossy());
                let _ = std::fs::remove_file(&export_path);
            }
        }
        // Fallback: try common PEM bundle locations
        try_load_pem_file(root_store, "C:\\ProgramData\\curl\\ca-bundle.crt");
        try_load_pem_file(root_store, "C:\\curl\\ca-bundle.crt");
    }
}

/// Try to load PEM-encoded CA certs from a file path.
/// PEM is just base64 between `-----BEGIN CERTIFICATE-----` / `-----END CERTIFICATE-----`
/// delimiters — plain format logic, hand-rolled per the roadmap.
fn try_load_pem_file(root_store: &mut rustls::RootCertStore, path: &str) {
    let Ok(pem_data) = std::fs::read(path) else {
        return;
    };
    let pem = String::from_utf8_lossy(&pem_data);
    for block in pem.split("-----BEGIN CERTIFICATE-----").skip(1) {
        let Some(b64) = block.split("-----END CERTIFICATE-----").next() else {
            continue;
        };
        let b64_clean: String = b64.chars().filter(|c| !c.is_ascii_whitespace()).collect();
        if let Ok(der) = base64_decode(&b64_clean) {
            root_store
                .add(rustls::pki_types::CertificateDer::from(der))
                .ok();
        }
    }
}

/// Hand-rolled base64 decoder — standard alphabet, no padding required.
/// This is plain format logic (RFC 4648 §4), not a crypto primitive.
fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let input = input.trim_end_matches('=');
    if input.is_empty() {
        return Ok(Vec::new());
    }

    let mut buf = Vec::with_capacity(input.len() * 3 / 4);
    let mut acc: u32 = 0;
    let mut bits: usize = 0;

    for ch in input.chars() {
        let val = TABLE
            .iter()
            .position(|&b| b == ch as u8)
            .ok_or_else(|| format!("invalid base64 char: {}", ch))? as u32;
        acc = (acc << 6) | val;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            buf.push((acc >> bits) as u8);
        }
    }

    Ok(buf)
}
