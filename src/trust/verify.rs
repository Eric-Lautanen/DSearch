use ed25519_dalek::{Signature, VerifyingKey};

use crate::trust::sign::{AnnouncementFields, RecordFields};

/// Verification result for signatures.
#[derive(Debug, Clone)]
pub struct VerifyResult {
    pub valid: bool,
    pub reason: Option<String>,
}

impl VerifyResult {
    pub fn ok() -> Self {
        Self {
            valid: true,
            reason: None,
        }
    }

    pub fn fail(reason: impl Into<String>) -> Self {
        Self {
            valid: false,
            reason: Some(reason.into()),
        }
    }
}

/// Verify a ContentRecord signature.
pub fn verify_record_signature(
    verifying_key: &VerifyingKey,
    fields: &RecordFields,
    sig: &Signature,
) -> VerifyResult {
    if crate::trust::sign::verify_record_sig(verifying_key, fields, sig) {
        VerifyResult::ok()
    } else {
        VerifyResult::fail("ContentRecord signature verification failed")
    }
}

/// Verify an Announcement signature.
pub fn verify_announcement_signature(
    verifying_key: &VerifyingKey,
    fields: &AnnouncementFields,
    sig: &Signature,
) -> VerifyResult {
    if crate::trust::sign::verify_announcement_sig(verifying_key, fields, sig) {
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
    let computed = crate::trust::sign::compute_record_id(
        source_url,
        source_hash,
        schema,
        tags,
        body,
        created_at,
    );
    if computed == record_id {
        VerifyResult::ok()
    } else {
        VerifyResult::fail("record_id does not match content hash")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    #[test]
    fn verify_result_ok() {
        let result = VerifyResult::ok();
        assert!(result.valid);
        assert!(result.reason.is_none());
    }

    #[test]
    fn verify_result_fail() {
        let result = VerifyResult::fail("test reason");
        assert!(!result.valid);
        assert_eq!(result.reason.unwrap(), "test reason");
    }

    #[test]
    fn verify_record_signature_valid() {
        let mut rng = OsRng;
        let sk = SigningKey::generate(&mut rng);
        let vk = sk.verifying_key();

        let fields = RecordFields {
            id: b"test-id".to_vec(),
            source_url: b"https://example.com".to_vec(),
            source_hash: b"hash123".to_vec(),
            schema: b"wiki/article".to_vec(),
            tags: b"test".to_vec(),
            body: b"body content".to_vec(),
            created_at: b"1000".to_vec(),
            expires_at: b"2000".to_vec(),
            scrape_source: b"url".to_vec(),
            refresh_policy: b"once".to_vec(),
        };

        let sig = crate::trust::sign::sign_record(&sk, &fields);
        let result = verify_record_signature(&vk, &fields, &sig);
        assert!(result.valid);
    }

    #[test]
    fn verify_record_signature_wrong_key() {
        let mut rng = OsRng;
        let sk = SigningKey::generate(&mut rng);
        let wrong_sk = SigningKey::generate(&mut rng);
        let wrong_vk = wrong_sk.verifying_key();

        let fields = RecordFields {
            id: b"test-id".to_vec(),
            source_url: b"https://example.com".to_vec(),
            source_hash: b"hash123".to_vec(),
            schema: b"wiki/article".to_vec(),
            tags: b"test".to_vec(),
            body: b"body content".to_vec(),
            created_at: b"1000".to_vec(),
            expires_at: b"2000".to_vec(),
            scrape_source: b"url".to_vec(),
            refresh_policy: b"once".to_vec(),
        };

        let sig = crate::trust::sign::sign_record(&sk, &fields);
        let result = verify_record_signature(&wrong_vk, &fields, &sig);
        assert!(!result.valid);
    }

    #[test]
    fn verify_announcement_signature_valid() {
        let mut rng = OsRng;
        let sk = SigningKey::generate(&mut rng);
        let vk = sk.verifying_key();

        let fields = AnnouncementFields {
            record_id: b"rid1".to_vec(),
            source_hash: b"sh1".to_vec(),
            schema: b"wiki/article".to_vec(),
            tags: b"test".to_vec(),
            holder_addr: b"1.2.3.4:7744".to_vec(),
            expires_at: b"9999".to_vec(),
        };

        let sig = crate::trust::sign::sign_announcement(&sk, &fields);
        let result = verify_announcement_signature(&vk, &fields, &sig);
        assert!(result.valid);
    }

    #[test]
    fn verify_announcement_signature_wrong_key() {
        let mut rng = OsRng;
        let sk = SigningKey::generate(&mut rng);
        let wrong_sk = SigningKey::generate(&mut rng);
        let wrong_vk = wrong_sk.verifying_key();

        let fields = AnnouncementFields {
            record_id: b"rid1".to_vec(),
            source_hash: b"sh1".to_vec(),
            schema: b"wiki/article".to_vec(),
            tags: b"test".to_vec(),
            holder_addr: b"1.2.3.4:7744".to_vec(),
            expires_at: b"9999".to_vec(),
        };

        let sig = crate::trust::sign::sign_announcement(&sk, &fields);
        let result = verify_announcement_signature(&wrong_vk, &fields, &sig);
        assert!(!result.valid);
    }

    #[test]
    fn verify_record_id_valid() {
        let computed = crate::trust::sign::compute_record_id(
            b"https://example.com",
            b"hash123",
            b"wiki/article",
            b"test",
            b"body",
            b"1000",
        );
        let result = verify_record_id(
            &computed,
            b"https://example.com",
            b"hash123",
            b"wiki/article",
            b"test",
            b"body",
            b"1000",
        );
        assert!(result.valid);
    }

    #[test]
    fn verify_record_id_invalid() {
        let result = verify_record_id(
            "wrong-id",
            b"https://example.com",
            b"hash123",
            b"wiki/article",
            b"test",
            b"body",
            b"1000",
        );
        assert!(!result.valid);
    }
}
