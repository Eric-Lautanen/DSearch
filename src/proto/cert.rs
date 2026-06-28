use ed25519_dalek::pkcs8::EncodePrivateKey;
use ed25519_dalek::{Signature, SigningKey, Verifier, VerifyingKey};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use std::path::Path;
use x509_cert::der::{Decode, Encode};
use x509_cert::Certificate;

/// Generate an Ed25519 keypair and derive a self-signed TLS certificate
/// using the **same** key for both identity signing and TLS.
/// Returns (signing_key, node_id_hex, cert_der, key_pkcs8_der).
pub fn generate_identity() -> Result<(SigningKey, String, Vec<u8>, Vec<u8>), CertError> {
    let mut rng = rand::rngs::OsRng;
    let signing_key = SigningKey::generate(&mut rng);
    let node_id = node_id_from_pubkey(&signing_key.verifying_key());

    let (cert_der, key_der) = make_cert_and_key(&signing_key, &node_id)?;

    Ok((signing_key, node_id, cert_der, key_der))
}

/// Build a self-signed TLS cert + PKCS#8 private key DER from the signing key.
/// Uses the same Ed25519 keypair for both the certificate and TLS,
/// so the cert's public key always matches the private key.
fn make_cert_and_key(
    signing_key: &SigningKey,
    node_id: &str,
) -> Result<(Vec<u8>, Vec<u8>), CertError> {
    let pkcs8_doc = signing_key
        .to_pkcs8_der()
        .map_err(|e| CertError::Generation(format!("PKCS#8 encode: {}", e)))?;
    let pkcs8_bytes = pkcs8_doc.as_bytes();

    let key_pair = rcgen::KeyPair::from_pkcs8_der_and_sign_algo(
        &PrivatePkcs8KeyDer::from(pkcs8_bytes.to_vec()),
        &rcgen::PKCS_ED25519,
    )
    .map_err(|e| CertError::Generation(format!("rcgen KeyPair from PKCS#8: {}", e)))?;

    let mut params = rcgen::CertificateParams::default();
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "dsearch");
    params.subject_alt_names.push(rcgen::SanType::URI(
        rcgen::Ia5String::try_from(node_id.to_string())
            .map_err(|e| CertError::Generation(format!("invalid SAN URI: {}", e)))?,
    ));
    let cert = params
        .self_signed(&key_pair)
        .map_err(|e| CertError::Generation(e.to_string()))?;

    Ok((cert.der().to_vec(), key_pair.serialize_der()))
}

/// Derive node_id as Blake3 hex of the Ed25519 public key bytes.
pub fn node_id_from_pubkey(verifying_key: &VerifyingKey) -> String {
    let pubkey_bytes = verifying_key.to_bytes();
    blake3::hash(&pubkey_bytes).to_hex().to_string()
}

/// Save identity key, TLS key, and cert to data_dir.
/// We store the raw Ed25519 secret key bytes (32 bytes) as identity.key,
/// the PKCS#8 TLS private key as identity.tls,
/// and the TLS cert DER as node.crt.
pub fn save_identity(
    data_dir: &Path,
    signing_key: &SigningKey,
    cert_der: &[u8],
    key_der: &[u8],
) -> Result<(), CertError> {
    std::fs::create_dir_all(data_dir).map_err(CertError::Io)?;

    let key_bytes = signing_key.to_bytes();
    let key_path = data_dir.join("identity.key");
    std::fs::write(&key_path, key_bytes).map_err(CertError::Io)?;

    // Restrict identity.key to owner-read-write only
    restrict_file_permissions(&key_path);

    std::fs::write(data_dir.join("node.crt"), cert_der).map_err(CertError::Io)?;

    std::fs::write(data_dir.join("identity.tls"), key_der).map_err(CertError::Io)?;

    Ok(())
}

/// Set file permissions to owner-read-write only (0600 on Unix, ACLs on Windows).
fn restrict_file_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        let _ = std::fs::set_permissions(path, perms);
    }
    #[cfg(windows)]
    {
        // On Windows, use icacls to remove inherited permissions and grant
        // only the current user full control. Best-effort — if it fails,
        // the file still exists with default permissions.
        let path_str = match path.to_str() {
            Some(s) => s.to_string(),
            None => return,
        };
        // Get current username from environment
        let username = std::env::var("USERNAME").unwrap_or_else(|_| "CURRENT_USER".to_string());
        let _ = std::process::Command::new("icacls")
            .args([
                &path_str,
                "/inheritance:r",
                "/grant:r",
                &format!("{}:(F)", username),
            ])
            .output();
    }
}

/// Load existing identity from data_dir, or generate and save a new one.
pub fn load_or_generate_identity(
    data_dir: &Path,
) -> Result<(SigningKey, String, Vec<u8>, Vec<u8>), CertError> {
    let key_path = data_dir.join("identity.key");
    let cert_path = data_dir.join("node.crt");
    let tls_key_path = data_dir.join("identity.tls");

    if key_path.exists() && cert_path.exists() && tls_key_path.exists() {
        let key_bytes = std::fs::read(&key_path).map_err(CertError::Io)?;
        let cert_der = std::fs::read(&cert_path).map_err(CertError::Io)?;
        let tls_key_der = std::fs::read(&tls_key_path).map_err(CertError::Io)?;

        let signing_key = SigningKey::from_bytes(
            key_bytes
                .as_slice()
                .try_into()
                .map_err(|_| CertError::KeyFormat("identity.key must be 32 bytes".to_string()))?,
        );
        let node_id = node_id_from_pubkey(&signing_key.verifying_key());

        let (_, expected_key_der) = make_cert_and_key(&signing_key, &node_id)?;
        if tls_key_der != expected_key_der {
            let (new_cert_der, new_key_der) = make_cert_and_key(&signing_key, &node_id)?;
            std::fs::write(&cert_path, &new_cert_der).map_err(CertError::Io)?;
            std::fs::write(&tls_key_path, &new_key_der).map_err(CertError::Io)?;
            return Ok((signing_key, node_id, new_cert_der, new_key_der));
        }

        Ok((signing_key, node_id, cert_der, tls_key_der))
    } else {
        let (signing_key, node_id, cert_der, key_der) = generate_identity()?;
        save_identity(data_dir, &signing_key, &cert_der, &key_der)?;
        Ok((signing_key, node_id, cert_der, key_der))
    }
}

/// Build a Quinn server configuration from cert_der and tls_key_der.
pub fn server_config(
    cert_der: &[u8],
    tls_key_der: &[u8],
) -> Result<quinn::ServerConfig, CertError> {
    let cert = CertificateDer::from(cert_der.to_vec());
    let key = PrivateKeyDer::from(PrivatePkcs8KeyDer::from(tls_key_der.to_vec()));

    let tls_server_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .map_err(|e| CertError::Tls(e.to_string()))?;

    let quic_server_config = quinn::crypto::rustls::QuicServerConfig::try_from(tls_server_config)
        .map_err(|e| CertError::Tls(e.to_string()))?;

    let mut server_config =
        quinn::ServerConfig::with_crypto(std::sync::Arc::new(quic_server_config));
    let mut transport = quinn::TransportConfig::default();
    transport.max_idle_timeout(Some(std::time::Duration::from_secs(30).try_into().unwrap()));
    server_config.transport_config(std::sync::Arc::new(transport));

    Ok(server_config)
}

/// Build a Quinn client configuration with custom cert verifier.
pub fn client_config() -> Result<quinn::ClientConfig, CertError> {
    let tls_client_config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(std::sync::Arc::new(DsearchCertVerifier))
        .with_no_client_auth();

    let quic_client_config = quinn::crypto::rustls::QuicClientConfig::try_from(tls_client_config)
        .map_err(|e| CertError::Tls(e.to_string()))?;

    let mut client_config = quinn::ClientConfig::new(std::sync::Arc::new(quic_client_config));
    let mut transport = quinn::TransportConfig::default();
    transport.max_idle_timeout(Some(std::time::Duration::from_secs(30).try_into().unwrap()));
    client_config.transport_config(std::sync::Arc::new(transport));

    Ok(client_config)
}

/// Custom certificate verifier that validates self-signed dsearch certs.
///
/// Verification steps:
/// 1. Parse the X.509 certificate
/// 2. Verify issuer == subject (self-signed)
/// 3. Extract node_id from the SAN URI extension
/// 4. Extract the Ed25519 public key from subjectPublicKeyInfo
/// 5. Verify node_id == Blake3(Ed25519 pubkey bytes)
/// 6. Verify the certificate's self-signature using the public key
#[derive(Debug)]
struct DsearchCertVerifier;

impl rustls::client::danger::ServerCertVerifier for DsearchCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        verify_dsearch_cert(end_entity.as_ref())?;
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        verify_tls_signature(message, cert.as_ref(), dss.signature(), dss.scheme)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        verify_tls_signature(message, cert.as_ref(), dss.signature(), dss.scheme)
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![rustls::SignatureScheme::ED25519]
    }
}

/// Verify a dsearch self-signed certificate per the trust model:
/// - Self-signed (issuer == subject)
/// - SAN URI contains the node_id
/// - node_id == Blake3(Ed25519 public key bytes)
/// - Certificate signature is valid against its own public key
fn verify_dsearch_cert(cert_der: &[u8]) -> Result<(), rustls::Error> {
    let cert = Certificate::from_der(cert_der)
        .map_err(|e| rustls::Error::General(format!("cert parse: {}", e)))?;

    // 1. Self-signed check: issuer == subject
    if cert.tbs_certificate.issuer != cert.tbs_certificate.subject {
        return Err(rustls::Error::General(
            "cert not self-signed: issuer != subject".into(),
        ));
    }

    // 2. Extract node_id from SAN URI
    let node_id = extract_san_uri(&cert)
        .ok_or_else(|| rustls::Error::General("no SAN URI found in cert".into()))?;

    // 3. Extract Ed25519 public key
    let pubkey_bytes = extract_ed25519_pubkey(&cert)
        .ok_or_else(|| rustls::Error::General("no Ed25519 public key in cert".into()))?;

    // 4. Verify node_id matches Blake3(pubkey)
    let expected_node_id = blake3::hash(&pubkey_bytes).to_hex().to_string();
    if node_id != expected_node_id {
        return Err(rustls::Error::General(format!(
            "node_id mismatch: SAN claims {} but pubkey hashes to {}",
            node_id, expected_node_id
        )));
    }

    // 5. Verify certificate self-signature
    verify_cert_signature(&cert, &pubkey_bytes)?;

    Ok(())
}

/// Extract the URI value from the Subject Alternative Name extension (OID 2.5.29.17).
fn extract_san_uri(cert: &Certificate) -> Option<String> {
    let extensions = cert.tbs_certificate.extensions.as_ref()?;
    for ext in extensions.iter() {
        if ext.extn_id.to_string() == "2.5.29.17" {
            let san: x509_cert::ext::pkix::name::GeneralNames =
                x509_cert::der::Decode::from_der(ext.extn_value.as_bytes()).ok()?;
            for name in san {
                if let x509_cert::ext::pkix::name::GeneralName::UniformResourceIdentifier(uri) =
                    name
                {
                    return Some(uri.to_string());
                }
            }
        }
    }
    None
}

/// Extract the raw 32-byte Ed25519 public key from subjectPublicKeyInfo.
fn extract_ed25519_pubkey(cert: &Certificate) -> Option<[u8; 32]> {
    let spki = &cert.tbs_certificate.subject_public_key_info;
    if spki.algorithm.oid.to_string() != "1.3.101.112" {
        return None;
    }
    let raw = spki.subject_public_key.raw_bytes();
    if raw.len() != 32 {
        return None;
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(raw);
    Some(key)
}

/// Verify the certificate's self-signature: re-encode TBS, check with Ed25519.
fn verify_cert_signature(cert: &Certificate, pubkey_bytes: &[u8; 32]) -> Result<(), rustls::Error> {
    let verifying_key = VerifyingKey::from_bytes(pubkey_bytes)
        .map_err(|_| rustls::Error::General("invalid Ed25519 public key".into()))?;

    // Re-encode the TBS certificate to get the signed data
    let tbs_der = cert
        .tbs_certificate
        .to_der()
        .map_err(|e| rustls::Error::General(format!("TBS encode: {}", e)))?;

    let sig_raw = cert.signature.raw_bytes();
    if sig_raw.len() != 64 {
        return Err(rustls::Error::General(
            "invalid signature bit string".into(),
        ));
    }
    let sig_bytes = sig_raw;

    let signature = Signature::from_slice(sig_bytes)
        .map_err(|_| rustls::Error::General("invalid Ed25519 signature length".into()))?;

    verifying_key
        .verify(&tbs_der, &signature)
        .map_err(|_| rustls::Error::General("cert signature verification failed".into()))
}

/// Verify a TLS handshake signature (CertificateVerify) using the cert's Ed25519 key.
fn verify_tls_signature(
    message: &[u8],
    cert_der: &[u8],
    sig_bytes: &[u8],
    scheme: rustls::SignatureScheme,
) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
    if scheme != rustls::SignatureScheme::ED25519 {
        return Err(rustls::Error::General(
            "unsupported signature scheme".into(),
        ));
    }

    let cert = Certificate::from_der(cert_der)
        .map_err(|e| rustls::Error::General(format!("cert parse: {}", e)))?;
    let pubkey_bytes = extract_ed25519_pubkey(&cert)
        .ok_or_else(|| rustls::Error::General("no Ed25519 public key in cert".into()))?;

    let verifying_key = VerifyingKey::from_bytes(&pubkey_bytes)
        .map_err(|_| rustls::Error::General("invalid Ed25519 public key".into()))?;

    let signature = Signature::from_slice(sig_bytes)
        .map_err(|_| rustls::Error::General("invalid Ed25519 signature".into()))?;

    verifying_key
        .verify(message, &signature)
        .map_err(|_| rustls::Error::General("TLS signature verification failed".into()))?;

    Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
}

#[derive(Debug)]
pub enum CertError {
    Generation(String),
    Tls(String),
    KeyFormat(String),
    Io(std::io::Error),
}

impl std::fmt::Display for CertError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CertError::Generation(s) => write!(f, "cert generation error: {}", s),
            CertError::Tls(s) => write!(f, "TLS error: {}", s),
            CertError::KeyFormat(s) => write!(f, "key format error: {}", s),
            CertError::Io(e) => write!(f, "I/O error: {}", e),
        }
    }
}

impl std::error::Error for CertError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_cert_passes_verification() {
        let (signing_key, node_id, cert_der, _key_der) = generate_identity().unwrap();
        assert!(
            verify_dsearch_cert(&cert_der).is_ok(),
            "self-generated cert should verify"
        );
        let expected = node_id_from_pubkey(&signing_key.verifying_key());
        assert_eq!(node_id, expected);
    }

    #[test]
    fn tampered_cert_fails_verification() {
        let (_signing_key, _node_id, cert_der, _key_der) = generate_identity().unwrap();
        let mut tampered = cert_der.clone();
        if !tampered.is_empty() {
            let mid = tampered.len() / 2;
            tampered[mid] ^= 0xFF;
        }
        assert!(
            verify_dsearch_cert(&tampered).is_err(),
            "tampered cert should fail verification"
        );
    }

    #[test]
    fn wrong_key_cert_fails_verification() {
        let (signing_key_a, _node_id_a, _cert_der_a, _key_der_a) = generate_identity().unwrap();
        let (_signing_key_b, node_id_b, _cert_der_b, _key_der_b) = generate_identity().unwrap();

        // Create a cert with key A but SAN URI claiming node_id_b
        let (bad_cert_der, _) = make_cert_and_key(&signing_key_a, &node_id_b).unwrap();
        assert!(
            verify_dsearch_cert(&bad_cert_der).is_err(),
            "mismatched node_id should fail"
        );
    }

    #[test]
    fn extract_san_uri_from_generated_cert() {
        let (_signing_key, node_id, cert_der, _key_der) = generate_identity().unwrap();
        let cert = Certificate::from_der(&cert_der).unwrap();
        let extracted = extract_san_uri(&cert).unwrap();
        assert_eq!(extracted, node_id);
    }

    #[test]
    fn extract_ed25519_pubkey_from_generated_cert() {
        let (signing_key, _node_id, cert_der, _key_der) = generate_identity().unwrap();
        let cert = Certificate::from_der(&cert_der).unwrap();
        let extracted = extract_ed25519_pubkey(&cert).unwrap();
        assert_eq!(extracted, signing_key.verifying_key().to_bytes());
    }
}
