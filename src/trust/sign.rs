use ed25519_dalek::{SigningKey, VerifyingKey, Signature, Signer, Verifier};

/// Canonical encoding: fields concatenated in struct-declaration order,
/// each length-prefixed (u32 BE), UTF-8 for strings.
pub fn canonical_encode_fields(fields: &[&[u8]]) -> Vec<u8> {
    let mut buf = Vec::new();
    for field in fields {
        buf.extend_from_slice(&(field.len() as u32).to_be_bytes());
        buf.extend_from_slice(field);
    }
    buf
}

/// Sign a ContentRecord. Covers all fields except `sig`.
/// sign(id, source_url, source_hash, schema, tags, body, created_at, expires_at,
///      scrape_source, refresh_policy)
pub fn sign_record(
    signing_key: &SigningKey,
    id: &[u8],
    source_url: &[u8],
    source_hash: &[u8],
    schema: &[u8],
    tags: &[u8],
    body: &[u8],
    created_at: &[u8],
    expires_at: &[u8],
    scrape_source: &[u8],
    refresh_policy: &[u8],
) -> Signature {
    let encoded = canonical_encode_fields(&[
        id, source_url, source_hash, schema, tags, body,
        created_at, expires_at, scrape_source, refresh_policy,
    ]);
    signing_key.sign(&encoded)
}

/// Verify a ContentRecord signature.
pub fn verify_record_sig(
    verifying_key: &VerifyingKey,
    id: &[u8],
    source_url: &[u8],
    source_hash: &[u8],
    schema: &[u8],
    tags: &[u8],
    body: &[u8],
    created_at: &[u8],
    expires_at: &[u8],
    scrape_source: &[u8],
    refresh_policy: &[u8],
    sig: &Signature,
) -> bool {
    let encoded = canonical_encode_fields(&[
        id, source_url, source_hash, schema, tags, body,
        created_at, expires_at, scrape_source, refresh_policy,
    ]);
    verifying_key.verify(&encoded, sig).is_ok()
}

/// Sign an Announcement. Covers all fields except `sig`.
/// sign(record_id, source_hash, schema, tags, holder_addr, expires_at)
pub fn sign_announcement(
    signing_key: &SigningKey,
    record_id: &[u8],
    source_hash: &[u8],
    schema: &[u8],
    tags: &[u8],
    holder_addr: &[u8],
    expires_at: &[u8],
) -> Signature {
    let encoded = canonical_encode_fields(&[record_id, source_hash, schema, tags, holder_addr, expires_at]);
    signing_key.sign(&encoded)
}

/// Verify an Announcement signature.
pub fn verify_announcement_sig(
    verifying_key: &VerifyingKey,
    record_id: &[u8],
    source_hash: &[u8],
    schema: &[u8],
    tags: &[u8],
    holder_addr: &[u8],
    expires_at: &[u8],
    sig: &Signature,
) -> bool {
    let encoded = canonical_encode_fields(&[record_id, source_hash, schema, tags, holder_addr, expires_at]);
    verifying_key.verify(&encoded, sig).is_ok()
}

/// Compute record_id as Blake3 of canonical content fields.
/// Includes: source_url, source_hash, schema, tags, body, created_at
pub fn compute_record_id(
    source_url: &[u8],
    source_hash: &[u8],
    schema: &[u8],
    tags: &[u8],
    body: &[u8],
    created_at: &[u8],
) -> String {
    let encoded = canonical_encode_fields(&[source_url, source_hash, schema, tags, body, created_at]);
    blake3::hash(&encoded).to_hex().to_string()
}

/// Well-known tracking query parameters to strip during URL normalization.
const TRACKING_PARAMS: &[&str] = &[
    "utm_source",
    "utm_medium",
    "utm_campaign",
    "utm_term",
    "utm_content",
    "utm_id",
    "fbclid",
    "gclid",
    "gclsrc",
    "dclid",
    "msclkid",
    "mc_eid",
    "_ga",
    "_gl",
    "_hsenc",
    "_hsmi",
    "hsCtaTracking",
    "ver",
    "ref",
    "referrer",
    "source",
    "s_cid",
    "elqTrackId",
    "elqTrack",
    "assetType",
    "assetId",
    "recipient_id",
    "campaign_id",
    "site_id",
];

/// Normalize a source URL for canonical hashing.
/// - Lowercase the scheme and host
/// - Strip tracking query parameters (utm_*, fbclid, gclid, etc.)
/// - Remove trailing slash from path (unless path is just "/")
/// - Remove fragment (#...)
/// - Sort remaining query parameters for deterministic ordering
pub fn normalize_source_url(url: &str) -> String {
    let url = url.trim();

    // Split off fragment — everything after # is discarded
    let url = match url.find('#') {
        Some(pos) => &url[..pos],
        None => url,
    };

    // Split into scheme+host vs path+query
    let after_scheme = match url.find("://") {
        Some(pos) => pos + 3,
        None => return url.to_string(), // No scheme, return as-is
    };

    let scheme = &url[..after_scheme].to_lowercase();
    let rest = &url[after_scheme..];

    // Split host from path+query at the first /
    let (host, path_and_query) = match rest.find('/') {
        Some(pos) => (&rest[..pos], &rest[pos..]),
        None => (rest, "/"),
    };

    let host_lower = host.to_lowercase();

    // Split path from query
    let (path, query_string) = match path_and_query.find('?') {
        Some(pos) => (&path_and_query[..pos], &path_and_query[pos + 1..]),
        None => (path_and_query, ""),
    };

    // Strip trailing slash from path (but keep "/" as-is)
    let path_stripped = if path.len() > 1 && path.ends_with('/') {
        &path[..path.len() - 1]
    } else {
        path
    };

    // Parse and filter query parameters
    let mut params: Vec<(&str, &str)> = Vec::new();
    if !query_string.is_empty() {
        for pair in query_string.split('&') {
            if pair.is_empty() {
                continue;
            }
            match pair.find('=') {
                Some(pos) => {
                    let key = &pair[..pos];
                    // Skip tracking parameters (case-insensitive)
                    if TRACKING_PARAMS.contains(&key.to_lowercase().as_str()) {
                        continue;
                    }
                    params.push((key, &pair[pos + 1..]));
                }
                None => {
                    let key = pair;
                    if TRACKING_PARAMS.contains(&key.to_lowercase().as_str()) {
                        continue;
                    }
                    params.push((key, ""));
                }
            }
        }
    }

    // Sort params by key for deterministic ordering
    params.sort_by_key(|(k, _)| *k);

    // Reconstruct
    let mut result = format!("{}{}", scheme, host_lower);
    result.push_str(path_stripped);

    if !params.is_empty() {
        result.push('?');
        for (i, (k, v)) in params.iter().enumerate() {
            if i > 0 {
                result.push('&');
            }
            result.push_str(k);
            if !v.is_empty() {
                result.push('=');
                result.push_str(v);
            }
        }
    }

    result
}

/// Compute source_hash as Blake3 of the canonical (normalized) source URL.
pub fn compute_source_hash(source_url: &[u8]) -> String {
    let url_str = std::str::from_utf8(source_url).unwrap_or("");
    let normalized = normalize_source_url(url_str);
    blake3::hash(normalized.as_bytes()).to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_verify_record_roundtrip() {
        let mut rng = rand::rngs::OsRng;
        let sk = SigningKey::generate(&mut rng);
        let vk = sk.verifying_key();

        let sig = sign_record(&sk, b"id", b"url", b"hash", b"schema", b"tags", b"body", b"1234", b"5678", b"url", b"once");
        assert!(verify_record_sig(&vk, b"id", b"url", b"hash", b"schema", b"tags", b"body", b"1234", b"5678", b"url", b"once", &sig));
        assert!(!verify_record_sig(&vk, b"wrong", b"url", b"hash", b"schema", b"tags", b"body", b"1234", b"5678", b"url", b"once", &sig));
    }

    #[test]
    fn sign_verify_announcement_roundtrip() {
        let mut rng = rand::rngs::OsRng;
        let sk = SigningKey::generate(&mut rng);
        let vk = sk.verifying_key();

        let sig = sign_announcement(&sk, b"rid", b"shash", b"schema", b"tags", b"addr", b"exp");
        assert!(verify_announcement_sig(&vk, b"rid", b"shash", b"schema", b"tags", b"addr", b"exp", &sig));
        assert!(!verify_announcement_sig(&vk, b"wrong", b"shash", b"schema", b"tags", b"addr", b"exp", &sig));
    }

    #[test]
    fn canonical_encoding_deterministic() {
        let a = canonical_encode_fields(&[b"hello", b"world"]);
        let b = canonical_encode_fields(&[b"hello", b"world"]);
        assert_eq!(a, b);
    }

    #[test]
    fn compute_record_id_deterministic() {
        let a = compute_record_id(b"url", b"hash", b"schema", b"tags", b"body", b"1234");
        let b = compute_record_id(b"url", b"hash", b"schema", b"tags", b"body", b"1234");
        assert_eq!(a, b);
    }

    #[test]
    fn compute_source_hash_deterministic() {
        let a = compute_source_hash(b"https://example.com/page");
        let b = compute_source_hash(b"https://example.com/page");
        assert_eq!(a, b);
        assert_ne!(compute_source_hash(b"https://example.com/other"), a);
    }

    #[test]
    fn normalize_lowercases_scheme_and_host() {
        assert_eq!(
            normalize_source_url("HTTPS://EXAMPLE.COM/page"),
            "https://example.com/page"
        );
    }

    #[test]
    fn normalize_strips_tracking_params() {
        assert_eq!(
            normalize_source_url("https://example.com/page?utm_source=twitter&id=123"),
            "https://example.com/page?id=123"
        );
    }

    #[test]
    fn normalize_strips_fragment() {
        assert_eq!(
            normalize_source_url("https://example.com/page#section"),
            "https://example.com/page"
        );
    }

    #[test]
    fn normalize_strips_trailing_slash() {
        assert_eq!(
            normalize_source_url("https://example.com/page/"),
            "https://example.com/page"
        );
    }

    #[test]
    fn normalize_sorts_query_params() {
        assert_eq!(
            normalize_source_url("https://example.com/page?b=2&a=1"),
            "https://example.com/page?a=1&b=2"
        );
    }

    #[test]
    fn normalize_case_insensitive_dedup() {
        // Same URL with different case should produce the same source_hash
        let a = compute_source_hash(b"https://Example.com/Page?ID=1");
        let b = compute_source_hash(b"https://example.com/Page?ID=1");
        assert_eq!(a, b);
    }

    #[test]
    fn normalize_tracking_params_case_insensitive() {
        assert_eq!(
            normalize_source_url("https://example.com/page?UTM_SOURCE=tw&Id=1"),
            "https://example.com/page?Id=1"
        );
    }

    #[test]
    fn normalize_preserves_path() {
        assert_eq!(
            normalize_source_url("https://example.com/path/to/resource"),
            "https://example.com/path/to/resource"
        );
    }

    #[test]
    fn normalize_no_query() {
        assert_eq!(
            normalize_source_url("https://example.com/page"),
            "https://example.com/page"
        );
    }

    #[test]
    fn normalize_root_path() {
        assert_eq!(
            normalize_source_url("https://example.com/"),
            "https://example.com/"
        );
    }
}
