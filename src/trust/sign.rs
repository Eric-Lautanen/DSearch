pub fn canonical_encode_fields(fields: &[&[u8]]) -> Vec<u8> {
    let mut buf = Vec::new();
    for field in fields {
        buf.extend_from_slice(&(field.len() as u32).to_be_bytes());
        buf.extend_from_slice(field);
    }
    buf
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

/// Compute source_hash as Blake3 of canonical source URL.
pub fn compute_source_hash(source_url: &[u8]) -> String {
    blake3::hash(source_url).to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
