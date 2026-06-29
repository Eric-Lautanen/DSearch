use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};

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

/// Fields of a ContentRecord, used for signing and verification.
/// Groups the many record fields into a struct to avoid too_many_arguments.
/// Owns its data so it can outlive the temporary strings it's built from.
#[derive(Clone)]
pub struct RecordFields {
    pub id: Vec<u8>,
    pub source_url: Vec<u8>,
    pub source_hash: Vec<u8>,
    pub schema: Vec<u8>,
    pub tags: Vec<u8>,
    pub body: Vec<u8>,
    pub created_at: Vec<u8>,
    pub expires_at: Vec<u8>,
    pub scrape_source: Vec<u8>,
    pub refresh_policy: Vec<u8>,
}

impl RecordFields {
    fn encode(&self) -> Vec<u8> {
        canonical_encode_fields(&[
            &self.id,
            &self.source_url,
            &self.source_hash,
            &self.schema,
            &self.tags,
            &self.body,
            &self.created_at,
            &self.expires_at,
            &self.scrape_source,
            &self.refresh_policy,
        ])
    }
}

/// Fields of an Announcement, used for signing and verification.
/// Groups the announcement fields into a struct to avoid too_many_arguments.
/// Owns its data so it can outlive the temporary strings it's built from.
#[derive(Clone)]
pub struct AnnouncementFields {
    pub record_id: Vec<u8>,
    pub source_hash: Vec<u8>,
    pub schema: Vec<u8>,
    pub tags: Vec<u8>,
    pub holder_addr: Vec<u8>,
    pub expires_at: Vec<u8>,
}

impl AnnouncementFields {
    fn encode(&self) -> Vec<u8> {
        canonical_encode_fields(&[
            &self.record_id,
            &self.source_hash,
            &self.schema,
            &self.tags,
            &self.holder_addr,
            &self.expires_at,
        ])
    }
}

/// Sign a ContentRecord. Covers all fields except `sig`.
pub fn sign_record(signing_key: &SigningKey, fields: &RecordFields) -> Signature {
    signing_key.sign(&fields.encode())
}

/// Verify a ContentRecord signature.
pub fn verify_record_sig(
    verifying_key: &VerifyingKey,
    fields: &RecordFields,
    sig: &Signature,
) -> bool {
    verifying_key.verify(&fields.encode(), sig).is_ok()
}

/// Sign an Announcement. Covers all fields except `sig`.
pub fn sign_announcement(signing_key: &SigningKey, fields: &AnnouncementFields) -> Signature {
    signing_key.sign(&fields.encode())
}

/// Verify an Announcement signature.
pub fn verify_announcement_sig(
    verifying_key: &VerifyingKey,
    fields: &AnnouncementFields,
    sig: &Signature,
) -> bool {
    verifying_key.verify(&fields.encode(), sig).is_ok()
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
    let encoded =
        canonical_encode_fields(&[source_url, source_hash, schema, tags, body, created_at]);
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

        let fields = RecordFields {
            id: b"id".to_vec(),
            source_url: b"url".to_vec(),
            source_hash: b"hash".to_vec(),
            schema: b"schema".to_vec(),
            tags: b"tags".to_vec(),
            body: b"body".to_vec(),
            created_at: b"1234".to_vec(),
            expires_at: b"5678".to_vec(),
            scrape_source: b"url".to_vec(),
            refresh_policy: b"once".to_vec(),
        };
        let sig = sign_record(&sk, &fields);
        assert!(verify_record_sig(&vk, &fields, &sig));
        let wrong_fields = RecordFields {
            id: b"wrong".to_vec(),
            ..fields.clone()
        };
        assert!(!verify_record_sig(&vk, &wrong_fields, &sig));
    }

    #[test]
    fn sign_verify_announcement_roundtrip() {
        let mut rng = rand::rngs::OsRng;
        let sk = SigningKey::generate(&mut rng);
        let vk = sk.verifying_key();

        let fields = AnnouncementFields {
            record_id: b"rid".to_vec(),
            source_hash: b"shash".to_vec(),
            schema: b"schema".to_vec(),
            tags: b"tags".to_vec(),
            holder_addr: b"addr".to_vec(),
            expires_at: b"exp".to_vec(),
        };
        let sig = sign_announcement(&sk, &fields);
        assert!(verify_announcement_sig(&vk, &fields, &sig));
        let wrong_fields = AnnouncementFields {
            record_id: b"wrong".to_vec(),
            ..fields.clone()
        };
        assert!(!verify_announcement_sig(&vk, &wrong_fields, &sig));
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
