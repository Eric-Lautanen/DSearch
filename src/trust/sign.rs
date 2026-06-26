use ed25519_dalek::{SigningKey, Signature, Signer, Verifier, VerifyingKey};

/// Canonical encoding: fields concatenated in struct-declaration order,
/// each length-prefixed (u32 BE), UTF-8 for strings.
/// This is the same framing discipline as the wire protocol.

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

/// Compute source_hash as Blake3 of canonical source URL.
pub fn compute_source_hash(source_url: &[u8]) -> String {
    blake3::hash(source_url).to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;

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
    fn sign_verify_record_wrong_scrape_source() {
        let mut rng = rand::rngs::OsRng;
        let sk = SigningKey::generate(&mut rng);
        let vk = sk.verifying_key();

        let sig = sign_record(&sk, b"id", b"url", b"hash", b"schema", b"tags", b"body", b"1234", b"5678", b"url", b"once");
        assert!(!verify_record_sig(&vk, b"id", b"url", b"hash", b"schema", b"tags", b"body", b"1234", b"5678", b"api", b"once", &sig));
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
}
