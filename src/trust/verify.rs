use ed25519_dalek::{VerifyingKey, Signature};

/// Verification result for signatures.
#[derive(Debug, Clone)]
pub struct VerifyResult {
    pub valid: bool,
    pub reason: Option<String>,
}

impl VerifyResult {
    pub fn ok() -> Self {
        Self { valid: true, reason: None }
    }

    pub fn fail(reason: impl Into<String>) -> Self {
        Self { valid: false, reason: Some(reason.into()) }
    }
}

/// Verify a ContentRecord signature.
pub fn verify_record_signature(
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
) -> VerifyResult {
    if crate::trust::sign::verify_record_sig(
        verifying_key, id, source_url, source_hash, schema, tags, body,
        created_at, expires_at, scrape_source, refresh_policy, sig,
    ) {
        VerifyResult::ok()
    } else {
        VerifyResult::fail("ContentRecord signature verification failed")
    }
}

/// Verify an Announcement signature.
pub fn verify_announcement_signature(
    verifying_key: &VerifyingKey,
    record_id: &[u8],
    source_hash: &[u8],
    schema: &[u8],
    tags: &[u8],
    holder_addr: &[u8],
    expires_at: &[u8],
    sig: &Signature,
) -> VerifyResult {
    if crate::trust::sign::verify_announcement_sig(
        verifying_key, record_id, source_hash, schema, tags, holder_addr, expires_at, sig,
    ) {
        VerifyResult::ok()
    } else {
        VerifyResult::fail("Announcement signature verification failed")
    }
}

/// Verify that a record_id matches the Blake3 of the canonical content fields.
pub fn verify_record_id(
    record_id: &str,
    source_url: &[u8],
    source_hash: &[u8],
    schema: &[u8],
    tags: &[u8],
    body: &[u8],
    created_at: &[u8],
) -> VerifyResult {
    let computed = crate::trust::sign::compute_record_id(source_url, source_hash, schema, tags, body, created_at);
    if computed == record_id {
        VerifyResult::ok()
    } else {
        VerifyResult::fail("record_id does not match content hash")
    }
}
